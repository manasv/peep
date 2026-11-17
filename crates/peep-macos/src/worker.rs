use anyhow::{Result, anyhow, bail};
use peep_core::Event;
use scriptrs::{
    LongFormTranscriptionPipeline, ParakeetDeployment, TranscriptionControl, TranscriptionDeadline,
    WindowLimit,
};
use std::{path::PathBuf, sync::mpsc, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Control {
    pub cancel: CancellationToken,
    pub inference: TranscriptionControl,
}
impl Control {
    pub fn new() -> Self {
        Self {
            cancel: CancellationToken::new(),
            inference: TranscriptionControl::new(
                TranscriptionDeadline::from_now(Duration::from_secs(180)),
                WindowLimit::new(64).expect("nonzero window limit"),
            ),
        }
    }
    pub fn cancel(&self) {
        self.cancel.cancel();
        self.inference.cancel();
    }
}
pub enum Job {
    Prepare(u64, PathBuf, Control),
    Download(u64, Control),
    Transcribe(u64, Vec<f32>, u32, Control),
    Shutdown,
}
pub fn spawn(
    events: async_channel::Sender<Event>,
) -> (mpsc::Sender<Job>, async_channel::Receiver<()>) {
    let (tx, rx) = mpsc::channel();
    let (cleanup_tx, cleanup_rx) = async_channel::bounded(1);
    std::thread::Builder::new()
        .name("peep-speech".into())
        .spawn(move || {
            let mut pipeline: Option<LongFormTranscriptionPipeline> = None;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("download runtime");
            for job in rx {
                let (id, control, result) = match job {
                    Job::Prepare(id, path, control) => {
                        let result = load(&path, &control).map(|model| {
                            pipeline = Some(model);
                            Event {
                                kind: "ready".into(),
                                id,
                                path: path.to_string_lossy().into_owned(),
                                ..Default::default()
                            }
                        });
                        (id, control, result)
                    }
                    Job::Download(id, control) => {
                        let result = runtime
                            .block_on(crate::models::download(id, &events, &control.cancel))
                            .and_then(|path| {
                                let _ = events.try_send(Event {
                                    kind: "preparing".into(),
                                    id,
                                    ..Default::default()
                                });
                                pipeline = Some(load(&path, &control)?);
                                Ok(Event {
                                    kind: "ready".into(),
                                    id,
                                    path: path.to_string_lossy().into_owned(),
                                    ..Default::default()
                                })
                            });
                        (id, control, result)
                    }
                    Job::Transcribe(id, samples, rate, control) => {
                        let result = (|| -> Result<Event> {
                            let audio = crate::audio::resample(&samples, rate)?;
                            if audio.len() < 4800 {
                                bail!(
                                    "That recording was too short. Speak for at least 0.3 seconds."
                                );
                            }
                            if control.cancel.is_cancelled() {
                                bail!("Canceled");
                            }
                            let model = pipeline
                                .as_ref()
                                .ok_or_else(|| anyhow!("The speech model is not prepared."))?;
                            let result = model
                                .run_with_preprocessor(
                                    &audio,
                                    model.config(),
                                    &control.inference,
                                    &UnchangedAudio,
                                )
                                .map_err(|e| anyhow!("{e}"))?;
                            Ok(Event {
                                kind: "transcript".into(),
                                id,
                                text: result.text,
                                ..Default::default()
                            })
                        })();
                        (id, control, result)
                    }
                    Job::Shutdown => {
                        drop(pipeline);
                        let _ = cleanup_tx.try_send(());
                        break;
                    }
                };
                let event = if control.cancel.is_cancelled() {
                    Event {
                        kind: "canceled".into(),
                        id,
                        ..Default::default()
                    }
                } else {
                    result.unwrap_or_else(|e| Event {
                        kind: "error".into(),
                        id,
                        text: e.to_string(),
                        ..Default::default()
                    })
                };
                let _ = events.try_send(event);
            }
        })
        .expect("start speech worker");
    (tx, cleanup_rx)
}
fn load(path: &std::path::Path, control: &Control) -> Result<LongFormTranscriptionPipeline> {
    crate::models::verify(path, &control.cancel)?;
    let model =
        LongFormTranscriptionPipeline::from_dir_with_deployment(path, ParakeetDeployment::v3())?;
    if control.cancel.is_cancelled() {
        bail!("Canceled");
    }
    Ok(model)
}

struct UnchangedAudio;
impl scriptrs::WindowPreprocessor for UnchangedAudio {
    type Error = std::convert::Infallible;
    fn process(
        &self,
        _: &mut scriptrs::AudioWindowMut<'_>,
        _: &TranscriptionControl,
    ) -> std::result::Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_fixture_transcribes() {
        let (Ok(model), Ok(audio)) = (
            std::env::var("PEEP_TEST_MODEL_PATH"),
            std::env::var("PEEP_TEST_AUDIO_PATH"),
        ) else {
            return;
        };
        let mut reader = hound::WavReader::open(audio).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.)
            .collect();
        let (events, rx) = async_channel::unbounded();
        let (worker, cleanup) = spawn(events);
        worker
            .send(Job::Prepare(1, model.into(), Control::new()))
            .unwrap();
        let ready = rx.recv_blocking().unwrap();
        assert_eq!(ready.kind, "ready", "{}", ready.text);
        worker
            .send(Job::Transcribe(
                2,
                samples,
                spec.sample_rate,
                Control::new(),
            ))
            .unwrap();
        let text = rx.recv_blocking().unwrap();
        assert_eq!(text.kind, "transcript", "{}", text.text);
        assert!(!text.text.is_empty());
        worker.send(Job::Shutdown).unwrap();
        cleanup.recv_blocking().unwrap();
    }
}
