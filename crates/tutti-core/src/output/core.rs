//! CPAL audio output wrapper.

use crate::callback::AudioCallbackState;
use crate::{Error, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

#[derive(Clone, Default)]
pub struct AudioEngineConfig {
    pub output_device_index: Option<usize>,
}

/// Wrapper to hold a `cpal::Stream` in a `Send` context.
///
/// `cpal::Stream` is `!Send` due to platform internals (`*mut ()`, non-Send closures).
/// This is safe because `AudioEngine` is only accessed behind a `Mutex` in `TuttiSystem`,
/// ensuring single-threaded access. The stream is never moved across threads directly —
/// it lives for the lifetime of the engine and is dropped when the engine stops.
struct StreamHandle(#[allow(dead_code)] cpal::Stream);

// SAFETY: The stream is only accessed behind a Mutex<AudioEngine> in TuttiSystem,
// so it's never concurrently accessed. It stays on the thread that created it
// until AudioEngine is dropped.
unsafe impl Send for StreamHandle {}

pub struct AudioEngine {
    sample_rate: f64,
    channels: usize,
    is_running: bool,
    output_device_index: Option<usize>,
    _stream: Option<StreamHandle>,
}

impl AudioEngine {
    pub fn new(config: AudioEngineConfig) -> Result<Self> {
        let device = Self::get_device(config.output_device_index)?;
        let output_config = device.default_output_config()?;

        Ok(Self {
            sample_rate: output_config.sample_rate().0 as f64,
            channels: output_config.channels() as usize,
            is_running: false,
            output_device_index: config.output_device_index,
            _stream: None,
        })
    }

    pub fn start(&mut self, state: AudioCallbackState) -> Result<()> {
        if self.is_running {
            return Ok(());
        }

        let device = Self::get_device(self.output_device_index)?;
        let config = device.default_output_config()?;

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => self.build_stream::<f32>(&device, &config.into(), state)?,
            cpal::SampleFormat::I16 => self.build_stream::<i16>(&device, &config.into(), state)?,
            cpal::SampleFormat::U16 => self.build_stream::<u16>(&device, &config.into(), state)?,
            format => {
                return Err(Error::InvalidConfig(format!(
                    "Unsupported sample format: {:?}",
                    format
                )));
            }
        };

        stream.play()?;

        self._stream = Some(StreamHandle(stream));
        self.is_running = true;

        Ok(())
    }

    fn get_device(index: Option<usize>) -> Result<cpal::Device> {
        let host = cpal::default_host();

        if let Some(idx) = index {
            let devices: Vec<_> = host.output_devices()?.collect();

            let device_count = devices.len();
            devices.into_iter().nth(idx).ok_or_else(|| {
                Error::InvalidDevice(format!(
                    "Output device index {} out of range (available: {})",
                    idx, device_count
                ))
            })
        } else {
            host.default_output_device()
                .ok_or_else(|| Error::InvalidDevice("No output device available".to_string()))
        }
    }

    fn build_stream<T>(
        &self,
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        state: AudioCallbackState,
    ) -> Result<cpal::Stream>
    where
        T: cpal::SizedSample + cpal::FromSample<f32>,
    {
        let channels = config.channels as usize;

        let stream = device.build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let frames = data.len() / channels;
                    let mut output_f32 = vec![0.0f32; frames * 2];

                    crate::callback::process_audio(&state, &mut output_f32);

                    for (i, sample) in data.iter_mut().enumerate() {
                        let channel = i % channels;
                        let frame = i / channels;
                        let value = if channel < 2 {
                            output_f32.get(frame * 2 + channel).copied().unwrap_or(0.0)
                        } else {
                            0.0
                        };
                        *sample = T::from_sample(value);
                    }
                }));

                if result.is_err() {
                    // Panic in callback - output silence
                    for sample in data.iter_mut() {
                        *sample = T::from_sample(0.0);
                    }
                }
            },
            |_err| {
                // Audio stream error - cannot log from callback
            },
            None,
        )?;

        Ok(stream)
    }

    pub fn set_output_device(&mut self, index: Option<usize>) {
        self.output_device_index = index;
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn is_running(&self) -> bool {
        self.is_running
    }

    /// List available output devices.
    pub fn list_output_devices() -> Result<Vec<String>> {
        let host = cpal::default_host();
        let devices: Result<Vec<String>> = host
            .output_devices()?
            .enumerate()
            .map(|(idx, device)| Ok(format!("{}: {}", idx, device.name()?)))
            .collect();
        devices
    }

    /// Get the name of the current output device.
    pub fn current_output_device_name(&self) -> Result<String> {
        let device = Self::get_device(self.output_device_index)?;
        Ok(device.name()?)
    }
}
