use anyhow::{Result, anyhow, bail};
use cpal::{
    FromSample, Sample, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use peep_core::Event;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

pub struct Audio {
    stream: Option<cpal::Stream>,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: u32,
}
impl Audio {
    pub fn start(id: u64, events: async_channel::Sender<Event>) -> Result<Self> {
        let device = cpal::default_host()
            .default_input_device()
            .ok_or_else(|| anyhow!("No microphone is available."))?;
        let config = device.default_input_config()?;
        let rate = config.sample_rate();
        let channels = config.channels() as usize;
        if !(8_000..=192_000).contains(&rate) || !(1..=32).contains(&channels) {
            bail!("Unsupported microphone format.");
        }
        let samples = Arc::new(Mutex::new(Vec::with_capacity(rate as usize * 120)));
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(
                &device,
                config.into(),
                samples.clone(),
                events,
                id,
                channels,
            )?,
            cpal::SampleFormat::I16 => build::<i16>(
                &device,
                config.into(),
                samples.clone(),
                events,
                id,
                channels,
            )?,
            cpal::SampleFormat::I32 => build::<i32>(
                &device,
                config.into(),
                samples.clone(),
                events,
                id,
                channels,
            )?,
            _ => bail!("This microphone’s sample format is unsupported."),
        };
        stream.play()?;
        Ok(Self {
            stream: Some(stream),
            samples,
            rate,
        })
    }
    pub fn stop(mut self) -> Result<(Vec<f32>, u32)> {
        self.stream.take();
        let samples = std::mem::take(
            &mut *self
                .samples
                .lock()
                .map_err(|_| anyhow!("Audio buffer unavailable."))?,
        );
        Ok((samples, self.rate))
    }
}
fn build<T: SizedSample + Sample>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    samples: Arc<Mutex<Vec<f32>>>,
    events: async_channel::Sender<Event>,
    id: u64,
    channels: usize,
) -> Result<cpal::Stream>
where
    f32: FromSample<T>,
{
    let limit = config.sample_rate as usize * 120;
    let mut meter = LevelMeter::new(config.sample_rate);
    let stopped = Arc::new(AtomicBool::new(false));
    let stopped_error = stopped.clone();
    let events_error = events.clone();
    Ok(device.build_input_stream::<T, _, _>(
        config,
        move |data, _| {
            if stopped.load(Ordering::Relaxed) {
                return;
            }
            let Ok(mut output) = samples.try_lock() else {
                if !stopped.swap(true, Ordering::Relaxed) {
                    let _ = events.try_send(Event {
                        kind: "capture_error".into(),
                        id,
                        text: "Microphone audio was interrupted.".into(),
                        ..Default::default()
                    });
                }
                return;
            };
            for frame in data.chunks_exact(channels) {
                if output.len() == limit {
                    if !stopped.swap(true, Ordering::Relaxed) {
                        let _ = events.try_send(Event {
                            kind: "limit".into(),
                            id,
                            ..Default::default()
                        });
                    }
                    break;
                }
                let value =
                    frame.iter().map(|x| f32::from_sample(*x)).sum::<f32>() / channels as f32;
                if !value.is_finite() {
                    stopped.store(true, Ordering::Relaxed);
                    let _ = events.try_send(Event {
                        kind: "capture_error".into(),
                        id,
                        text: "Microphone supplied invalid audio.".into(),
                        ..Default::default()
                    });
                    break;
                }
                output.push(value);
                if let Some(level) = meter.push(value) {
                    let _ = events.try_send(Event {
                        kind: "audio_level".into(),
                        id,
                        progress: level,
                        ..Default::default()
                    });
                }
            }
        },
        move |_| {
            if !stopped_error.swap(true, Ordering::Relaxed) {
                let _ = events_error.try_send(Event {
                    kind: "capture_error".into(),
                    id,
                    text: "The microphone disconnected or its format changed.".into(),
                    ..Default::default()
                });
            }
        },
        None,
    )?)
}

struct LevelMeter {
    interval: u32,
    count: u32,
    energy: f64,
}
impl LevelMeter {
    fn new(rate: u32) -> Self {
        Self {
            interval: rate.div_ceil(30).max(1),
            count: 0,
            energy: 0.,
        }
    }
    fn push(&mut self, sample: f32) -> Option<f64> {
        self.energy += f64::from(sample).powi(2);
        self.count += 1;
        if self.count < self.interval {
            return None;
        }
        let rms = (self.energy / f64::from(self.count)).sqrt();
        self.count = 0;
        self.energy = 0.;
        Some(((20. * rms.max(1e-3).log10() + 60.) / 60.).clamp(0., 1.))
    }
}

pub fn resample(samples: &[f32], rate: u32) -> Result<Vec<f32>> {
    if samples.is_empty() || samples.iter().any(|x| !x.is_finite()) {
        bail!("No valid microphone audio was captured.");
    }
    if !(8_000..=192_000).contains(&rate) || samples.len() > rate as usize * 120 {
        bail!("Recording exceeds the supported format or two-minute limit.");
    }
    if rate == 16_000 {
        return Ok(samples.to_vec());
    }
    let parameters = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 128,
        window: WindowFunction::BlackmanHarris2,
    };
    let mut resampler = SincFixedIn::<f32>::new(16_000. / rate as f64, 1., parameters, 1024, 1)?;
    let delay = resampler.output_delay();
    let wanted = samples.len() * 16_000 / rate as usize;
    let mut output = Vec::with_capacity(wanted + 4096);
    for chunk in samples.chunks(1024) {
        let mut padded = vec![0.; 1024];
        padded[..chunk.len()].copy_from_slice(chunk);
        output.extend(resampler.process(&[padded], None)?.remove(0));
    }
    while output.len() < wanted + delay {
        output.extend(resampler.process(&[vec![0.; 1024]], None)?.remove(0));
    }
    Ok(output[delay..delay + wanted].to_vec())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn meter_tracks_volume_and_limits_update_rate() {
        for (amplitude, expected) in [(0., 0.), (0.01, 1. / 3.), (0.1, 2. / 3.), (1., 1.)] {
            let mut meter = LevelMeter::new(48_000);
            let levels: Vec<_> = (0..48_000).filter_map(|_| meter.push(amplitude)).collect();
            assert_eq!(levels.len(), 30);
            assert!(levels.iter().all(|level| (level - expected).abs() < 1e-6));
        }
    }
    #[test]
    fn conversion_preserves_duration_and_audible_tone() {
        for rate in [16_000, 44_100, 48_000, 96_000] {
            let input: Vec<f32> = (0..rate)
                .map(|n| (n as f32 * 440. * 2. * std::f32::consts::PI / rate as f32).sin() * 0.2)
                .collect();
            let output = resample(&input, rate).unwrap();
            assert_eq!(output.len(), 16_000);
            assert!(output.iter().all(|x| x.is_finite()));
            assert!(output.iter().map(|x| x * x).sum::<f32>() / output.len() as f32 > 0.01);
        }
    }
    #[test]
    fn invalid_and_empty_input_are_rejected() {
        assert!(resample(&[], 48_000).is_err());
        assert!(resample(&[f32::NAN], 48_000).is_err());
        assert!(resample(&[0.; 4], 0).is_err());
    }
}
