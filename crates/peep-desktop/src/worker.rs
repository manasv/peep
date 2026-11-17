use anyhow::{Result, anyhow, bail};
use peep_core::Event;
use peep_runtime::{audio, models::ONNX};
use sherpa_rs::transducer::{TransducerConfig, TransducerRecognizer};
use std::{
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

pub enum Job {
    Prepare(u64, PathBuf, CancellationToken),
    Download(u64, CancellationToken),
    Transcribe(u64, Vec<f32>, u32, CancellationToken),
    Shutdown,
}

pub fn spawn(
    events: async_channel::Sender<Event>,
) -> (mpsc::Sender<Job>, async_channel::Receiver<()>) {
    let (tx, rx) = mpsc::channel();
    let (cleanup_tx, cleanup_rx) = async_channel::bounded(1);
    std::thread::Builder::new()
        .name("peep-onnx".into())
        .spawn(move || {
            let mut pipeline = None;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("download runtime");
            for job in rx {
                let (id, cancel, result) = match job {
                    Job::Prepare(id, path, cancel) => {
                        let result = load(&path, &cancel).map(|model| {
                            pipeline = Some(model);
                            Event {
                                kind: "ready".into(),
                                id,
                                path: path.to_string_lossy().into_owned(),
                                ..Default::default()
                            }
                        });
                        (id, cancel, result)
                    }
                    Job::Download(id, cancel) => {
                        let result = runtime
                            .block_on(ONNX.download(id, &events, &cancel))
                            .and_then(|path| {
                                let _ = events.try_send(Event {
                                    kind: "preparing".into(),
                                    id,
                                    ..Default::default()
                                });
                                pipeline = Some(load(&path, &cancel)?);
                                Ok(Event {
                                    kind: "ready".into(),
                                    id,
                                    path: path.to_string_lossy().into_owned(),
                                    ..Default::default()
                                })
                            });
                        (id, cancel, result)
                    }
                    Job::Transcribe(id, samples, rate, cancel) => {
                        let result = (|| -> Result<Event> {
                            let mut samples = audio::resample(&samples, rate)?;
                            if samples.len() < 4800 {
                                bail!(
                                    "That recording was too short. Speak for at least 0.3 seconds."
                                );
                            }
                            let model = pipeline
                                .as_mut()
                                .ok_or_else(|| anyhow!("The speech model is not prepared."))?;
                            let text = transcribe(model, &mut samples, &cancel)?;
                            Ok(Event {
                                kind: "transcript".into(),
                                id,
                                text,
                                ..Default::default()
                            })
                        })();
                        (id, cancel, result)
                    }
                    Job::Shutdown => break,
                };
                let event = if cancel.is_cancelled() {
                    Event {
                        kind: "canceled".into(),
                        id,
                        ..Default::default()
                    }
                } else {
                    result.unwrap_or_else(|error| Event {
                        kind: "error".into(),
                        id,
                        text: error.to_string(),
                        ..Default::default()
                    })
                };
                let _ = events.try_send(event);
            }
            drop(pipeline);
            let _ = cleanup_tx.try_send(());
        })
        .expect("start ONNX worker");
    (tx, cleanup_rx)
}

fn load(path: &std::path::Path, cancel: &CancellationToken) -> Result<TransducerRecognizer> {
    ONNX.verify(path, cancel)?;
    let file = |name| path.join(name).to_string_lossy().into_owned();
    let model = TransducerRecognizer::new(TransducerConfig {
        encoder: file("encoder.int8.onnx"),
        decoder: file("decoder.int8.onnx"),
        joiner: file("joiner.int8.onnx"),
        tokens: file("tokens.txt"),
        sample_rate: 16_000,
        feature_dim: 80,
        model_type: "nemo_transducer".into(),
        decoding_method: "greedy_search".into(),
        provider: Some("cpu".into()),
        num_threads: std::thread::available_parallelism().map_or(1, |n| n.get().min(4)) as i32,
        ..Default::default()
    })
    .map_err(|error| anyhow!("Could not load Parakeet: {error}"))?;
    if cancel.is_cancelled() {
        bail!("Canceled");
    }
    Ok(model)
}

fn transcribe(
    model: &mut TransducerRecognizer,
    samples: &mut [f32],
    cancel: &CancellationToken,
) -> Result<String> {
    let peak = samples.iter().copied().map(f32::abs).fold(0., f32::max);
    if peak < 1e-5 {
        return Ok(String::new());
    }
    let gain = (0.8 / peak).clamp(1., 20.);
    for sample in samples.iter_mut() {
        *sample *= gain;
    }
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut text = Vec::new();
    let mut remaining = &samples[..];
    while !remaining.is_empty() {
        if cancel.is_cancelled() {
            bail!("Canceled");
        }
        if Instant::now() >= deadline {
            bail!("Transcription timed out. Try a shorter recording.");
        }
        let end = chunk_end(remaining);
        let result = model.transcribe(16_000, &remaining[..end]);
        if !result.trim().is_empty() {
            text.push(result.trim().to_owned());
        }
        remaining = &remaining[end..];
    }
    if cancel.is_cancelled() {
        bail!("Canceled");
    }
    if Instant::now() >= deadline {
        bail!("Transcription timed out. Try a shorter recording.");
    }
    Ok(text.join(" "))
}

fn chunk_end(samples: &[f32]) -> usize {
    const MAX: usize = 20 * 16_000;
    if samples.len() <= MAX {
        return samples.len();
    }
    let (frames, _) = samples[..MAX].as_chunks::<160>();
    let mut quiet = 0;
    for index in (10 * 100..frames.len()).rev() {
        let energy = frames[index].iter().map(|x| x * x).sum::<f32>() / 160.;
        if energy < 0.0001 {
            quiet += 1;
            if quiet == 25 {
                return (index + 12) * 160;
            }
        } else {
            quiet = 0;
        }
    }
    MAX
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_audio_splits_at_a_pause_without_losing_samples() {
        let mut samples = vec![0.1; 45 * 16_000];
        samples[18 * 16_000..19 * 16_000].fill(0.);
        let end = chunk_end(&samples);
        assert!((18 * 16_000..19 * 16_000).contains(&end));
        assert_eq!(chunk_end(&samples[end..]), 20 * 16_000);
        assert_eq!(chunk_end(&samples[..12_345]), 12_345);
    }

    #[test]
    fn local_fixture_transcribes_and_canceled_jobs_never_publish_text() {
        let (Ok(model), Ok(audio)) = (
            std::env::var("PEEP_TEST_ONNX_MODEL_PATH"),
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
            .send(Job::Prepare(1, model.into(), CancellationToken::new()))
            .unwrap();
        let ready = rx.recv_blocking().unwrap();
        assert_eq!(ready.kind, "ready", "{}", ready.text);
        let cancel = CancellationToken::new();
        cancel.cancel();
        worker
            .send(Job::Transcribe(
                2,
                samples.clone(),
                spec.sample_rate,
                cancel,
            ))
            .unwrap();
        assert_eq!(rx.recv_blocking().unwrap().kind, "canceled");
        worker
            .send(Job::Transcribe(
                3,
                samples,
                spec.sample_rate,
                CancellationToken::new(),
            ))
            .unwrap();
        let text = rx.recv_blocking().unwrap();
        assert_eq!(text.kind, "transcript", "{}", text.text);
        assert!(!text.text.trim().is_empty());
        if let Ok(expected) = std::env::var("PEEP_TEST_EXPECTED_TEXT") {
            assert_eq!(
                text.text.to_lowercase().trim(),
                expected.to_lowercase().trim()
            );
        }
        worker.send(Job::Shutdown).unwrap();
        cleanup.recv_blocking().unwrap();
    }
}
