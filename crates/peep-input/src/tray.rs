use anyhow::Result;
use peep_core::{Event, Phase};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

pub struct Tray {
    tray: TrayIcon,
    status: MenuItem,
    finish: MenuItem,
    cancel: MenuItem,
    copy: MenuItem,
    discard: MenuItem,
    idle_icon: Icon,
    recording_icon: Icon,
}
impl Tray {
    pub fn new(events: async_channel::Sender<Event>) -> Result<Self> {
        let image =
            image::load_from_memory(include_bytes!("../../../assets/icons/Peep-header-128.png"))?
                .into_rgba8();
        let (width, height) = image.dimensions();
        let mut recording = image.clone();
        for pixel in recording.pixels_mut() {
            if pixel[3] != 0 {
                pixel[0] = 220;
                pixel[1] = 45;
                pixel[2] = 45;
            }
        }
        let idle_icon = Icon::from_rgba(image.into_raw(), width, height)?;
        let recording_icon = Icon::from_rgba(recording.into_raw(), width, height)?;
        let status = MenuItem::new("Model needed", false, None);
        let finish = MenuItem::with_id("stop", "Finish Dictation", false, None);
        let cancel = MenuItem::with_id("cancel", "Cancel Dictation", false, None);
        let copy = MenuItem::with_id("copy", "Copy Transcript", false, None);
        let discard = MenuItem::with_id("discard", "Discard Transcript", false, None);
        let open = MenuItem::with_id("open", "Open Peep", true, None);
        let quit = MenuItem::with_id("quit", "Quit Peep", true, None);
        let menu = Menu::with_items(&[
            &status,
            &PredefinedMenuItem::separator(),
            &finish,
            &cancel,
            &copy,
            &discard,
            &PredefinedMenuItem::separator(),
            &open,
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
            .with_icon(idle_icon.clone())
            .with_tooltip("Peep")
            .build()?;
        Ok(Self {
            tray,
            status,
            finish,
            cancel,
            copy,
            discard,
            idle_icon,
            recording_icon,
        })
    }
    pub fn update(&self, title: &str, phase: Phase, can_copy: bool) {
        self.status.set_text(title);
        self.finish.set_enabled(phase == Phase::Recording);
        self.cancel.set_enabled(matches!(
            phase,
            Phase::Downloading
                | Phase::RequestingMicrophone
                | Phase::Recording
                | Phase::Transcribing
        ));
        self.copy.set_enabled(can_copy);
        self.discard.set_enabled(can_copy);
        let _ = self.tray.set_icon(Some(if phase == Phase::Recording {
            self.recording_icon.clone()
        } else {
            self.idle_icon.clone()
        }));
        let _ = self.tray.set_tooltip(Some(format!("Peep — {title}")));
    }
}
