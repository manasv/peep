pub mod worker;

use anyhow::{Result, anyhow, bail};
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{HotKey, Modifiers},
};
use peep_core::{Command, Event};
use peep_runtime::{Preferences, audio::Audio, models::ONNX};
use std::{cell::RefCell, path::PathBuf, sync::mpsc};
use tokio_util::sync::CancellationToken;

pub use peep_runtime::lock_instance;

thread_local! { static SERVICES: RefCell<Option<Services>> = const { RefCell::new(None) }; }

struct Services {
    events: async_channel::Sender<Event>,
    preferences: Preferences,
    hotkeys: Option<GlobalHotKeyManager>,
    registered: Option<HotKey>,
    shortcut_error: String,
    recording_shortcut: bool,
    speech: mpsc::Sender<worker::Job>,
    capture: Option<Audio>,
    cancel: CancellationToken,
    active_id: u64,
    worker_busy: bool,
    shutting_down: bool,
}

impl Services {
    fn new(events: async_channel::Sender<Event>) -> Result<(Self, async_channel::Receiver<()>)> {
        let preferences = Preferences::load().unwrap_or_else(|error| {
            eprintln!("Peep settings could not load: {error}");
            Preferences::default()
        });
        let (speech, cleanup) = worker::spawn(events.clone());
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
        let hotkeys = if global_shortcuts_supported() {
            GlobalHotKeyManager::new().ok()
        } else {
            None
        };
        let mut service = Self {
            events,
            preferences,
            hotkeys,
            registered: None,
            shortcut_error: String::new(),
            recording_shortcut: false,
            speech,
            capture: None,
            cancel: CancellationToken::new(),
            active_id: 0,
            worker_busy: false,
            shutting_down: false,
        };
        service.register_shortcut();
        Ok((service, cleanup))
    }

    fn emit(&self, kind: &str, id: u64, text: impl Into<String>) {
        let _ = self.events.try_send(Event {
            kind: kind.into(),
            id,
            text: text.into(),
            ..Default::default()
        });
    }

    fn register_shortcut(&mut self) {
        self.shortcut_error.clear();
        let Some(manager) = &self.hotkeys else {
            self.shortcut_error = "Global shortcuts unavailable; use Record below".into();
            return;
        };
        if let Some(previous) = self.registered.take() {
            let _ = manager.unregister(previous);
        }
        if !self.preferences.shortcut_enabled || self.recording_shortcut {
            return;
        }
        match parse_shortcut(&self.preferences.shortcut).and_then(|key| {
            manager.register(key)?;
            Ok(key)
        }) {
            Ok(key) => self.registered = Some(key),
            Err(error) => self.shortcut_error = error.to_string(),
        }
    }

    fn permissions(&self) {
        let _ = self.events.try_send(Event {
            kind: "settings".into(),
            shortcut: shortcut_display_name(&self.preferences.shortcut),
            shortcut_enabled: self.preferences.shortcut_enabled,
            shortcut_available: self.registered.is_some(),
            text: self.shortcut_error.clone(),
            ..Default::default()
        });
    }

    fn save(&self) {
        if let Err(error) = self.preferences.save() {
            eprintln!("Peep settings could not save: {error}");
        }
    }

    fn begin(&mut self, id: u64) -> Result<CancellationToken> {
        if self.worker_busy || self.capture.is_some() {
            bail!("The previous recording is still finishing.");
        }
        self.active_id = id;
        self.cancel = CancellationToken::new();
        Ok(self.cancel.clone())
    }

    fn command(&mut self, command: Command) -> Result<()> {
        if self.shutting_down {
            return Ok(());
        }
        match command {
            Command::Initialize => {
                self.permissions();
                let args: Vec<String> = std::env::args().collect();
                let path = args
                    .iter()
                    .position(|arg| arg == "--models")
                    .and_then(|i| args.get(i + 1))
                    .map(PathBuf::from)
                    .or_else(|| self.preferences.model_path.clone())
                    .or_else(|| ONNX.directory().exists().then(|| ONNX.directory()));
                if let Some(path) = path {
                    let _ = self.events.try_send(Event {
                        kind: "model_found".into(),
                        path: path.to_string_lossy().into_owned(),
                        ..Default::default()
                    });
                }
            }
            Command::Prepare { id, path } => {
                let cancel = self.begin(id)?;
                self.speech
                    .send(worker::Job::Prepare(id, path.into(), cancel))?;
                self.worker_busy = true;
            }
            Command::Download { id } => {
                let cancel = self.begin(id)?;
                self.speech.send(worker::Job::Download(id, cancel))?;
                self.worker_busy = true;
            }
            Command::Start { id } => {
                self.begin(id)?;
                self.capture = Some(Audio::start(id, self.events.clone())?);
                self.emit("recording", id, "");
            }
            Command::Stop { id } if id == self.active_id => {
                let (samples, rate) = self
                    .capture
                    .take()
                    .ok_or_else(|| anyhow!("No recording to stop."))?
                    .stop()?;
                self.speech.send(worker::Job::Transcribe(
                    id,
                    samples,
                    rate,
                    self.cancel.clone(),
                ))?;
                self.worker_busy = true;
            }
            Command::Deliver { id, .. } if id == self.active_id => self.emit("copy_ready", id, ""),
            Command::Cancel { id } if id == self.active_id => {
                self.capture.take();
                self.cancel.cancel();
                if !self.worker_busy {
                    self.emit("canceled", id, "");
                }
            }
            Command::Discard { id } if id == self.active_id => {
                self.capture.take();
            }
            Command::Copy { text } => {
                if !text.is_empty() {
                    self.emit("copy_requested", 0, text);
                }
            }
            Command::Settings { key, value } => {
                if key == "globalShortcutEnabled" {
                    self.preferences.shortcut_enabled = value;
                    self.register_shortcut();
                    self.save();
                    self.permissions();
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
                    if key.mods == Modifiers::CONTROL
                        && matches!(
                            key.key,
                            global_hotkey::hotkey::Code::KeyQ | global_hotkey::hotkey::Code::Comma
                        )
                    {
                        bail!("Used by Peep");
                    }
                    let manager = self.hotkeys.as_ref().ok_or_else(|| {
                        anyhow!("Global shortcuts are unavailable in this session.")
                    })?;
                    if let Some(previous) = self.registered.take() {
                        manager.unregister(previous)?;
                    }
                    manager.register(key)?;
                    if self.preferences.shortcut_enabled {
                        self.registered = Some(key);
                    } else {
                        manager.unregister(key)?;
                    }
                    Ok(())
                })();
                match result {
                    Ok(()) => {
                        self.preferences.shortcut = chord;
                        self.save();
                        self.emit("shortcut_changed", 0, "");
                    }
                    Err(error) => {
                        self.register_shortcut();
                        self.emit("shortcut_rejected", 0, error.to_string());
                    }
                }
                self.permissions();
            }
            Command::CancelShortcut => {
                self.recording_shortcut = false;
                self.register_shortcut();
                self.permissions();
            }
            Command::RefreshPermissions => self.permissions(),
            Command::Shutdown => {
                self.shutting_down = true;
                self.capture.take();
                self.cancel.cancel();
                if let Some(key) = self.registered.take()
                    && let Some(manager) = &self.hotkeys
                {
                    let _ = manager.unregister(key);
                }
                self.speech.send(worker::Job::Shutdown)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn event(&mut self, event: &Event) -> bool {
        match event.kind.as_str() {
            "hotkey" => {
                if !self.shutting_down && self.registered.map(|key| key.id as u64) == Some(event.id)
                {
                    self.emit("toggle_dictation", 0, "");
                }
                true
            }
            "audio_level" => true,
            "capture_error" if event.id == self.active_id && self.capture.is_some() => {
                self.capture.take();
                self.cancel.cancel();
                self.emit("error", event.id, event.text.clone());
                true
            }
            "ready" if event.id == self.active_id => {
                self.worker_busy = false;
                self.preferences.model_path = Some(event.path.clone().into());
                self.save();
                false
            }
            "transcript" | "error" | "canceled" if event.id == self.active_id => {
                self.worker_busy = false;
                false
            }
            _ => false,
        }
    }
}

fn global_shortcuts_supported() -> bool {
    if cfg!(target_os = "linux") {
        std::env::var_os("WAYLAND_DISPLAY").is_none_or(|value| value.is_empty())
            && std::env::var("XDG_SESSION_TYPE").as_deref() != Ok("wayland")
            && std::env::var_os("DISPLAY").is_some_and(|value| !value.is_empty())
    } else {
        true
    }
}

fn parse_shortcut(chord: &str) -> Result<HotKey> {
    let key: HotKey = chord.parse()?;
    if !key
        .mods
        .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::META)
    {
        bail!("Use a shortcut with Control, Alt, or Super.");
    }
    Ok(key)
}

fn shortcut_display_name(chord: &str) -> String {
    chord
        .split('+')
        .map(|part| match part {
            "Super" | "Meta" if cfg!(target_os = "windows") => "Windows",
            "Alt" => "Alt",
            other => other
                .strip_prefix("Key")
                .or_else(|| other.strip_prefix("Digit"))
                .unwrap_or(other),
        })
        .collect::<Vec<_>>()
        .join("–")
}

pub fn initialize(events: async_channel::Sender<Event>) -> Result<async_channel::Receiver<()>> {
    let (service, worker_cleanup) = Services::new(events.clone())?;
    let (cleanup_tx, cleanup) = async_channel::bounded(1);
    std::thread::Builder::new()
        .name("peep-shutdown".into())
        .spawn(move || {
            let _ = worker_cleanup.recv_blocking();
            let _ = cleanup_tx.try_send(());
            let _ = events.try_send(Event {
                kind: "shutdown_complete".into(),
                ..Default::default()
            });
        })?;
    SERVICES.with(|cell| *cell.borrow_mut() = Some(service));
    Ok(cleanup)
}

pub fn send(command: Command) {
    SERVICES.with(|cell| {
        if let Some(service) = cell.borrow_mut().as_mut() {
            let copying = matches!(command, Command::Copy { .. });
            if let Err(error) = service.command(command) {
                service.emit(
                    if copying { "critical_problem" } else { "error" },
                    service.active_id,
                    error.to_string(),
                );
            }
        }
    });
}

pub fn handle_event(event: &Event) -> bool {
    SERVICES.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .is_some_and(|service| service.event(event))
    })
}

pub fn model_download_size() -> u64 {
    ONNX.download_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcuts_and_labels_use_desktop_key_names() {
        assert!(parse_shortcut("Control+Shift+Space").is_ok());
        assert!(parse_shortcut("Shift+Space").is_err());
        assert_eq!(shortcut_display_name("Control+Alt+Digit2"), "Control–Alt–2");
    }
}
