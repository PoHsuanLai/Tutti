//! CPAL audio output wrapper (requires std).

#[allow(unused_extern_crates)]
extern crate std;

use crate::callback::AudioCallbackState;
use crate::compat::{String, Vec};
use crate::{Error, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Wrapper to hold `cpal::Stream` in a `Send` context.
///
/// # Safety
/// `cpal::Stream` is `!Send` due to platform internals. This is safe because
/// `AudioEngine` is only accessed behind a `Mutex` in `TuttiSystem`.
struct StreamHandle(#[allow(dead_code)] cpal::Stream);

unsafe impl Send for StreamHandle {}

pub(crate) struct AudioEngine {
    sample_rate: f64,
    channels: usize,
    is_running: bool,
    device_index: Option<usize>,
    _stream: Option<StreamHandle>,
}

impl AudioEngine {
    pub(crate) fn new(device_index: Option<usize>) -> Result<Self> {
        let device = get_device(device_index)?;
        let config = device.default_output_config()?;

        Ok(Self {
            sample_rate: config.sample_rate().0 as f64,
            channels: config.channels() as usize,
            is_running: false,
            device_index,
            _stream: None,
        })
    }

    pub(crate) fn start(&mut self, state: AudioCallbackState) -> Result<()> {
        if self.is_running {
            return Ok(());
        }

        let device = get_device(self.device_index)?;
        let config = device.default_output_config()?;

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => build_stream::<f32>(&device, &config.into(), state)?,
            cpal::SampleFormat::I16 => build_stream::<i16>(&device, &config.into(), state)?,
            cpal::SampleFormat::U16 => build_stream::<u16>(&device, &config.into(), state)?,
            format => {
                return Err(Error::InvalidConfig(format!(
                    "Unsupported sample format: {format:?}"
                )));
            }
        };

        stream.play()?;
        self._stream = Some(StreamHandle(stream));
        self.is_running = true;

        Ok(())
    }

    pub(crate) fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub(crate) fn channels(&self) -> usize {
        self.channels
    }

    pub(crate) fn is_running(&self) -> bool {
        self.is_running
    }

    pub(crate) fn set_device(&mut self, index: Option<usize>) {
        self.device_index = index;
    }

    pub(crate) fn device_name(&self) -> Result<String> {
        Ok(get_device(self.device_index)?.name()?)
    }

    pub(crate) fn list_devices() -> Result<Vec<String>> {
        cpal::default_host()
            .output_devices()?
            .enumerate()
            .map(|(i, d)| Ok(format!("{i}: {}", d.name()?)))
            .collect()
    }
}

fn get_device(index: Option<usize>) -> Result<cpal::Device> {
    let host = cpal::default_host();

    match index {
        Some(i) => {
            let devices: Vec<_> = host.output_devices()?.collect();
            let count = devices.len();
            devices.into_iter().nth(i).ok_or_else(|| {
                Error::InvalidDevice(format!("Device index {i} out of range ({count} available)"))
            })
        }
        None => host
            .default_output_device()
            .ok_or_else(|| Error::InvalidDevice("No output device available".into())),
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    state: AudioCallbackState,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = config.channels as usize;

    // Pre-allocated buffers (grow on first callback, then stable)
    let mut output_f32 = Vec::<f32>::new();
    let mut lufs_left = Vec::<f32>::new();
    let mut lufs_right = Vec::<f32>::new();

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let frames = data.len() / channels;

                process_dsp(&state, frames, &mut output_f32);
                update_lufs_metering(&state, frames, &output_f32, &mut lufs_left, &mut lufs_right);
                write_output(data, channels, &output_f32);
            }));

            if result.is_err() {
                output_silence(data);
            }
        },
        |_err| {},
        None,
    )?;

    Ok(stream)
}

/// Process DSP graph into stereo f32 buffer.
#[inline]
fn process_dsp(state: &AudioCallbackState, frames: usize, output: &mut Vec<f32>) {
    let needed = frames * 2;
    if output.len() < needed {
        output.resize(needed, 0.0);
    } else {
        output[..needed].fill(0.0);
    }
    crate::callback::process_audio(state, &mut output[..needed]);
}

/// Update LUFS metering (non-blocking, skips if locked).
#[inline]
fn update_lufs_metering(
    state: &AudioCallbackState,
    frames: usize,
    output: &[f32],
    lufs_left: &mut Vec<f32>,
    lufs_right: &mut Vec<f32>,
) {
    if !state.metering.is_lufs_enabled() {
        return;
    }

    if lufs_left.len() < frames {
        lufs_left.resize(frames, 0.0);
        lufs_right.resize(frames, 0.0);
    }

    for i in 0..frames {
        lufs_left[i] = output[i * 2];
        lufs_right[i] = output[i * 2 + 1];
    }

    if let Some(mut ebur128) = state.metering.ebur128().try_lock() {
        let _ = ebur128.add_frames_planar_f32(&[&lufs_left[..frames], &lufs_right[..frames]]);
    }
}

/// Convert stereo f32 to output format and write to device buffer.
#[inline]
fn write_output<T: cpal::SizedSample + cpal::FromSample<f32>>(
    data: &mut [T],
    channels: usize,
    output: &[f32],
) {
    for (i, sample) in data.iter_mut().enumerate() {
        let frame = i / channels;
        let ch = i % channels;
        let value = if ch < 2 { output[frame * 2 + ch] } else { 0.0 };
        *sample = T::from_sample(value);
    }
}

/// Output silence (panic recovery).
#[inline]
fn output_silence<T: cpal::SizedSample + cpal::FromSample<f32>>(data: &mut [T]) {
    for sample in data.iter_mut() {
        *sample = T::from_sample(0.0);
    }
}
