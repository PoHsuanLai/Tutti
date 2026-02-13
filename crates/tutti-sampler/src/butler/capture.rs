//! Audio capture (recording) functionality for butler thread.

use super::metrics::IOMetrics;
use super::prefetch::CaptureBufferConsumer;
use super::request::CaptureId;
use hound::{SampleFormat, WavSpec, WavWriter};
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

pub struct CaptureConsumerState {
    pub consumer: CaptureBufferConsumer,
    pub writer: Option<WavWriter<BufWriter<File>>>,
    pub channels: usize,
}

pub(super) fn create_wav_writer(
    file_path: &PathBuf,
    sample_rate: f64,
    channels: usize,
) -> Option<WavWriter<BufWriter<File>>> {
    let spec = WavSpec {
        channels: channels as u16,
        sample_rate: sample_rate as u32,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };

    match File::create(file_path) {
        Ok(file) => {
            let buf_writer = BufWriter::new(file);
            WavWriter::new(buf_writer, spec).ok()
        }
        Err(_) => None,
    }
}

pub(super) fn flush_capture(
    state: &mut CaptureConsumerState,
    metrics: &IOMetrics,
    max_samples: usize,
) {
    let Some(writer) = state.writer.as_mut() else {
        return;
    };

    let available = state.consumer.available();
    let to_read = available.min(max_samples);

    if to_read == 0 {
        return;
    }

    let mut buffer = vec![(0.0f32, 0.0f32); to_read];
    let read = state.consumer.read_into(&mut buffer);

    for &(left, right) in &buffer[..read] {
        if writer.write_sample(left).is_err() {
            return;
        }
        if state.channels > 1 && writer.write_sample(right).is_err() {
            return;
        }
    }

    let bytes_written = read as u64 * state.channels as u64 * 4;
    metrics.record_write(bytes_written);

    state.consumer.add_frames_written(read as u64);
}

pub(super) fn flush_all_captures(
    capture_consumers: &mut std::collections::HashMap<CaptureId, CaptureConsumerState>,
    metrics: &IOMetrics,
    threshold: usize,
    force: bool,
) {
    for state in capture_consumers.values_mut() {
        let available = state.consumer.available();

        if force || available >= threshold {
            flush_capture(state, metrics, if force { usize::MAX } else { threshold });
        }
    }
}
