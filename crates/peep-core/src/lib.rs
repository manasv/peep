#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    NeedsModel,
    Downloading,
    Preparing,
    RequestingMicrophone,
    Ready,
    Recording,
    Transcribing,
    Delivering,
    Canceling,
}
impl Phase {
    pub fn title(self) -> &'static str {
        match self {
            Self::NeedsModel => "Model needed",
            Self::Downloading => "Downloading model",
            Self::Preparing => "Preparing Parakeet",
            Self::Ready => "Ready to dictate",
            Self::RequestingMicrophone => "Starting dictation",
            Self::Recording => "Recording",
            Self::Transcribing => "Transcribing",
            Self::Delivering => "Inserting text",
            Self::Canceling => "Canceling",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Initialize,
    Prepare {
        id: u64,
        path: String,
    },
    Download {
        id: u64,
    },
    Start {
        id: u64,
    },
    StartCopy {
        id: u64,
    },
    Stop {
        id: u64,
    },
    Deliver {
        id: u64,
        text: String,
    },
    Cancel {
        id: u64,
    },
    Discard {
        id: u64,
    },
    Copy {
        text: String,
    },
    Feedback {
        text: String,
    },
    DismissFeedback,
    Settings {
        key: String,
        value: bool,
    },
    Accessibility,
    MicrophoneSettings,
    RecordShortcut,
    SetShortcut {
        chord: String,
    },
    CancelShortcut,
    RefreshPermissions,
    Presentation {
        title: String,
        phase: Phase,
        can_copy: bool,
    },
    Shutdown,
}

#[derive(Clone, Debug, Default)]
pub struct Event {
    pub kind: String,
    pub id: u64,
    pub text: String,
    pub path: String,
    pub progress: f64,
    pub shortcut: String,
    pub shortcut_enabled: bool,
    pub shortcut_available: bool,
    pub shortcut_portal: bool,
    pub indicator: bool,
    pub accessibility: bool,
    pub microphone: String,
}

#[derive(Default, Debug)]
pub struct Session {
    pub phase: Phase,
    pub transcript: String,
    pub message: String,
    pub paste_unconfirmed: bool,
    pub progress: f64,
    id: u64,
    model_ready: bool,
}
impl Session {
    pub fn can_cancel(&self) -> bool {
        matches!(
            self.phase,
            Phase::Downloading
                | Phase::RequestingMicrophone
                | Phase::Recording
                | Phase::Transcribing
        )
    }
    pub fn busy(&self) -> bool {
        !matches!(self.phase, Phase::Ready | Phase::NeedsModel)
    }
    pub fn start(&mut self) -> Vec<Command> {
        if self.phase != Phase::Ready {
            return vec![];
        }
        self.id += 1;
        self.transcript.clear();
        self.paste_unconfirmed = false;
        self.message.clear();
        self.phase = Phase::RequestingMicrophone;
        vec![Command::Start { id: self.id }]
    }
    pub fn stop(&mut self) -> Vec<Command> {
        if self.phase != Phase::Recording {
            return vec![];
        }
        self.phase = Phase::Transcribing;
        vec![Command::Stop { id: self.id }]
    }
    pub fn cancel(&mut self) -> Vec<Command> {
        if !self.can_cancel() {
            return vec![];
        }
        self.phase = Phase::Canceling;
        vec![Command::Cancel { id: self.id }]
    }
    pub fn download(&mut self) -> Vec<Command> {
        if self.phase != Phase::NeedsModel {
            return vec![];
        }
        self.id += 1;
        self.message.clear();
        self.progress = 0.;
        self.phase = Phase::Downloading;
        vec![Command::Download { id: self.id }]
    }
    pub fn toggle(&mut self) -> Vec<Command> {
        match self.phase {
            Phase::Ready => self.start(),
            Phase::Recording => self.stop(),
            Phase::RequestingMicrophone => self.cancel(),
            _ => vec![],
        }
    }
    fn finish(&mut self) {
        self.phase = if self.model_ready {
            Phase::Ready
        } else {
            Phase::NeedsModel
        };
    }
    pub fn event(&mut self, event: &Event) -> Vec<Command> {
        if event.kind == "model_found" && matches!(self.phase, Phase::NeedsModel | Phase::Ready) {
            self.id += 1;
            self.message.clear();
            self.phase = Phase::Preparing;
            return vec![Command::Prepare {
                id: self.id,
                path: event.path.clone(),
            }];
        }
        if event.id != self.id {
            return vec![];
        }
        match (event.kind.as_str(), self.phase) {
            ("progress", Phase::Downloading) => {
                self.progress = event.progress.clamp(self.progress, 1.)
            }
            ("preparing", Phase::Downloading) => self.phase = Phase::Preparing,
            ("ready", Phase::Preparing) => {
                self.model_ready = true;
                self.finish();
            }
            ("recording", Phase::RequestingMicrophone) => self.phase = Phase::Recording,
            ("limit", Phase::Recording) => return self.stop(),
            ("transcript", Phase::Transcribing) => {
                self.transcript = event.text.trim().to_string();
                if self.transcript.is_empty() {
                    self.message = "No speech detected. Try again.".into();
                } else {
                    self.phase = Phase::Delivering;
                    return vec![Command::Deliver {
                        id: self.id,
                        text: self.transcript.clone(),
                    }];
                }
                self.finish();
                return vec![Command::Discard { id: self.id }];
            }
            ("copy_ready", Phase::Delivering) => {
                self.message = "Your text is ready to copy.".into();
                self.finish();
                return vec![Command::Discard { id: self.id }];
            }
            ("delivered", Phase::Delivering) => {
                self.message.clear();
                self.transcript.clear();
                self.finish();
                return vec![Command::Discard { id: self.id }];
            }
            ("paste_sent", Phase::Delivering) => {
                self.message = event.text.clone();
                self.paste_unconfirmed = true;
                self.finish();
                return vec![Command::Discard { id: self.id }];
            }
            ("error", phase) if phase != Phase::Canceling && self.busy() => {
                self.message = event.text.clone();
                if phase == Phase::Delivering {
                    self.message.push_str(" Your transcript is ready to copy.");
                }
                self.finish();
                return vec![Command::Discard { id: self.id }];
            }
            ("canceled", Phase::Canceling) => {
                self.message.clear();
                self.finish();
            }
            _ => {}
        }
        vec![]
    }
    pub fn presentation(&self) -> Command {
        Command::Presentation {
            title: self.phase.title().into(),
            phase: self.phase,
            can_copy: !self.transcript.is_empty() && !self.busy(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(kind: &str, id: u64, text: &str) -> Event {
        Event {
            kind: kind.into(),
            id,
            text: text.into(),
            ..Default::default()
        }
    }
    fn ready() -> Session {
        Session {
            phase: Phase::Ready,
            model_ready: true,
            ..Default::default()
        }
    }
    #[test]
    fn unconfirmed_paste_keeps_copy_available_without_blocking_the_next_recording() {
        let mut session = ready();
        session.start();
        session.event(&event("recording", 1, ""));
        session.stop();
        session.event(&event("transcript", 1, "testing testing"));
        assert_eq!(session.phase, Phase::Delivering);
        assert_eq!(
            session.event(&event("paste_sent", 1, "Paste sent.")),
            vec![Command::Discard { id: 1 }]
        );
        assert_eq!(session.phase, Phase::Ready);
        assert!(session.paste_unconfirmed);
        assert_eq!(session.transcript, "testing testing");
        assert!(matches!(
            session.presentation(),
            Command::Presentation { can_copy: true, .. }
        ));
        assert_eq!(session.toggle(), vec![Command::Start { id: 2 }]);
        assert!(!session.paste_unconfirmed);
        assert!(session.transcript.is_empty());
        session.event(&event("paste_sent", 1, "stale"));
        assert!(!session.paste_unconfirmed);
        assert!(session.message.is_empty());
    }
    #[test]
    fn model_selection_can_replace_a_ready_model_and_preserve_it_on_failure() {
        let mut s = ready();
        let effects = s.event(&Event {
            kind: "model_found".into(),
            path: "/models/replacement".into(),
            ..Default::default()
        });
        assert_eq!(s.phase, Phase::Preparing);
        assert!(
            matches!(effects.as_slice(), [Command::Prepare { id: 1, path }] if path == "/models/replacement")
        );
        s.event(&event("error", 1, "Invalid model folder"));
        assert_eq!(s.phase, Phase::Ready);
        assert_eq!(s.message, "Invalid model folder");
        assert!(matches!(s.start().as_slice(), [Command::Start { id: 2 }]));
    }

    #[test]
    fn model_selection_cannot_interrupt_active_dictation() {
        let mut s = ready();
        s.start();
        let effects = s.event(&Event {
            kind: "model_found".into(),
            path: "/models/replacement".into(),
            ..Default::default()
        });
        assert!(effects.is_empty());
        assert_eq!(s.phase, Phase::RequestingMicrophone);
        s.event(&event("recording", 1, ""));
        assert_eq!(s.phase, Phase::Recording);
    }

    #[test]
    fn canceled_result_never_delivers_or_overwrites_next_session() {
        let mut s = ready();
        s.start();
        s.stop();
        s.event(&event("recording", 1, ""));
        s.stop();
        s.cancel();
        assert!(s.event(&event("transcript", 1, "stale")).is_empty());
        s.event(&event("canceled", 1, ""));
        s.start();
        assert!(s.event(&event("transcript", 1, "stale")).is_empty());
        assert!(s.transcript.is_empty());
        assert_eq!(s.phase, Phase::RequestingMicrophone);
    }
    #[test]
    fn global_transcript_delivers_exactly_once_and_survives_failure() {
        let mut s = ready();
        s.start();
        s.event(&event("recording", 1, ""));
        s.stop();
        let e = event("transcript", 1, "hello");
        assert_eq!(
            s.event(&e),
            vec![Command::Deliver {
                id: 1,
                text: "hello".into()
            }]
        );
        assert!(s.event(&e).is_empty());
        s.event(&event("error", 1, "Focus changed."));
        assert_eq!(s.transcript, "hello");
        assert_eq!(s.phase, Phase::Ready);
        assert!(s.message.contains("ready to copy"));
    }
    #[test]
    fn desktop_copy_result_preserves_text_until_explicitly_copied() {
        let mut s = ready();
        s.start();
        s.event(&event("recording", 1, ""));
        s.stop();
        s.event(&event("transcript", 1, "hello"));
        assert_eq!(
            s.event(&event("copy_ready", 1, "")),
            vec![Command::Discard { id: 1 }]
        );
        assert_eq!(s.phase, Phase::Ready);
        assert_eq!(s.transcript, "hello");
        assert!(matches!(
            s.presentation(),
            Command::Presentation { can_copy: true, .. }
        ));
        assert!(s.event(&event("copy_ready", 1, "")).is_empty());
        assert_eq!(s.transcript, "hello");
    }

    #[test]
    fn canceled_or_stale_copy_result_cannot_replace_a_new_session() {
        let mut s = ready();
        s.start();
        s.cancel();
        assert!(s.event(&event("copy_ready", 1, "")).is_empty());
        s.event(&event("canceled", 1, ""));
        s.start();
        assert!(s.event(&event("copy_ready", 1, "")).is_empty());
        assert_eq!(s.phase, Phase::RequestingMicrophone);
        assert!(s.transcript.is_empty());
    }

    #[test]
    fn silence_never_inserts() {
        let mut s = ready();
        s.start();
        s.event(&event("recording", 1, ""));
        s.stop();
        assert_eq!(
            s.event(&event("transcript", 1, "   ")),
            vec![Command::Discard { id: 1 }]
        );
        assert_eq!(s.phase, Phase::Ready);
    }
    #[test]
    fn every_nonempty_result_inserts_and_success_keeps_no_preview() {
        let mut s = ready();
        s.start();
        s.event(&event("recording", 1, ""));
        s.stop();
        assert_eq!(
            s.event(&event("transcript", 1, "hello")),
            vec![Command::Deliver {
                id: 1,
                text: "hello".into()
            }]
        );
        s.event(&event("delivered", 1, "Inserted into TextEdit."));
        assert!(s.transcript.is_empty());
        assert!(s.message.is_empty());
        assert_eq!(s.phase, Phase::Ready);
    }
    #[test]
    fn cancel_download_waits_for_cleanup_before_retry() {
        let mut s = Session::default();
        s.download();
        s.cancel();
        assert!(s.download().is_empty());
        s.event(&event("ready", 1, ""));
        assert_eq!(s.phase, Phase::Canceling);
        s.event(&event("canceled", 1, ""));
        assert_eq!(s.phase, Phase::NeedsModel);
        assert!(s.message.is_empty());
        assert!(!s.download().is_empty());
    }
    #[test]
    fn duplicate_actions_and_out_of_order_events_are_ignored() {
        let mut s = ready();
        s.start();
        assert!(s.start().is_empty());
        assert!(s.stop().is_empty());
        s.event(&event("transcript", 1, "too early"));
        assert!(s.transcript.is_empty());
        s.event(&event("recording", 1, ""));
        assert!(!s.stop().is_empty());
        assert!(s.stop().is_empty());
    }
}
