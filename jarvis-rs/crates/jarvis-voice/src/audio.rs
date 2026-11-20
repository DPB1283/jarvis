use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, FromSample, SampleFormat, SizedSample};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::{Arc, mpsc as sync};
use std::thread;
use tokio::sync::mpsc;

pub const INPUT_RATE: u32 = 16_000;
pub const TTS_RATE: u32 = 24_000;
pub const FRAME_SAMPLES: usize = 512;

pub struct Resampler {
    step: f64,
    previous: Option<f32>,
    position: u64,
    next: f64,
}

impl Resampler {
    pub fn new(input: u32, output: u32) -> Self {
        Self {
            step: f64::from(input) / f64::from(output),
            previous: None,
            position: 0,
            next: 0.0,
        }
    }

    pub fn push(&mut self, sample: f32, mut output: impl FnMut(f32)) {
        let position = self.position as f64;
        while self.next <= position {
            let value = match self.previous {
                Some(previous) => {
                    previous + (sample - previous) * (self.next - position + 1.0) as f32
                }
                None => sample,
            };
            output(value);
            self.next += self.step;
        }
        self.previous = Some(sample);
        self.position += 1;
    }
}

pub fn device_names(input: bool) -> Result<Vec<String>> {
    let host = cpal::default_host();
    let mut names: Vec<_> = if input {
        host.input_devices()?
            .filter_map(|device| device.name().ok())
            .collect()
    } else {
        host.output_devices()?
            .filter_map(|device| device.name().ok())
            .collect()
    };
    names.sort();
    names.dedup();
    Ok(names)
}

fn device(name: &str, input: bool) -> Result<Device> {
    let host = cpal::default_host();
    let selected = if name.is_empty() {
        None
    } else if input {
        host.input_devices()?
            .find(|device| device.name().is_ok_and(|current| current == name))
    } else {
        host.output_devices()?
            .find(|device| device.name().is_ok_and(|current| current == name))
    };
    selected
        .or_else(|| {
            if input {
                host.default_input_device()
            } else {
                host.default_output_device()
            }
        })
        .context(if input {
            "No microphone is available."
        } else {
            "No audio output is available."
        })
}

pub struct InputStream {
    _stop: sync::Sender<()>,
}

impl InputStream {
    pub fn open(name: String, gain: f64, frames: mpsc::Sender<Vec<i16>>) -> Result<Self> {
        let (stop, stopped) = sync::channel();
        let (ready, started) = sync::sync_channel(1);
        thread::Builder::new()
            .name("jarvis-microphone".into())
            .spawn(move || match input_stream(&name, gain, frames) {
                Ok(stream) => {
                    let _ = ready.send(Ok(()));
                    let _ = stopped.recv();
                    drop(stream);
                }
                Err(error) => {
                    let _ = ready.send(Err(error));
                }
            })?;
        started.recv()??;
        Ok(Self { _stop: stop })
    }
}

fn input_stream(name: &str, gain: f64, frames: mpsc::Sender<Vec<i16>>) -> Result<cpal::Stream> {
    let device = device(name, true)?;
    let supported = device.default_input_config()?;
    let config = supported.config();
    fn build<T>(
        device: &Device,
        config: &cpal::StreamConfig,
        frames: mpsc::Sender<Vec<i16>>,
        gain: f64,
    ) -> Result<cpal::Stream>
    where
        T: SizedSample,
        f32: FromSample<T>,
    {
        let channels = usize::from(config.channels);
        let mut converter = Resampler::new(config.sample_rate.0, INPUT_RATE);
        let mut frame = Vec::with_capacity(FRAME_SAMPLES);
        Ok(device.build_input_stream(
            config,
            move |samples: &[T], _| {
                for group in samples.chunks_exact(channels) {
                    let mono = group
                        .iter()
                        .map(|value| value.to_sample::<f32>())
                        .max_by(|a, b| a.abs().total_cmp(&b.abs()))
                        .unwrap_or(0.0);
                    converter.push((mono * (gain.clamp(0.0, 500.0) / 100.0) as f32).clamp(-1.0, 1.0), |value| {
                        frame.push((value.clamp(-1.0, 1.0) * 32767.0) as i16);
                        if frame.len() == FRAME_SAMPLES {
                            let full =
                                std::mem::replace(&mut frame, Vec::with_capacity(FRAME_SAMPLES));
                            let _ = frames.try_send(full);
                        }
                    });
                }
            },
            |error| tracing::warn!(%error, "Microphone stream error"),
            None,
        )?)
    }
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, frames, gain),
        SampleFormat::I16 => build::<i16>(&device, &config, frames, gain),
        SampleFormat::U16 => build::<u16>(&device, &config, frames, gain),
        format => Err(anyhow!("Unsupported microphone sample format {format:?}")),
    }?;
    stream.play()?;
    Ok(stream)
}

pub struct OutputStream {
    _stop: sync::Sender<()>,
    buffer: Arc<Mutex<VecDeque<f32>>>,
    pub rate: u32,
}

impl OutputStream {
    pub fn open(name: String, meter: mpsc::Sender<f32>) -> Result<Self> {
        let (stop, stopped) = sync::channel();
        let (ready, started) = sync::sync_channel(1);
        let buffer = Arc::new(Mutex::new(VecDeque::new()));
        let target = buffer.clone();
        thread::Builder::new()
            .name("jarvis-speaker".into())
            .spawn(move || match output_stream(&name, target, meter) {
                Ok((stream, rate)) => {
                    let _ = ready.send(Ok(rate));
                    let _ = stopped.recv();
                    drop(stream);
                }
                Err(error) => {
                    let _ = ready.send(Err(error));
                }
            })?;
        Ok(Self {
            _stop: stop,
            buffer,
            rate: started.recv()??,
        })
    }

    pub fn push(&self, samples: &[f32]) {
        self.buffer.lock().extend(samples.iter().copied());
    }

    pub fn queued(&self) -> usize {
        self.buffer.lock().len()
    }

    pub fn clear(&self) {
        self.buffer.lock().clear();
    }
}

fn output_stream(
    name: &str,
    samples: Arc<Mutex<VecDeque<f32>>>,
    meter: mpsc::Sender<f32>,
) -> Result<(cpal::Stream, u32)> {
    let device = device(name, false)?;
    let supported = device.default_output_config()?;
    let config = supported.config();
    fn build<T>(
        device: &Device,
        config: &cpal::StreamConfig,
        samples: Arc<Mutex<VecDeque<f32>>>,
        meter: mpsc::Sender<f32>,
    ) -> Result<cpal::Stream>
    where
        T: SizedSample + FromSample<f32>,
    {
        let channels = usize::from(config.channels);
        let sample_count = config.sample_rate.0 as usize / 25;
        let mut energy = 0.0;
        let mut count = 0;
        let mut previous_level = 0.0;
        Ok(device.build_output_stream(
            config,
            move |output: &mut [T], _| {
                let mut buffer = samples.lock();
                for frame in output.chunks_exact_mut(channels) {
                    let value = buffer.pop_front().unwrap_or(0.0);
                    frame.fill(T::from_sample(value));
                    energy += value * value;
                    count += 1;
                    if count >= sample_count {
                        let level = ((energy / count as f32).sqrt() * 3.0).min(1.0);
                        if level > 0.0 || previous_level > 0.0 {
                            let _ = meter.try_send(level);
                        }
                        previous_level = level;
                        count = 0;
                        energy = 0.0;
                    }
                }
            },
            |error| tracing::warn!(%error, "Speaker stream error"),
            None,
        )?)
    }
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, samples, meter),
        SampleFormat::I16 => build::<i16>(&device, &config, samples, meter),
        SampleFormat::U16 => build::<u16>(&device, &config, samples, meter),
        format => Err(anyhow!("Unsupported output sample format {format:?}")),
    }?;
    stream.play()?;
    Ok((stream, config.sample_rate.0))
}

pub fn chime(rate: u32, tones: &[f32], seconds: f32) -> Vec<f32> {
    let note_length = (rate as f32 * seconds / tones.len() as f32) as usize;
    tones
        .iter()
        .flat_map(|tone| {
            (0..note_length).map(move |sample| {
                let time = sample as f32 / rate as f32;
                let progress = sample as f32 / note_length as f32;
                let envelope = (progress * 12.0).min(1.0) * (1.0 - progress).powi(2);
                (time * tone * std::f32::consts::TAU).sin() * envelope * 0.18
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::Resampler;

    #[test]
    fn resampling_spans_callback_boundaries() {
        let mut converter = Resampler::new(48_000, 16_000);
        let mut output = Vec::new();
        for chunk in [[0.0, 1.0], [2.0, 3.0], [4.0, 5.0], [6.0, 7.0]] {
            for sample in chunk {
                converter.push(sample, |value| output.push(value));
            }
        }
        assert_eq!(output, [0.0, 3.0, 6.0]);
    }
}
