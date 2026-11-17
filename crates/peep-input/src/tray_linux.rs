use anyhow::Result;
use ksni::{blocking::TrayMethods, menu::StandardItem};
use peep_core::{Event, Phase};

pub struct Tray(ksni::blocking::Handle<State>);
struct State {
    events: async_channel::Sender<Event>,
    title: String,
    phase: Phase,
    can_copy: bool,
    idle: ksni::Icon,
    recording: ksni::Icon,
}
impl Tray {
    pub fn new(events: async_channel::Sender<Event>) -> Result<Self> {
        let image =
            image::load_from_memory(include_bytes!("../../../assets/icons/Peep-header-128.png"))?
                .into_rgba8();
        let (width, height) = image.dimensions();
        let mut idle = Vec::new();
        let mut recording = Vec::new();
        for pixel in image.pixels() {
            idle.extend([pixel[3], pixel[0], pixel[1], pixel[2]]);
            recording.extend([pixel[3], 220, 45, 45]);
        }
        Ok(Self(
            State {
                events,
                title: "Model needed".into(),
                phase: Phase::NeedsModel,
                can_copy: false,
                idle: ksni::Icon {
                    width: width as i32,
                    height: height as i32,
                    data: idle,
                },
                recording: ksni::Icon {
                    width: width as i32,
                    height: height as i32,
                    data: recording,
                },
            }
            .spawn()?,
        ))
    }
    pub fn update(&self, title: &str, phase: Phase, can_copy: bool) {
        self.0.update(|state| {
            state.title = title.into();
            state.phase = phase;
            state.can_copy = can_copy;
        });
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        self.0.shutdown().wait();
    }
}
impl State {
    fn emit(&self, kind: &str) {
        let _ = self.events.try_send(Event {
            kind: kind.into(),
            ..Default::default()
        });
    }
    fn item(&self, label: &str, enabled: bool, action: &'static str) -> ksni::MenuItem<Self> {
        StandardItem {
            label: label.into(),
            enabled,
            activate: Box::new(move |state: &mut Self| state.emit(action)),
            ..Default::default()
        }
        .into()
    }
}
impl ksni::Tray for State {
    fn id(&self) -> String {
        "dev.peep.app".into()
    }
    fn title(&self) -> String {
        format!("Peep — {}", self.title)
    }
    fn activate(&mut self, _: i32, _: i32) {
        self.emit("open");
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![if self.phase == Phase::Recording {
            self.recording.clone()
        } else {
            self.idle.clone()
        }]
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            self.item(&self.title, false, ""),
            ksni::MenuItem::Separator,
            self.item("Finish Dictation", self.phase == Phase::Recording, "stop"),
            self.item(
                "Cancel Dictation",
                matches!(
                    self.phase,
                    Phase::Downloading
                        | Phase::RequestingMicrophone
                        | Phase::Recording
                        | Phase::Transcribing
                ),
                "cancel",
            ),
            self.item("Copy Transcript", self.can_copy, "copy"),
            ksni::MenuItem::Separator,
            self.item("Open Peep", true, "open"),
            self.item("Quit Peep", true, "quit"),
        ]
    }
}
