//! Butler thread for asynchronous disk I/O operations.

use super::prefetch::{CaptureBufferConsumer, RegionBufferProducer};
use super::request::{ButlerCommand, ButlerState, CaptureId, RegionId};
use super::stream_state::ChannelStreamState;
use crossbeam_channel::{bounded, Receiver, Sender};
use dashmap::DashMap;
use hound::{SampleFormat, WavSpec, WavWriter};
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tutti_core::Wave;

/// Butler thread for asynchronous disk I/O.
pub struct ButlerThread {
    command_tx: Sender<ButlerCommand>,
    command_rx: Option<Receiver<ButlerCommand>>,
    stream_states: Arc<DashMap<usize, ChannelStreamState>>,
    sample_cache: Arc<DashMap<PathBuf, Arc<Wave>>>,
    thread_handle: Option<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
    state: Arc<AtomicU8>,
    chunk_size: usize,
    flush_threshold: usize,
    sample_rate: f64,
}

/// State for a capture consumer.
pub struct CaptureConsumerState {
    pub consumer: CaptureBufferConsumer,
    pub writer: Option<WavWriter<BufWriter<File>>>,
    pub channels: usize,
}

impl ButlerThread {
    pub fn new(
        sample_cache: Arc<DashMap<PathBuf, Arc<Wave>>>,
        channel_capacity: usize,
        sample_rate: f64,
    ) -> Self {
        let (tx, rx) = bounded(channel_capacity);

        Self {
            command_tx: tx,
            command_rx: Some(rx),
            stream_states: Arc::new(DashMap::new()),
            sample_cache,
            thread_handle: None,
            shutdown: Arc::new(AtomicBool::new(false)),
            state: Arc::new(AtomicU8::new(ButlerState::Running as u8)),
            chunk_size: 16384,
            flush_threshold: 8192,
            sample_rate,
        }
    }

    pub fn command_sender(&self) -> Sender<ButlerCommand> {
        self.command_tx.clone()
    }

    pub fn start(&mut self) {
        if self.thread_handle.is_some() {
            return;
        }

        let rx = self.command_rx.take().expect("command_rx already taken");
        let stream_states = Arc::clone(&self.stream_states);
        let sample_cache = Arc::clone(&self.sample_cache);
        let shutdown = Arc::clone(&self.shutdown);
        let state = Arc::clone(&self.state);
        let chunk_size = self.chunk_size;
        let flush_threshold = self.flush_threshold;
        let sample_rate = self.sample_rate;

        let handle = thread::Builder::new()
            .name("dawai-butler".into())
            .spawn(move || {
                butler_loop(
                    rx,
                    stream_states,
                    sample_cache,
                    shutdown,
                    state,
                    chunk_size,
                    flush_threshold,
                    sample_rate,
                );
            })
            .expect("Failed to spawn butler thread");

        self.thread_handle = Some(handle);
    }

    pub fn stop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = self.command_tx.send(ButlerCommand::Shutdown);

        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ButlerThread {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Butler thread main loop for disk I/O operations.
#[allow(clippy::too_many_arguments)]
fn butler_loop(
    rx: Receiver<ButlerCommand>,
    stream_states: Arc<DashMap<usize, ChannelStreamState>>,
    sample_cache: Arc<DashMap<PathBuf, Arc<Wave>>>,
    shutdown: Arc<AtomicBool>,
    state: Arc<AtomicU8>,
    chunk_size: usize,
    flush_threshold: usize,
    sample_rate: f64,
) {
    // Butler thread owns all producers and consumers (not shared!)
    let mut producers: std::collections::HashMap<RegionId, RegionBufferProducer> =
        std::collections::HashMap::new();
    let mut capture_consumers: std::collections::HashMap<CaptureId, CaptureConsumerState> =
        std::collections::HashMap::new();

    // Pre-allocate interleaving buffer to avoid allocation in hot path
    let mut interleave_buffer: Vec<(f32, f32)> = Vec::with_capacity(chunk_size);

    let mut is_paused = false;
    loop {
        if shutdown.load(Ordering::SeqCst) {
            // Flush all capture buffers before shutdown
            flush_all_captures(&mut capture_consumers, flush_threshold, true);
            break;
        }

        // Process incoming commands (non-blocking batch)
        loop {
            match rx.try_recv() {
                Ok(cmd) => match cmd {
                    // === Transport Control ===
                    ButlerCommand::Run => {
                        is_paused = false;
                        state.store(ButlerState::Running as u8, Ordering::SeqCst);
                    }
                    ButlerCommand::Pause => {
                        is_paused = true;
                        state.store(ButlerState::Paused as u8, Ordering::SeqCst);
                    }
                    ButlerCommand::WaitForCompletion => {
                        // Flush all capture buffers before continuing
                        flush_all_captures(&mut capture_consumers, flush_threshold, true);
                    }

                    // === Region Management ===
                    ButlerCommand::RegisterProducer {
                        region_id,
                        producer,
                    } => {
                        producers.insert(region_id, producer);
                    }
                    ButlerCommand::RemoveRegion(region_id) => {
                        producers.remove(&region_id);
                    }
                    ButlerCommand::SeekRegion {
                        region_id,
                        sample_offset,
                    } => {
                        // Update file position atomically
                        // The audio callback will detect the discontinuity and flush its consumer buffer
                        // The butler will start refilling from the new position on next refill cycle
                        if let Some(producer) = producers.get(&region_id) {
                            producer.set_file_position(sample_offset as u64);
                        }
                    }

                    // File streaming with ring buffers
                    ButlerCommand::StreamAudioFile {
                        channel_index,
                        file_path,
                        start_sample: _start_sample,
                        duration_samples: _duration_samples,
                        offset_samples,
                        speed,
                        gain,
                    } => {
                        use super::prefetch::RegionBuffer;
                        use parking_lot::Mutex;

                        // Load Wave file to get metadata
                        let wave = match Wave::load(&file_path) {
                            Ok(w) => Arc::new(w),
                            Err(_) => {
                                continue;
                            }
                        };

                        let file_length = wave.len() as u64;
                        let file_sample_rate = wave.sample_rate();
                        let channels = wave.channels();

                        // Calculate optimal buffer size based on file size
                        let buffer_capacity = calculate_buffer_size(file_length, sample_rate);
                        let region_id = RegionId::generate();

                        let (producer, consumer) = RegionBuffer::with_capacity(
                            region_id,
                            file_path.clone(),
                            file_length,
                            file_sample_rate,
                            channels,
                            buffer_capacity,
                        );

                        // Set initial file position
                        producer.set_file_position(offset_samples as u64);

                        // Pre-fill buffer with initial chunk
                        // (Butler will continue refilling in main loop)
                        cache_file_in_sample_cache(&sample_cache, file_path.clone(), wave);

                        // Register producer with Butler
                        producers.insert(region_id, producer);

                        // Ensure stream state exists for this track
                        stream_states
                            .entry(channel_index)
                            .or_insert_with(|| ChannelStreamState::new(channel_index, sample_rate));

                        // Start streaming with ring buffer consumer
                        if let Some(mut stream_state) = stream_states.get_mut(&channel_index) {
                            stream_state.start_streaming(
                                file_path.clone(),
                                Arc::new(Mutex::new(consumer)),
                                speed,
                                gain,
                            );
                        }
                    }
                    ButlerCommand::StopStreaming { channel_index } => {
                        if let Some(mut stream_state) = stream_states.get_mut(&channel_index) {
                            stream_state.stop_streaming();
                        }
                    }
                    ButlerCommand::SetPlaybackPosition {
                        channel_index: _channel_index,
                        position_seconds: _position_seconds,
                    } => {
                        // Position management handled externally");
                    }

                    // === Capture (Recording Write-Behind) ===
                    ButlerCommand::RegisterCapture {
                        capture_id,
                        consumer,
                        file_path,
                        sample_rate,
                        channels,
                    } => {
                        let writer = create_wav_writer(&file_path, sample_rate, channels);
                        capture_consumers.insert(
                            capture_id,
                            CaptureConsumerState {
                                consumer,
                                writer,
                                channels,
                            },
                        );
                    }
                    ButlerCommand::RemoveCapture(capture_id) => {
                        // Flush remaining data and finalize
                        if let Some(mut state) = capture_consumers.remove(&capture_id) {
                            flush_capture(&mut state, usize::MAX);
                            if let Some(writer) = state.writer.take() {
                                if writer.finalize().is_err() {}
                            }
                        }
                    }
                    ButlerCommand::Flush(req) => {
                        if let Some(state) = capture_consumers.get_mut(&req.capture_id) {
                            flush_capture(state, usize::MAX);
                        }
                    }
                    ButlerCommand::FlushAll => {
                        flush_all_captures(&mut capture_consumers, flush_threshold, true);
                    }

                    // === Lifecycle ===
                    ButlerCommand::Shutdown => {
                        flush_all_captures(&mut capture_consumers, flush_threshold, true);
                        return;
                    }
                },
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    flush_all_captures(&mut capture_consumers, flush_threshold, true);
                    return;
                }
            }
        }

        // Skip work if paused (but still process commands above)
        if is_paused {
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        // Skip work if idle (no streams and no captures)
        if stream_states.is_empty() && capture_consumers.is_empty() {
            thread::sleep(Duration::from_millis(1));
            continue;
        }

        // Ring buffer refill: check loops first
        check_and_handle_loops(&stream_states, &mut producers);

        // Refill ring buffers from disk
        refill_all_streams(
            &stream_states,
            &mut producers,
            &sample_cache,
            chunk_size,
            &mut interleave_buffer,
        );

        // Flush capture buffers that have enough data
        flush_all_captures(&mut capture_consumers, flush_threshold, false);
    }
}

/// Check and handle stream loop conditions.
fn check_and_handle_loops(
    stream_states: &DashMap<usize, ChannelStreamState>,
    producers: &mut std::collections::HashMap<RegionId, RegionBufferProducer>,
) {
    for stream_entry in stream_states.iter() {
        let stream_state = stream_entry.value();

        // Check if this stream needs to loop
        if let Some(loop_start) = stream_state.check_loop_condition() {
            // Get region ID for this stream
            if let Some(region_id) = stream_state.region_id() {
                if let Some(producer) = producers.get_mut(&region_id) {
                    // Flush buffer and seek to loop start
                    stream_state.flush_buffer();
                    producer.set_file_position(loop_start);
                }
            }
        }
    }
}

/// Refill ring buffers from disk with adaptive prefetch.
///
/// Uses a pre-allocated buffer to avoid allocation in the hot path.
fn refill_all_streams(
    _stream_states: &DashMap<usize, ChannelStreamState>,
    producers: &mut std::collections::HashMap<RegionId, RegionBufferProducer>,
    sample_cache: &DashMap<PathBuf, Arc<Wave>>,
    base_chunk_size: usize,
    interleave_buffer: &mut Vec<(f32, f32)>,
) {
    // Iterate over all region buffer producers
    for (_region_id, producer) in producers.iter_mut() {
        let available = producer.capacity() - producer.write_space();
        let buffer_capacity = producer.capacity();

        // Calculate buffer fill percentage
        let fill_percentage = (available as f32 / buffer_capacity as f32) * 100.0;

        // Adaptive refill strategy based on buffer state
        let (should_refill, chunk_multiplier) = if fill_percentage < 10.0 {
            // Critical: Buffer nearly empty - aggressive refill with double chunk size
            (true, 2.0)
        } else if fill_percentage < 25.0 {
            // Low: Buffer running low - normal refill
            (true, 1.0)
        } else if fill_percentage < 75.0 {
            // Medium: Buffer has decent amount - use smaller chunks to reduce I/O
            (true, 0.5)
        } else {
            // High: Buffer is full - skip refill
            (false, 1.0)
        };

        if !should_refill {
            continue;
        }

        // Adaptive chunk size
        let chunk_size = (base_chunk_size as f32 * chunk_multiplier) as usize;

        // Load Wave file from cache (or disk if not cached)
        let file_path = producer.file_path();
        let wave = match sample_cache.get(file_path) {
            Some(cached) => cached.value().clone(),
            None => {
                // Not in cache - load from disk and cache it
                match Wave::load(file_path) {
                    Ok(w) => {
                        let arc_wave = Arc::new(w);
                        sample_cache.insert(file_path.clone(), arc_wave.clone());
                        arc_wave
                    }
                    Err(_) => {
                        continue;
                    }
                }
            }
        };

        // Get current file position
        let file_position = producer.file_position() as usize;
        let channels = wave.channels();

        // Read chunk from Wave file
        interleave_buffer.clear();
        (0..chunk_size).for_each(|i| {
            let sample_idx = file_position + i;

            if sample_idx >= wave.len() {
                // Reached end of file - pad with silence
                interleave_buffer.push((0.0, 0.0));
            } else {
                let left = wave.at(0, sample_idx);
                let right = if channels > 1 {
                    wave.at(1, sample_idx)
                } else {
                    left // Mono -> duplicate
                };
                interleave_buffer.push((left, right));
            }
        });

        // Write samples to ring buffer producer
        let written = producer.write(interleave_buffer);

        // Update file position
        producer.set_file_position((file_position + written) as u64);

        if written < chunk_size {}
    }
}

/// Helper: Cache Wave file in sample cache (avoids reloading from disk)
fn cache_file_in_sample_cache(
    sample_cache: &DashMap<PathBuf, Arc<Wave>>,
    file_path: PathBuf,
    wave: Arc<Wave>,
) {
    sample_cache.insert(file_path, wave);
}

/// Calculate optimal buffer size based on file size.
fn calculate_buffer_size(file_length_samples: u64, sample_rate: f64) -> usize {
    // Convert file size to megabytes (stereo, 32-bit float)
    let file_size_bytes = file_length_samples * 2 * 4; // 2 channels * 4 bytes per sample
    let file_size_mb = file_size_bytes as f64 / (1024.0 * 1024.0);

    let buffer_seconds = if file_size_mb < 50.0 {
        // Small file - load entire file
        (file_length_samples as f64 / sample_rate).min(30.0) // Cap at 30 seconds
    } else if file_size_mb < 200.0 {
        // Medium file - 10 second buffer
        10.0
    } else if file_size_mb < 500.0 {
        // Large file - 5 second buffer
        5.0
    } else {
        // Huge file - 3 second buffer (minimum for smooth streaming)
        3.0
    };

    let buffer_capacity = (buffer_seconds * sample_rate) as usize;
    buffer_capacity.max(4096) // Minimum 4096 samples
}

/// Create a WAV writer for capture
fn create_wav_writer(
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

/// Flush a single capture buffer to disk
fn flush_capture(state: &mut CaptureConsumerState, max_samples: usize) {
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

    // Write interleaved samples to WAV
    for &(left, right) in &buffer[..read] {
        if writer.write_sample(left).is_err() {
            return;
        }
        if state.channels > 1 && writer.write_sample(right).is_err() {
            return;
        }
    }

    state.consumer.add_frames_written(read as u64);
}

/// Flush all capture buffers
fn flush_all_captures(
    capture_consumers: &mut std::collections::HashMap<CaptureId, CaptureConsumerState>,
    threshold: usize,
    force: bool,
) {
    for state in capture_consumers.values_mut() {
        let available = state.consumer.available();

        // Only flush if above threshold (or force flush)
        if force || available >= threshold {
            flush_capture(state, if force { usize::MAX } else { threshold });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: TempoMapSnapshot is not publicly exported from tutti-core,
    // so butler tests that need tempo map integration should be in integration tests.

    #[test]
    fn test_region_id_generation() {
        let id1 = RegionId::generate();
        let id2 = RegionId::generate();
        let id3 = RegionId::generate();

        // Each ID should be unique
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_capture_id_generation() {
        let id1 = CaptureId::generate();
        let id2 = CaptureId::generate();
        let id3 = CaptureId::generate();

        // Each ID should be unique
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
    }
}
