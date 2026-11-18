#[cfg(all(feature = "desktop-ui", target_os = "windows"))]
pub mod tray;
#[cfg(all(feature = "desktop-ui", target_os = "linux"))]
#[path = "tray_linux.rs"]
pub mod tray;

use anyhow::{Result, bail};
use peep_core::Event;
use std::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[cfg(all(unix, any(target_os = "linux", feature = "linux-integration")))]
pub mod control;
#[cfg(all(unix, any(target_os = "linux", feature = "linux-integration")))]
pub mod linux;
#[cfg(all(unix, any(target_os = "linux", feature = "linux-integration")))]
pub mod notifications;
#[cfg(all(unix, any(target_os = "linux", feature = "linux-integration")))]
pub mod portal;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "linux")]
pub use linux::Context as NativeContext;
#[cfg(target_os = "windows")]
pub use windows::Context as NativeContext;
#[cfg(target_os = "windows")]
pub use windows::show_toast_window;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub text: String,
    pub start: usize,
    pub end: usize,
}
impl Snapshot {
    pub fn replace(&self, text: &str) -> Result<String> {
        let mut characters: Vec<char> = self.text.chars().collect();
        if self.start > self.end || self.end > characters.len() {
            bail!("Invalid editor selection.");
        }
        characters.splice(self.start..self.end, text.chars());
        Ok(characters.into_iter().collect())
    }
}

pub enum Job {
    Capture(u64, CancellationToken),
    Deliver(u64, String),
    Discard(u64),
    EnableAccessibility,
    CheckAccessibility,
    Shutdown,
}

pub fn spawn(
    events: async_channel::Sender<Event>,
) -> (mpsc::Sender<Job>, async_channel::Receiver<()>) {
    let (tx, rx) = mpsc::channel();
    let (done_tx, done) = async_channel::bounded(1);
    std::thread::Builder::new()
        .name("peep-delivery".into())
        .spawn(move || {
            #[cfg(target_os = "windows")]
            let mut context = windows::Context::new();
            #[cfg(target_os = "linux")]
            let mut context = linux::Context::new();
            #[cfg(not(any(target_os = "linux", target_os = "windows")))]
            let mut context = unsupported::Context::new();
            let mut target = None;
            for job in rx {
                match job {
                    Job::Capture(id, cancel) => {
                        let result = context
                            .as_mut()
                            .map_err(|e| anyhow::anyhow!("{e}"))
                            .and_then(|context| context.capture());
                        if !cancel.is_cancelled() {
                            target = Some((id, result, cancel));
                            let _ = events.try_send(Event {
                                kind: "target_ready".into(),
                                id,
                                ..Default::default()
                            });
                        }
                    }
                    Job::Deliver(id, text) => {
                        let result = match target.take() {
                            Some((expected, result, cancel))
                                if expected == id && !cancel.is_cancelled() =>
                            {
                                result.and_then(|target| {
                                    context
                                        .as_mut()
                                        .map_err(|e| anyhow::anyhow!("{e}"))
                                        .and_then(|context| context.insert(target, &text, &cancel))
                                })
                            }
                            _ => Err(anyhow::anyhow!(
                                "The original editor is no longer available."
                            )),
                        };
                        let (kind, text) = match result {
                            Ok(()) => ("delivered", String::new()),
                            Err(error) => ("error", error.to_string()),
                        };
                        let _ = events.try_send(Event {
                            kind: kind.into(),
                            id,
                            text,
                            ..Default::default()
                        });
                    }
                    Job::Discard(id) => {
                        if target
                            .as_ref()
                            .is_some_and(|(expected, _, _)| *expected == id)
                        {
                            target = None;
                        }
                    }
                    job @ (Job::EnableAccessibility | Job::CheckAccessibility) => {
                        let result = context
                            .as_mut()
                            .map_err(|e| anyhow::anyhow!("{e}"))
                            .and_then(|context| {
                                if matches!(job, Job::EnableAccessibility) {
                                    context.enable_accessibility()?;
                                }
                                context.accessibility_status()
                            });
                        let _ = events.try_send(Event {
                            kind: "insertion_permission".into(),
                            accessibility: result.as_ref().is_ok_and(|allowed| *allowed),
                            text: result.err().map_or(String::new(), |e| e.to_string()),
                            ..Default::default()
                        });
                    }
                    Job::Shutdown => break,
                }
            }
            drop(target);
            drop(context);
            let _ = done_tx.try_send(());
        })
        .expect("start delivery worker");
    (tx, done)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod unsupported {
    use super::*;
    pub struct Context;
    impl Context {
        pub fn new() -> Result<Self> {
            Ok(Self)
        }
        pub fn capture(&mut self) -> Result<()> {
            bail!("Native insertion is unavailable in desktop preview.")
        }
        pub fn insert(&mut self, _: (), _: &str, _: &CancellationToken) -> Result<()> {
            bail!("Native insertion is unavailable in desktop preview.")
        }
        pub fn enable_accessibility(&mut self) -> Result<()> {
            bail!("Native insertion is unavailable in desktop preview.")
        }
        pub fn accessibility_status(&mut self) -> Result<bool> {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_preserves_unicode_and_surrounding_text() {
        let snapshot = Snapshot {
            text: "a😀b".into(),
            start: 1,
            end: 2,
        };
        assert_eq!(snapshot.replace("hello").unwrap(), "ahellob");
        assert!(
            Snapshot {
                end: 9,
                ..snapshot.clone()
            }
            .replace("x")
            .is_err()
        );
        assert!(
            Snapshot {
                start: 2,
                end: 1,
                ..snapshot
            }
            .replace("x")
            .is_err()
        );
    }
}
