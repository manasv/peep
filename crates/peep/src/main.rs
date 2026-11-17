use gpui::{prelude::*, *};
use peep_core::{Command, Event, Phase, Session};
use peep_macos::send;

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
    alert_open: bool,
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
        window.on_window_should_close(cx, move |_, cx| {
            let _ = weak.update(cx, |this, _cx| {
                if this.recording_shortcut {
                    this.recording_shortcut = false;
                    send(Command::CancelShortcut);
                }
            });
            cx.hide();
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
            alert_open: false,
        }
    }
    fn effects(&mut self, effects: Vec<Command>, cx: &mut Context<Self>) {
        for effect in effects {
            send(effect);
        }
        send(self.session.presentation());
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
    fn critical_alert(
        &mut self,
        title: &str,
        detail: &str,
        recover_text: bool,
        cx: &mut Context<Self>,
    ) {
        if self.alert_open || self.quitting {
            return;
        }
        self.show(cx);
        let buttons = if recover_text {
            vec![
                PromptButton::ok("Copy text"),
                PromptButton::cancel("Keep for later"),
                PromptButton::new("Discard"),
            ]
        } else {
            vec![PromptButton::ok("OK")]
        };
        let Ok(answer) = cx.update_window(self.window, |_, window, cx| {
            window.prompt(PromptLevel::Critical, title, Some(detail), &buttons, cx)
        }) else {
            return;
        };
        self.alert_open = true;
        cx.spawn(async move |this, cx| {
            let answer = answer.await.ok();
            let _ = this.update(cx, |this, cx| {
                this.alert_open = false;
                if recover_text {
                    match answer {
                        Some(0) => send(Command::Copy {
                            text: this.session.transcript.clone(),
                        }),
                        Some(2) => this.session.transcript.clear(),
                        _ => {}
                    }
                }
                this.effects(vec![], cx);
            });
        })
        .detach();
    }

    fn event(&mut self, event: Event, cx: &mut Context<Self>) {
        if peep_macos::handle_event(&event) {
            return;
        }
        match event.kind.as_str() {
            "settings" => {
                if event.shortcut_enabled
                    && !event.shortcut_available
                    && !self.recording_shortcut
                    && !event.text.is_empty()
                {
                    self.shortcut_problem = "Shortcut unavailable".into();
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
            "critical_problem" => {
                self.critical_alert("Peep needs your attention", &event.text, false, cx);
            }
            "text_copied" => {
                self.session.transcript.clear();
                self.effects(vec![], cx);
            }
            "toggle_dictation" if !self.quitting => {
                if self.alert_open {
                    return;
                }
                if !self.session.busy() && !self.session.transcript.is_empty() {
                    self.critical_alert(
                        "Your text hasn’t been inserted",
                        "Copy your dictated text before starting another recording.",
                        true,
                        cx,
                    );
                    return;
                }
                if self.session.phase == Phase::NeedsModel {
                    self.show(cx);
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
                if let Some(title) = critical_failure(before, self.session.phase, &event) {
                    let detail = if before == Phase::Delivering {
                        "Your dictated text is safe. Copy it to finish manually, or keep it for later."
                    } else {
                        "Offline dictation couldn’t be prepared. Please retry setup or reopen Peep."
                    };
                    self.critical_alert(title, detail, before == Phase::Delivering, cx);
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
                self.recording_shortcut = true;
                vec![Command::RecordShortcut]
            }
            UiAction::Refresh => vec![Command::RefreshPermissions],
        };
        self.effects(effects, cx);
    }
}

mod views;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let instance = peep_macos::lock_instance().expect("single-instance lock");
    if instance.is_none() {
        return;
    }
    let (tx, rx) = async_channel::unbounded();
    let root = Application::new();
    root.on_reopen(|cx| {
        cx.activate(true);
        for handle in cx.windows() {
            let _ = cx.update_window(handle, |_, window, _| window.activate_window());
        }
    });
    root.run(move |cx: &mut App| {
        let cleanup = peep_macos::initialize(tx).expect("initialize macOS services");
        cx.on_app_quit(move |_| {
            let cleanup = cleanup.clone();
            send(Command::Shutdown);
            async move {
                let _ = cleanup.recv().await;
            }
        })
        .detach();
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-,", ShowSettings, None),
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

fn critical_failure(before: Phase, after: Phase, event: &Event) -> Option<&'static str> {
    if event.kind != "error" || before == after {
        return None;
    }
    match before {
        Phase::Downloading | Phase::Preparing => Some("Peep couldn’t get ready"),
        Phase::Delivering => Some("Your text couldn’t be inserted"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::critical_failure;
    use peep_core::{Event, Phase};
    #[test]
    fn only_blocking_or_text_recovery_failures_raise_alerts() {
        let error = Event {
            kind: "error".into(),
            ..Default::default()
        };
        assert!(critical_failure(Phase::Delivering, Phase::Ready, &error).is_some());
        assert!(critical_failure(Phase::Downloading, Phase::NeedsModel, &error).is_some());
        assert!(critical_failure(Phase::Preparing, Phase::NeedsModel, &error).is_some());
        assert!(critical_failure(Phase::Delivering, Phase::Delivering, &error).is_none());
        assert!(critical_failure(Phase::RequestingMicrophone, Phase::Ready, &error).is_none());
        assert!(critical_failure(Phase::Recording, Phase::Ready, &error).is_none());
        assert!(
            critical_failure(
                Phase::Transcribing,
                Phase::Ready,
                &Event {
                    kind: "transcript".into(),
                    ..Default::default()
                }
            )
            .is_none()
        );
    }
}
