#![cfg(target_os = "macos")]

use peep_runtime::{Preferences, audio};
pub use peep_runtime::{data_directory, lock_instance};
mod clipboard;
mod delivery;
mod indicator;
mod interruptions;
mod languages;
mod models;
mod shortcut_conflicts;
mod worker;

use anyhow::{Context, Result, anyhow};
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{HotKey, Modifiers},
};
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSAccessibility, NSCellImagePosition, NSImage, NSPasteboard, NSPasteboardContentsOptions,
    NSPasteboardTypeString, NSWorkspace,
};
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
use objc2_foundation::{NSData, NSSize, NSString, NSURL};
use peep_core::{Command, Event, Phase};
use std::{cell::RefCell, path::PathBuf, sync::mpsc};
use tray_icon::{
    TrayIcon, TrayIconBuilder,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
};

thread_local! { static SERVICES: RefCell<Option<Services>> = const { RefCell::new(None) }; }

/// Configures utility-window restoration before AppKit creates any windows.
pub fn configure_application() {
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSDictionary, NSNumber, NSUserDefaults};
    let key = NSString::from_str("ApplePersistenceIgnoreState");
    let value = NSNumber::numberWithBool(true);
    let defaults = NSDictionary::<NSString, AnyObject>::from_slices(&[&*key], &[value.as_ref()]);
    unsafe { NSUserDefaults::standardUserDefaults().registerDefaults(&defaults) };
}

/// Shows a non-activating notification panel.
///
/// # Safety
/// Both pointers must refer to live NSViews on the main thread.
pub unsafe fn show_toast_window(
    view: std::ptr::NonNull<std::ffi::c_void>,
    settings: Option<std::ptr::NonNull<std::ffi::c_void>>,
) {
    let mtm = MainThreadMarker::new().expect("notification runs on the main thread");
    let hidden = objc2_app_kit::NSApplication::sharedApplication(mtm).isHidden();
    let view = unsafe { &*view.cast::<objc2_app_kit::NSView>().as_ptr() };
    if let Some(window) = view.window() {
        window.setHidesOnDeactivate(false);
        window.orderFrontRegardless();
    }
    if hidden && let Some(settings) = settings {
        unsafe { hide_settings_window(settings) };
    }
}

/// Hides only the settings window, without discarding its UI state.
///
/// # Safety
/// `view` must point to a live NSView on the main thread.
pub unsafe fn hide_settings_window(view: std::ptr::NonNull<std::ffi::c_void>) {
    let view = unsafe { &*view.cast::<objc2_app_kit::NSView>().as_ptr() };
    if let Some(window) = view.window() {
        window.orderOut(None);
    }
}

/// The current screen's usable height, excluding the Dock, menu bar and window chrome.
pub fn available_content_height() -> f32 {
    use objc2_app_kit::{NSApplication, NSScreen};
    let mtm = MainThreadMarker::new().expect("window layout runs on the main thread");
    let window = NSApplication::sharedApplication(mtm).mainWindow();
    let screen = window
        .as_ref()
        .and_then(|w| w.screen())
        .or_else(|| NSScreen::mainScreen(mtm));
    let chrome = window.as_ref().map_or(0., |w| {
        let frame = w.frame();
        frame.size.height - w.contentRectForFrameRect(frame).size.height
    });
    screen.map_or(f32::MAX, |s| (s.visibleFrame().size.height - chrome) as f32)
}
fn shortcut_display_name(chord: &str) -> String {
    chord
        .split('+')
        .map(|part| match part {
            "Super" | "Meta" => "Command",
            "Alt" => "Option",
            other => other
                .strip_prefix("Key")
                .or_else(|| other.strip_prefix("Digit"))
                .unwrap_or(other),
        })
        .collect::<Vec<_>>()
        .join("–")
}

struct StatusMenu {
    tray: TrayIcon,
    title: MenuItem,
    toggle: MenuItem,
    cancel: MenuItem,
    copy: MenuItem,
    discard: MenuItem,
    language: Submenu,
    languages: Vec<(String, CheckMenuItem)>,
}
impl StatusMenu {
    fn new(
        events: async_channel::Sender<Event>,
        selected_language: Option<&str>,
        mtm: MainThreadMarker,
    ) -> Result<Self> {
        let title = MenuItem::new("Model needed", false, None);
        let toggle = MenuItem::with_id("stop", "Stop Dictation", false, None);
        let cancel = MenuItem::with_id("cancel", "Cancel Dictation", false, None);
        let copy = MenuItem::with_id("copy", "Copy Transcript", false, None);
        let discard = MenuItem::with_id("discard", "Discard Transcript", false, None);
        let open = MenuItem::with_id("open", "Open Peep", true, None);
        let settings = MenuItem::with_id("open_settings", "Settings…", true, None);
        let quit = MenuItem::with_id("quit", "Quit Peep", true, None);
        let language = Submenu::new("Dictation language", true);
        let mut languages = Vec::new();
        for (code, label) in
            std::iter::once(("auto", "Automatic")).chain(languages::CHOICES.iter().copied())
        {
            let item = CheckMenuItem::with_id(
                format!("language:{code}"),
                label,
                true,
                selected_language.unwrap_or("auto") == code,
                None,
            );
            language.append(&item)?;
            languages.push((code.to_owned(), item));
        }
        let menu = Menu::with_items(&[
            &title,
            &PredefinedMenuItem::separator(),
            &toggle,
            &cancel,
            &copy,
            &discard,
            &language,
            &PredefinedMenuItem::separator(),
            &open,
            &settings,
            &quit,
        ])?;
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let _ = events.try_send(Event {
                kind: event.id.0,
                ..Default::default()
            });
        }));
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Peep")
            .build()?;
        // A template uses only the mascot's alpha silhouette; AppKit supplies
        // the correct tint for light, dark, and highlighted menu bar states.
        let image = NSImage::initWithData(
            NSImage::alloc(),
            &NSData::with_bytes(include_bytes!("../../../assets/icons/Peep-header-128.png")),
        )
        .context("load menu bar bird")?;
        image.setSize(NSSize::new(20., 20.));
        image.setTemplate(true);
        let item = tray.ns_status_item().context("create menu bar item")?;
        let button = item.button(mtm).context("create menu bar button")?;
        button.setImage(Some(&image));
        button.setImagePosition(NSCellImagePosition::ImageOnly);
        button.setAccessibilityLabel(Some(&NSString::from_str("Peep")));
        Ok(Self {
            tray,
            title,
            toggle,
            cancel,
            copy,
            discard,
            language,
            languages,
        })
    }
    fn select_language(&self, code: Option<&str>) {
        for (value, item) in &self.languages {
            item.set_checked(value == code.unwrap_or("auto"));
        }
    }
    fn update(&self, title: &str, phase: Phase, can_copy: bool) {
        self.title.set_text(title);
        self.toggle.set_enabled(phase == Phase::Recording);
        self.cancel.set_enabled(matches!(
            phase,
            Phase::Downloading
                | Phase::Recording
                | Phase::Transcribing
                | Phase::RequestingMicrophone
        ));
        self.copy.set_enabled(can_copy);
        self.discard.set_enabled(can_copy);
        self.language.set_enabled(!matches!(
            phase,
            Phase::RequestingMicrophone
                | Phase::Recording
                | Phase::Transcribing
                | Phase::Delivering
                | Phase::Canceling
        ));
        let _ = self.tray.set_tooltip(Some(format!("Peep — {title}")));
    }
}

struct Services {
    events: async_channel::Sender<Event>,
    preferences: Preferences,
    hotkeys: GlobalHotKeyManager,
    registered: Option<HotKey>,
    shortcut_error: String,
    recording_shortcut: bool,
    status: StatusMenu,
    indicator: indicator::Indicator,
    speech: mpsc::Sender<worker::Job>,
    delivery: mpsc::Sender<delivery::Job>,
    capture: Option<audio::Audio>,
    interruptions: Option<interruptions::Interruptions>,
    active_id: u64,
    control: Option<worker::Control>,
    worker_busy: bool,
    pending_microphone: bool,
    shutting_down: bool,
}
impl Services {
    fn new(
        events: async_channel::Sender<Event>,
        mtm: MainThreadMarker,
    ) -> Result<(Self, async_channel::Receiver<()>)> {
        let preferences_result = Preferences::load();
        let (preferences, preference_error) = match preferences_result {
            Ok(value) => (value, None),
            Err(error) => (
                Preferences::default(),
                Some(format!("Settings could not load: {error}")),
            ),
        };
        let (speech, speech_cleanup) = worker::spawn(events.clone());
        let (delivery, delivery_cleanup) = delivery::spawn(events.clone());
        let (cleanup_tx, cleanup) = async_channel::bounded(1);
        let shutdown_events = events.clone();
        std::thread::Builder::new()
            .name("peep-shutdown".into())
            .spawn(move || {
                let _ = speech_cleanup.recv_blocking();
                let _ = delivery_cleanup.recv_blocking();
                let _ = cleanup_tx.try_send(());
                let _ = shutdown_events.try_send(Event {
                    kind: "shutdown_complete".into(),
                    ..Default::default()
                });
            })?;
        let status = StatusMenu::new(
            events.clone(),
            preferences.dictation_language.as_deref(),
            mtm,
        )?;
        let hotkeys = GlobalHotKeyManager::new()?;
        let hotkey_events = events.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state == HotKeyState::Pressed {
                let _ = hotkey_events.try_send(Event {
                    kind: "hotkey".into(),
                    id: event.id as u64,
                    ..Default::default()
                });
            }
        }));
        let mut service = Self {
            events,
            preferences,
            hotkeys,
            registered: None,
            shortcut_error: String::new(),
            recording_shortcut: false,
            status,
            indicator: indicator::Indicator::new(mtm),
            speech,
            delivery,
            capture: None,
            interruptions: None,
            active_id: 0,
            control: None,
            worker_busy: false,
            pending_microphone: false,
            shutting_down: false,
        };
        service.register_shortcut();
        if let Some(error) = preference_error {
            eprintln!("Peep settings could not load: {error}");
        }
        Ok((service, cleanup))
    }
    fn emit(&self, kind: &str, id: u64, text: String) {
        let _ = self.events.try_send(Event {
            kind: kind.into(),
            id,
            text,
            ..Default::default()
        });
    }
    fn report_critical_problem(&self, text: &str) {
        self.emit("critical_problem", 0, text.into());
    }
    fn register_shortcut(&mut self) {
        self.shortcut_error.clear();
        if let Some(previous) = self.registered.take() {
            let _ = self.hotkeys.unregister(previous);
        }
        if !self.preferences.shortcut_enabled || self.recording_shortcut {
            return;
        }
        match parse_shortcut(&self.preferences.shortcut).and_then(|key| {
            self.hotkeys.register(key)?;
            Ok(key)
        }) {
            Ok(key) => self.registered = Some(key),
            Err(error) => self.shortcut_error = error.to_string(),
        }
    }
    fn permissions(&self) {
        let microphone = match microphone_status() {
            AVAuthorizationStatus::Authorized => "Allowed",
            AVAuthorizationStatus::NotDetermined => "Asked when you record",
            _ => "Not allowed",
        };
        let _ = self.events.try_send(Event {
            kind: "settings".into(),
            shortcut: shortcut_display_name(&self.preferences.shortcut),
            shortcut_enabled: self.preferences.shortcut_enabled,
            shortcut_available: self.registered.is_some(),
            indicator: self.preferences.indicator,
            accessibility: delivery::trusted(),
            microphone: microphone.into(),
            text: self.shortcut_error.clone(),
            ..Default::default()
        });
    }
    fn save(&self) {
        if let Err(error) = self.preferences.save() {
            eprintln!("Peep settings could not save: {error}");
        }
    }
    fn command(&mut self, command: Command, mtm: MainThreadMarker) -> Result<()> {
        if self.shutting_down && !matches!(command, Command::Shutdown) {
            return Ok(());
        }
        match command {
            Command::Initialize => {
                self.permissions();
                let args: Vec<String> = std::env::args().collect();
                let path = args
                    .iter()
                    .position(|x| x == "--parakeet-models")
                    .and_then(|i| args.get(i + 1))
                    .map(PathBuf::from)
                    .or_else(|| self.preferences.model_path.clone())
                    .or_else(|| {
                        models::model_directory()
                            .exists()
                            .then(models::model_directory)
                    });
                if let Some(path) = path {
                    let _ = self.events.try_send(Event {
                        kind: "model_found".into(),
                        path: path.to_string_lossy().into_owned(),
                        ..Default::default()
                    });
                }
            }
            Command::Prepare { id, path } => {
                let control = self.begin(id)?;
                self.worker_busy = true;
                let mut path = PathBuf::from(path);
                if path.file_name().is_some_and(|name| name == "parakeet-v3") {
                    path = path.parent().context("Model parent folder")?.to_owned();
                }
                self.speech.send(worker::Job::Prepare(id, path, control))?;
            }
            Command::Download { id } => {
                let control = self.begin(id)?;
                self.worker_busy = true;
                self.speech.send(worker::Job::Download(id, control))?;
            }
            Command::Start { id } => {
                let control = self.begin(id)?;
                let (pid, _, _) = delivery::frontmost()
                    .context("Focus a text field before pressing your dictation shortcut.")?;
                self.delivery
                    .send(delivery::Job::Capture(id, pid, control.cancel))?;
                self.pending_microphone = true;
            }
            Command::Stop { id } if id == self.active_id => {
                let language =
                    languages::selection(self.preferences.dictation_language.as_deref())?;
                self.interruptions.take();
                let (samples, rate) = self
                    .capture
                    .take()
                    .ok_or_else(|| anyhow!("No recording to stop."))?
                    .stop()?;
                self.worker_busy = true;
                self.speech.send(worker::Job::Transcribe(
                    id,
                    samples,
                    rate,
                    language,
                    self.control.clone().context("Missing session control")?,
                ))?;
            }
            Command::Deliver { id, text } if id == self.active_id => {
                self.delivery.send(delivery::Job::Deliver(id, text))?;
            }
            Command::Cancel { id } if id == self.active_id => {
                self.capture.take();
                self.interruptions.take();
                self.pending_microphone = false;
                if let Some(control) = &self.control {
                    control.cancel();
                }
                self.delivery.send(delivery::Job::Discard(id))?;
                if !self.worker_busy {
                    self.emit("canceled", id, String::new());
                }
            }
            Command::Discard { id } if id == self.active_id => {
                self.capture.take();
                self.interruptions.take();
                self.pending_microphone = false;
                self.delivery.send(delivery::Job::Discard(id))?;
            }
            Command::Copy { text } => {
                if !text.is_empty() {
                    let board = NSPasteboard::generalPasteboard();
                    board.prepareForNewContentsWithOptions(
                        NSPasteboardContentsOptions::CurrentHostOnly,
                    );
                    let copied = unsafe {
                        board.setString_forType(&NSString::from_str(&text), NSPasteboardTypeString)
                    };
                    if copied {
                        self.emit("text_copied", 0, String::new());
                    } else {
                        self.report_critical_problem("The transcript couldn’t be copied. Your dictated text is still available from the menu bar.");
                    }
                }
            }
            Command::Settings { key, value } => {
                match key.as_str() {
                    "globalShortcutEnabled" => {
                        self.preferences.shortcut_enabled = value;
                        self.register_shortcut();
                    }
                    "showRecordingIndicator" => self.preferences.indicator = value,
                    _ => {}
                }
                self.save();
                self.permissions();
            }
            Command::Accessibility => {
                if delivery::trusted() {
                    open_settings("Privacy_Accessibility");
                } else {
                    delivery::request_trust();
                    open_settings("Privacy_Accessibility");
                }
                self.permissions();
            }
            Command::MicrophoneSettings => {
                if microphone_status() == AVAuthorizationStatus::NotDetermined {
                    let events = self.events.clone();
                    let handler = block2::RcBlock::new(move |_: objc2::runtime::Bool| {
                        let _ = events.try_send(Event {
                            kind: "refresh_permissions".into(),
                            ..Default::default()
                        });
                    });
                    unsafe {
                        AVCaptureDevice::requestAccessForMediaType_completionHandler(
                            AVMediaTypeAudio.expect("audio media type"),
                            &handler,
                        );
                    }
                } else {
                    open_settings("Privacy_Microphone");
                }
            }
            Command::RecordShortcut => {
                self.recording_shortcut = true;
                self.register_shortcut();
            }
            Command::SetShortcut { chord } => {
                self.recording_shortcut = false;
                let result = (|| -> Result<()> {
                    let key = parse_shortcut(&chord)?;
                    if shortcut_conflicts::peep_conflict(key) {
                        anyhow::bail!("Used by Peep");
                    }
                    if shortcut_conflicts::system_conflict(key)? {
                        anyhow::bail!("Used by macOS");
                    }
                    if let Some(previous) = self.registered.take() {
                        self.hotkeys.unregister(previous)?;
                    }
                    self.hotkeys.register(key).map_err(|error| match error {
                        global_hotkey::Error::AlreadyRegistered(_) => anyhow!("Already in use"),
                        _ => anyhow!("Shortcut unavailable"),
                    })?;
                    if self.preferences.shortcut_enabled {
                        self.registered = Some(key);
                    } else {
                        self.hotkeys.unregister(key)?;
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    self.register_shortcut();
                    self.emit("shortcut_rejected", 0, error.to_string());
                } else {
                    self.preferences.shortcut = chord;
                    self.save();
                    self.emit("shortcut_changed", 0, String::new());
                }
                self.permissions();
            }
            Command::CancelShortcut => {
                self.recording_shortcut = false;
                self.register_shortcut();
                self.permissions();
            }
            Command::RefreshPermissions => self.permissions(),
            Command::Presentation {
                title,
                phase,
                can_copy,
            } => {
                self.status.update(&title, phase, can_copy);
                let active = matches!(
                    phase,
                    Phase::RequestingMicrophone
                        | Phase::Recording
                        | Phase::Transcribing
                        | Phase::Delivering
                );
                self.indicator.update(
                    &title,
                    phase == Phase::Recording,
                    self.preferences.indicator && active,
                    mtm,
                );
            }
            Command::Shutdown if !self.shutting_down => {
                self.shutting_down = true;
                self.capture.take();
                self.interruptions.take();
                self.pending_microphone = false;
                if let Some(control) = &self.control {
                    control.cancel();
                }
                if let Some(hotkey) = self.registered.take() {
                    let _ = self.hotkeys.unregister(hotkey);
                }
                self.indicator.update("", false, false, mtm);
                self.delivery.send(delivery::Job::Shutdown)?;
                self.speech.send(worker::Job::Shutdown)?;
            }
            _ => {}
        }
        Ok(())
    }
    fn begin(&mut self, id: u64) -> Result<worker::Control> {
        if self.worker_busy || self.capture.is_some() {
            anyhow::bail!("The previous recording is still finishing.");
        }
        self.active_id = id;
        let control = worker::Control::new();
        self.control = Some(control.clone());
        Ok(control)
    }
    fn request_microphone(&mut self) -> Result<()> {
        let id = self.active_id;
        self.pending_microphone = true;
        match microphone_status() {
            AVAuthorizationStatus::Authorized => self.start_capture()?,
            AVAuthorizationStatus::NotDetermined => {
                let events = self.events.clone();
                let handler = block2::RcBlock::new(move |allowed: objc2::runtime::Bool| {
                    let _ = events.try_send(Event {
                        kind: "microphone_result".into(),
                        id,
                        text: if allowed.as_bool() {
                            "allowed"
                        } else {
                            "denied"
                        }
                        .into(),
                        ..Default::default()
                    });
                });
                unsafe {
                    AVCaptureDevice::requestAccessForMediaType_completionHandler(
                        AVMediaTypeAudio.expect("audio media type"),
                        &handler,
                    );
                }
            }
            _ => {
                self.pending_microphone = false;
                anyhow::bail!("Allow Peep in System Settings › Privacy & Security › Microphone.");
            }
        }
        Ok(())
    }
    fn start_capture(&mut self) -> Result<()> {
        self.pending_microphone = false;
        self.interruptions = Some(interruptions::Interruptions::new(self.events.clone())?);
        self.capture = Some(audio::Audio::start(self.active_id, self.events.clone())?);
        self.emit("recording", self.active_id, String::new());
        self.permissions();
        Ok(())
    }
    fn event(&mut self, event: &Event, _mtm: MainThreadMarker) -> bool {
        match event.kind.as_str() {
            kind if kind.starts_with("language:") => {
                let code = kind
                    .strip_prefix("language:")
                    .filter(|code| *code != "auto");
                if !self.shutting_down
                    && self.capture.is_none()
                    && !self.pending_microphone
                    && !self.worker_busy
                    && languages::selection(code).is_ok()
                {
                    self.preferences.dictation_language = code.map(str::to_owned);
                    self.save();
                }
                self.status
                    .select_language(self.preferences.dictation_language.as_deref());
                true
            }
            "audio_level" => {
                if event.id == self.active_id && self.capture.is_some() {
                    self.indicator.level(event.progress);
                }
                true
            }
            "refresh_permissions" => {
                self.permissions();
                true
            }
            "hotkey" => {
                if !self.shutting_down && self.registered.map(|k| k.id as u64) == Some(event.id) {
                    self.emit("toggle_dictation", 0, String::new());
                }
                true
            }
            "target_ready" => {
                if event.id == self.active_id
                    && self.pending_microphone
                    && !self.shutting_down
                    && let Err(error) = self.request_microphone()
                {
                    self.pending_microphone = false;
                    self.emit("error", event.id, error.to_string());
                }
                true
            }
            "microphone_result" => {
                if self.active_id == event.id && self.pending_microphone && !self.shutting_down {
                    self.pending_microphone = false;
                    if event.text == "allowed" {
                        if let Err(error) = self.start_capture() {
                            self.emit("error", event.id, error.to_string());
                        }
                    } else {
                        self.emit(
                            "error",
                            event.id,
                            "Allow Peep in System Settings › Privacy & Security › Microphone."
                                .into(),
                        );
                    }
                    self.permissions();
                }
                true
            }
            "capture_interrupted" if self.capture.is_some() || self.pending_microphone => {
                self.capture.take();
                self.interruptions.take();
                self.pending_microphone = false;
                if let Some(control) = &self.control {
                    control.cancel();
                }
                self.emit("error", self.active_id, event.text.clone());
                true
            }
            "capture_error" if event.id == self.active_id && self.capture.is_some() => {
                self.capture.take();
                self.interruptions.take();
                self.emit("error", event.id, event.text.clone());
                true
            }
            "ready" if event.id == self.active_id => {
                self.worker_busy = false;
                self.preferences.model_path = Some(PathBuf::from(&event.path));
                self.save();
                false
            }
            "error" | "canceled" | "transcript" if event.id == self.active_id => {
                self.pending_microphone = false;
                self.worker_busy = false;
                false
            }
            _ => false,
        }
    }
}
fn microphone_status() -> AVAuthorizationStatus {
    unsafe {
        AVCaptureDevice::authorizationStatusForMediaType(
            AVMediaTypeAudio.expect("audio media type"),
        )
    }
}
fn open_settings(section: &str) {
    let url = NSURL::URLWithString(&NSString::from_str(&format!(
        "x-apple.systempreferences:com.apple.preference.security?{section}"
    )));
    if let Some(url) = url {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}
fn parse_shortcut(chord: &str) -> Result<HotKey> {
    let key: HotKey = chord.parse()?;
    if !key
        .mods
        .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::META)
    {
        anyhow::bail!("Use a shortcut with Control, Option, or Command.");
    }
    Ok(key)
}
pub fn initialize(events: async_channel::Sender<Event>) -> Result<async_channel::Receiver<()>> {
    let mtm =
        MainThreadMarker::new().context("AppKit services must initialize on the main thread")?;
    SERVICES.with(|cell| {
        let (service, cleanup) = Services::new(events, mtm)?;
        *cell.borrow_mut() = Some(service);
        Ok(cleanup)
    })
}
pub fn send(command: Command) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    SERVICES.with(|cell| {
        if let Some(service) = cell.borrow_mut().as_mut()
            && let Err(error) = service.command(command, mtm)
        {
            service.emit("error", service.active_id, error.to_string());
        }
    });
}
pub fn handle_event(event: &Event) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    SERVICES.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .is_some_and(|service| service.event(event, mtm))
    })
}
pub fn model_download_size() -> u64 {
    models::download_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcuts_require_a_command_modifier() {
        assert!(parse_shortcut("Control+Shift+Space").is_ok());
        assert!(parse_shortcut("Super+KeyK").is_ok());
        assert!(parse_shortcut("Shift+Space").is_err());
        assert!(parse_shortcut("bogus").is_err());
    }

    #[test]
    fn shortcut_display_uses_mac_key_names() {
        assert_eq!(shortcut_display_name("Shift+Super+KeyL"), "Shift–Command–L");
        assert_eq!(
            shortcut_display_name("Control+Alt+Digit2"),
            "Control–Option–2"
        );
    }
}
