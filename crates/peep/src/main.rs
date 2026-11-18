#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use gpui::{prelude::*, *};
use peep_core::{Command, Event, Phase, Session};
mod platform;
use platform::send;

#[cfg(target_os = "macos")]
fn hide_main_window(window: &mut Window, cx: &mut App) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    if let Ok(handle) = HasWindowHandle::window_handle(window)
        && let RawWindowHandle::AppKit(handle) = handle.as_raw()
    {
        unsafe { peep_macos::hide_settings_window(handle.ns_view) };
    }
    cx.hide();
}

actions!(
    peep,
    [
        Quit,
        CancelRecording,
        ShowSettings,
        NextFocus,
        PreviousFocus
    ]
);

#[derive(Clone, Copy)]
enum UiAction {
    Stop,
    Cancel,
    Download,
    Settings,
    ShortcutEnabled,
    Indicator,
    Accessibility,
    Microphone,
    RecordShortcut,
    Refresh,
    #[cfg(any(not(target_os = "macos"), feature = "desktop-preview"))]
    Record,
    #[cfg(any(not(target_os = "macos"), feature = "desktop-preview"))]
    Copy,
    #[cfg(any(not(target_os = "macos"), feature = "desktop-preview"))]
    Discard,
    #[cfg(any(not(target_os = "macos"), feature = "desktop-preview"))]
    Quit,
}

struct Peep {
    session: Session,
    logo: std::sync::Arc<Image>,
    preferences: Event,
    focus: FocusHandle,
    buttons: Vec<FocusHandle>,
    window: AnyWindowHandle,
    quitting: bool,
    recording_shortcut: bool,
    shortcut_problem: String,
}
impl Peep {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        events: async_channel::Receiver<Event>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let handle = window.window_handle();
        let recorder = cx.weak_entity();
        cx.intercept_keystrokes(move |event, _, cx| {
            let _ = recorder.update(cx, |this: &mut Self, cx| {
                if this.recording_shortcut {
                    cx.stop_propagation();
                    if event.keystroke.key == "escape" {
                        this.recording_shortcut = false;
                        send(Command::CancelShortcut);
                    } else if let Some(chord) = shortcut_chord(&event.keystroke) {
                        this.recording_shortcut = false;
                        send(Command::SetShortcut { chord });
                    }
                    cx.notify();
                }
            });
        })
        .detach();
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let _ = weak.update(cx, |this, _cx| {
                if this.recording_shortcut {
                    this.recording_shortcut = false;
                    send(Command::CancelShortcut);
                }
                if !platform::MACOS {
                    window.minimize_window();
                }
            });
            if platform::MACOS {
                #[cfg(target_os = "macos")]
                window.defer(cx, hide_main_window);
            }
            false
        });
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                send(Command::RefreshPermissions);
            } else if this.recording_shortcut {
                this.recording_shortcut = false;
                send(Command::CancelShortcut);
                cx.notify();
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this.update(cx, |this, cx| this.event(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        send(Command::Initialize);
        Self {
            session: Session::default(),
            logo: std::sync::Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../../../assets/icons/Peep-header-128.png").to_vec(),
            )),
            preferences: Event::default(),
            focus,
            buttons: (0..20).map(|_| cx.focus_handle().tab_stop(true)).collect(),
            window: handle,
            quitting: false,
            recording_shortcut: false,
            shortcut_problem: String::new(),
        }
    }
    fn effects(&mut self, effects: Vec<Command>, cx: &mut Context<Self>) {
        for effect in effects {
            send(effect);
        }
        send(self.session.presentation());
        if !platform::MACOS {
            let title = format!("Peep — {}", self.session.phase.title());
            let handle = self.window;
            cx.defer(move |cx| {
                let _ = cx.update_window(handle, |_, window, _| window.set_window_title(&title));
            });
        }
        cx.notify();
    }
    fn show(&self, cx: &mut Context<Self>) {
        cx.activate(true);
        let _ = cx.update_window(self.window, |_, window, _| window.activate_window());
    }
    fn quit(&mut self) {
        if !self.quitting {
            self.quitting = true;
            send(Command::Shutdown);
        }
    }
    fn feedback(&self, message: &str, cx: &mut Context<Self>) {
        if self.quitting || self.session.busy() {
            return;
        }
        send(Command::Feedback {
            text: message.into(),
        });
        cx.spawn(async |_, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(2500))
                .await;
            send(Command::DismissFeedback);
        })
        .detach();
    }

    fn event(&mut self, event: Event, cx: &mut Context<Self>) {
        if platform::handle_event(&event) {
            return;
        }
        match event.kind.as_str() {
            "settings" => {
                if event.shortcut_available {
                    self.shortcut_problem.clear();
                }
                if event.shortcut_enabled
                    && !event.shortcut_available
                    && !self.recording_shortcut
                    && !event.text.is_empty()
                {
                    self.shortcut_problem = event.text.clone();
                }
                self.preferences = event;
                cx.notify();
            }
            "shortcut_rejected" => {
                self.shortcut_problem = event.text;
                cx.notify();
            }
            "shortcut_changed" => {
                self.shortcut_problem.clear();
                cx.notify();
            }
            "notice" => {
                eprintln!("Peep: {}", event.text);
                self.feedback("Check permissions", cx);
            }
            "critical_problem" => {
                eprintln!("Peep: {}", event.text);
                self.feedback("Couldn’t copy text", cx);
            }
            "copy_requested" if event.text == self.session.transcript && !self.session.busy() => {
                cx.write_to_clipboard(ClipboardItem::new_string(event.text.clone()));
                if cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref()
                    == Some(&event.text)
                {
                    self.session.transcript.clear();
                    self.session.message = "Copied. Paste into your app with Ctrl–V.".into();
                    self.feedback("Text copied", cx);
                } else {
                    self.session.message = "Could not copy your text. Try Copy text again.".into();
                    self.feedback("Couldn’t copy text", cx);
                }
                self.effects(vec![], cx);
            }
            "text_copied" => {
                self.session.transcript.clear();
                self.session.message = if platform::MACOS {
                    String::new()
                } else {
                    "Copied. Paste into your app with Ctrl–V.".into()
                };
                self.feedback("Text copied", cx);
                self.effects(vec![], cx);
            }
            "toggle_dictation" if !self.quitting => {
                if !self.session.busy()
                    && !self.session.transcript.is_empty()
                    && !self.session.paste_unconfirmed
                {
                    self.feedback("Copy or discard text first", cx);
                    return;
                }
                if self.session.phase == Phase::NeedsModel {
                    self.feedback("Download a model first", cx);
                    return;
                }
                let effects = self.session.toggle();
                self.effects(effects, cx);
            }
            "stop" => {
                let effects = self.session.stop();
                self.effects(effects, cx);
            }
            "cancel" => {
                let effects = self.session.cancel();
                self.effects(effects, cx);
            }
            "copy" => send(Command::Copy {
                text: self.session.transcript.clone(),
            }),
            "discard" if !self.session.busy() => {
                self.session.transcript.clear();
                self.effects(vec![], cx);
            }
            "open" => self.show(cx),
            "open_settings" => {
                self.show(cx);
                cx.notify();
            }
            "quit" => self.quit(),
            "shutdown_complete" => cx.quit(),
            _ if !self.quitting => {
                let before = self.session.phase;
                let effects = self.session.event(&event);
                if let Some(message) = feedback_message(before, self.session.phase, &event) {
                    if event.kind == "error" {
                        eprintln!("Peep: {}", event.text);
                    }
                    self.feedback(message, cx);
                }
                self.effects(effects, cx);
            }
            _ => {}
        }
    }
    fn action(&mut self, action: UiAction, cx: &mut Context<Self>) {
        if self.quitting {
            return;
        }
        let effects = match action {
            UiAction::Stop => self.session.stop(),
            UiAction::Cancel => self.session.cancel(),
            UiAction::Download => self.session.download(),
            UiAction::Settings => vec![Command::RefreshPermissions],
            UiAction::ShortcutEnabled => vec![Command::Settings {
                key: "globalShortcutEnabled".into(),
                value: !self.preferences.shortcut_enabled,
            }],
            UiAction::Indicator => vec![Command::Settings {
                key: "showRecordingIndicator".into(),
                value: !self.preferences.indicator,
            }],
            UiAction::Accessibility => vec![Command::Accessibility],
            UiAction::Microphone => vec![Command::MicrophoneSettings],
            UiAction::RecordShortcut => {
                self.shortcut_problem.clear();
                self.recording_shortcut = !self.preferences.shortcut_portal;
                vec![Command::RecordShortcut]
            }
            UiAction::Refresh => vec![Command::RefreshPermissions],
            #[cfg(any(not(target_os = "macos"), feature = "desktop-preview"))]
            UiAction::Record => {
                if !self.session.transcript.is_empty() {
                    return;
                }
                self.session
                    .toggle()
                    .into_iter()
                    .map(|command| match command {
                        Command::Start { id } => Command::StartCopy { id },
                        command => command,
                    })
                    .collect()
            }
            #[cfg(any(not(target_os = "macos"), feature = "desktop-preview"))]
            UiAction::Copy => vec![Command::Copy {
                text: self.session.transcript.clone(),
            }],
            #[cfg(any(not(target_os = "macos"), feature = "desktop-preview"))]
            UiAction::Quit => {
                self.quit();
                return;
            }
            #[cfg(any(not(target_os = "macos"), feature = "desktop-preview"))]
            UiAction::Discard => {
                self.session.transcript.clear();
                self.session.message.clear();
                vec![]
            }
        };
        self.effects(effects, cx);
    }
}

mod views;

fn main() {
    #[cfg(target_os = "macos")]
    peep_macos::configure_application();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    #[cfg(target_os = "linux")]
    match platform::handle_cli() {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => {
            eprintln!("Peep: {error}. Start Peep before using desktop control commands.");
            std::process::exit(1);
        }
    }
    let instance = platform::lock_instance().expect("single-instance lock");
    if instance.is_none() {
        #[cfg(target_os = "linux")]
        platform::open_existing();
        return;
    }
    let (tx, rx) = async_channel::unbounded();
    let root = Application::new();
    root.on_reopen(|cx| {
        cx.activate(true);
        for handle in cx.windows() {
            if let Some(handle) = handle.downcast::<Peep>() {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            }
        }
    });
    root.run(move |cx: &mut App| {
        let cleanup = platform::initialize(tx).expect("initialize platform services");
        cx.on_app_quit(move |_| {
            let cleanup = cleanup.clone();
            send(Command::Shutdown);
            async move {
                let _ = cleanup.recv().await;
            }
        })
        .detach();
        cx.bind_keys([
            KeyBinding::new(platform::QUIT_KEY, Quit, None),
            KeyBinding::new(platform::SETTINGS_KEY, ShowSettings, None),
            KeyBinding::new("escape", CancelRecording, None),
            KeyBinding::new("tab", NextFocus, None),
            KeyBinding::new("shift-tab", PreviousFocus, None),
        ]);
        cx.set_menus(vec![Menu {
            name: "Peep".into(),
            items: vec![
                MenuItem::action("Settings…", ShowSettings),
                MenuItem::separator(),
                MenuItem::action("Quit Peep", Quit),
            ],
        }]);
        let bounds = Bounds::centered(None, size(px(480.), px(480.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    app_id: Some("dev.peep.app".into()),
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(440.), px(0.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Peep".into()),
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Peep::new(window, cx, rx)),
            )
            .expect("open Peep window");
        cx.on_action(move |_: &Quit, cx| {
            let _ = window.update(cx, |this, _, _| this.quit());
        });
        cx.on_action(move |_: &ShowSettings, cx| {
            let _ = window.update(cx, |this, _, cx| {
                this.show(cx);
                cx.notify();
            });
        });
        cx.activate(true);
        #[cfg(debug_assertions)]
        if let Some(message) = std::env::var_os("PEEP_TEST_INDICATOR") {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                #[cfg(target_os = "macos")]
                let _ = window.update(cx, |_, window, cx| hide_main_window(window, cx));
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                let _ = window.update(cx, |this, _, cx| {
                    this.feedback(&message.to_string_lossy(), cx)
                });
            })
            .detach();
        }
    });
}

fn shortcut_chord(stroke: &Keystroke) -> Option<String> {
    let modifiers = stroke.modifiers;
    if !(modifiers.control || modifiers.alt || modifiers.platform) {
        return None;
    }
    let mut parts = Vec::new();
    if modifiers.control {
        parts.push("Control".to_owned());
    }
    if modifiers.alt {
        parts.push("Alt".to_owned());
    }
    if modifiers.shift {
        parts.push("Shift".to_owned());
    }
    if modifiers.platform {
        parts.push("Super".to_owned());
    }
    let key = match stroke.key.as_str() {
        "space" => "Space".into(),
        "enter" => "Enter".into(),
        "tab" => "Tab".into(),
        value if value.len() == 1 && value.as_bytes()[0].is_ascii_alphabetic() => {
            format!("Key{}", value.to_ascii_uppercase())
        }
        value if value.len() == 1 && value.as_bytes()[0].is_ascii_digit() => {
            format!("Digit{value}")
        }
        _ => return None,
    };
    parts.push(key);
    Some(parts.join("+"))
}

fn feedback_message(before: Phase, after: Phase, event: &Event) -> Option<&'static str> {
    if before == after {
        return None;
    }
    match event.kind.as_str() {
        "delivered" => Some("Text inserted"),
        "paste_sent" => Some("Paste sent"),
        "copy_ready" => Some("Text ready to copy"),
        "transcript" if event.text.trim().is_empty() => Some("No speech detected"),
        "canceled" => Some("Canceled"),
        "error" => Some(match before {
            Phase::Delivering => "Couldn’t insert text",
            Phase::RequestingMicrophone => "Couldn’t start recording",
            Phase::Recording => "Recording interrupted",
            Phase::Transcribing => "Couldn’t transcribe",
            Phase::Downloading | Phase::Preparing => "Couldn’t load model",
            _ => "Something went wrong",
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::feedback_message;
    use peep_core::{Event, Phase};

    #[test]
    fn feedback_covers_success_and_failure_without_replaying_stale_results() {
        for (kind, before, message) in [
            ("delivered", Phase::Delivering, "Text inserted"),
            ("paste_sent", Phase::Delivering, "Paste sent"),
            ("copy_ready", Phase::Delivering, "Text ready to copy"),
            ("transcript", Phase::Transcribing, "No speech detected"),
            ("error", Phase::Delivering, "Couldn’t insert text"),
            (
                "error",
                Phase::RequestingMicrophone,
                "Couldn’t start recording",
            ),
        ] {
            let event = Event {
                kind: kind.into(),
                ..Default::default()
            };
            assert_eq!(
                feedback_message(before, Phase::Ready, &event),
                Some(message)
            );
            assert_eq!(feedback_message(Phase::Ready, Phase::Ready, &event), None);
        }
        let transcript = Event {
            kind: "transcript".into(),
            text: "hello".into(),
            ..Default::default()
        };
        assert_eq!(
            feedback_message(Phase::Transcribing, Phase::Delivering, &transcript),
            None
        );
    }
}
