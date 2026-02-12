//! Butler thread for asynchronous disk I/O operations.

use super::cache::LruCache;
use super::capture::{create_wav_writer, flush_all_captures, flush_capture, CaptureConsumerState};
use super::config::BufferConfig;
use super::loops::{
    calculate_buffer_size, capture_fadein_samples, capture_fadeout_samples, capture_samples,
    check_and_handle_loops,
};
use super::metrics::IOMetrics;
use super::pdc::check_pdc_updates;
use super::prefetch::RegionBufferProducer;
use super::refill::{refill_all_streams, refill_all_streams_parallel};
use super::request::{ButlerCommand, ButlerState, CaptureId, RegionId};
use super::stream_state::ChannelStreamState;
use super::varispeed::Varispeed;
use crossbeam_channel::{bounded, Receiver, Sender};
use dashmap::DashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use thread_priority::ThreadPriority;
use tutti_core::PdcManager;

/// Butler thread for asynchronous disk I/O.
pub struct ButlerThread {
    command_tx: Sender<ButlerCommand>,
    command_rx: Option<Receiver<ButlerCommand>>,
    stream_states: Arc<DashMap<usize, ChannelStreamState>>,
    sample_cache: Arc<LruCache>,
    metrics: Arc<IOMetrics>,
    thread_handle: Option<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
    state: Arc<AtomicU8>,
    config: BufferConfig,
    sample_rate: f64,
    pdc_manager: Option<Arc<PdcManager>>,
}

impl ButlerThread {
    /// Create a new butler thread with default configuration.
    #[allow(dead_code)]
    pub fn new(channel_capacity: usize, sample_rate: f64) -> Self {
        Self::with_config(channel_capacity, sample_rate, BufferConfig::default())
    }

    /// Create a new butler thread with custom configuration.
    pub fn with_config(channel_capacity: usize, sample_rate: f64, config: BufferConfig) -> Self {
        let (tx, rx) = bounded(channel_capacity);
        let sample_cache = Arc::new(LruCache::new(
            config.cache_max_entries,
            config.cache_max_bytes,
        ));

        Self {
            command_tx: tx,
            command_rx: Some(rx),
            stream_states: Arc::new(DashMap::new()),
            sample_cache,
            metrics: Arc::new(IOMetrics::new()),
            thread_handle: None,
            shutdown: Arc::new(AtomicBool::new(false)),
            state: Arc::new(AtomicU8::new(ButlerState::Running as u8)),
            config,
            sample_rate,
            pdc_manager: None,
        }
    }

    /// Set PDC manager for automatic delay compensation.
    ///
    /// When set, the butler will automatically apply PDC preroll to streams
    /// based on channel latency compensation values.
    pub fn with_pdc(mut self, pdc: Arc<PdcManager>) -> Self {
        self.pdc_manager = Some(pdc);
        self
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
        let metrics = Arc::clone(&self.metrics);
        let shutdown = Arc::clone(&self.shutdown);
        let state = Arc::clone(&self.state);
        let config = self.config;
        let sample_rate = self.sample_rate;
        let pdc_manager = self.pdc_manager.clone();

        let handle = thread::Builder::new()
            .name("dawai-butler".into())
            .spawn(move || {
                let _ = thread_priority::set_current_thread_priority(ThreadPriority::Max);

                butler_loop(
                    rx,
                    stream_states,
                    sample_cache,
                    metrics,
                    shutdown,
                    state,
                    config,
                    sample_rate,
                    pdc_manager,
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

    /// Get access to stream states for creating StreamingSamplerUnit instances.
    pub fn stream_states(&self) -> Arc<DashMap<usize, ChannelStreamState>> {
        Arc::clone(&self.stream_states)
    }

    /// Get I/O metrics.
    pub fn metrics(&self) -> Arc<IOMetrics> {
        Arc::clone(&self.metrics)
    }

    /// Get cache reference.
    pub fn cache(&self) -> Arc<LruCache> {
        Arc::clone(&self.sample_cache)
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
    sample_cache: Arc<LruCache>,
    metrics: Arc<IOMetrics>,
    shutdown: Arc<AtomicBool>,
    state: Arc<AtomicU8>,
    config: BufferConfig,
    sample_rate: f64,
    pdc_manager: Option<Arc<PdcManager>>,
) {
    let base_chunk_size = config.chunk_size;
    let flush_threshold = config.flush_threshold;
    let parallel_io = config.parallel_io;

    let mut producers: Vec<RegionBufferProducer> = Vec::new();
    let mut producer_index: std::collections::HashMap<RegionId, usize> =
        std::collections::HashMap::new();

    let mut capture_consumers: std::collections::HashMap<CaptureId, CaptureConsumerState> =
        std::collections::HashMap::new();

    let mut interleave_buffer: Vec<(f32, f32)> = Vec::with_capacity(base_chunk_size);

    let mut is_paused = false;
    let mut buffer_margin: f64 = 1.0;

    loop {
        if shutdown.load(Ordering::SeqCst) {
            flush_all_captures(&mut capture_consumers, &metrics, flush_threshold, true);
            break;
        }

        process_commands(
            &rx,
            &stream_states,
            &mut producers,
            &mut producer_index,
            &mut capture_consumers,
            &sample_cache,
            &metrics,
            &state,
            &config,
            sample_rate,
            &pdc_manager,
            &mut is_paused,
            &mut buffer_margin,
            flush_threshold,
        );

        if is_paused {
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        if stream_states.is_empty() && capture_consumers.is_empty() {
            thread::sleep(Duration::from_millis(1));
            continue;
        }

        check_pdc_updates(
            &pdc_manager,
            &stream_states,
            &mut producers,
            &producer_index,
            &sample_cache,
            &metrics,
            &config,
        );

        check_and_handle_loops(
            &stream_states,
            &mut producers,
            &producer_index,
            &sample_cache,
            &metrics,
        );

        if parallel_io && stream_states.len() >= 3 {
            refill_all_streams_parallel(
                &stream_states,
                &mut producers,
                &producer_index,
                &sample_cache,
                &metrics,
                base_chunk_size,
                buffer_margin,
            );
        } else {
            refill_all_streams(
                &stream_states,
                &mut producers,
                &producer_index,
                &sample_cache,
                &metrics,
                base_chunk_size,
                buffer_margin,
                &mut interleave_buffer,
            );
        }

        flush_all_captures(&mut capture_consumers, &metrics, flush_threshold, false);
    }
}

/// Process incoming commands from the command channel.
#[allow(clippy::too_many_arguments)]
fn process_commands(
    rx: &Receiver<ButlerCommand>,
    stream_states: &DashMap<usize, ChannelStreamState>,
    producers: &mut Vec<RegionBufferProducer>,
    producer_index: &mut std::collections::HashMap<RegionId, usize>,
    capture_consumers: &mut std::collections::HashMap<CaptureId, CaptureConsumerState>,
    sample_cache: &LruCache,
    metrics: &IOMetrics,
    state: &AtomicU8,
    config: &BufferConfig,
    sample_rate: f64,
    pdc_manager: &Option<Arc<PdcManager>>,
    is_paused: &mut bool,
    buffer_margin: &mut f64,
    flush_threshold: usize,
) {
    loop {
        match rx.try_recv() {
            Ok(cmd) => {
                handle_command(
                    cmd,
                    stream_states,
                    producers,
                    producer_index,
                    capture_consumers,
                    sample_cache,
                    metrics,
                    state,
                    config,
                    sample_rate,
                    pdc_manager,
                    is_paused,
                    buffer_margin,
                    flush_threshold,
                );
            }
            Err(crossbeam_channel::TryRecvError::Empty) => break,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                flush_all_captures(capture_consumers, metrics, flush_threshold, true);
                return;
            }
        }
    }
}

/// Handle a single butler command.
#[allow(clippy::too_many_arguments)]
fn handle_command(
    cmd: ButlerCommand,
    stream_states: &DashMap<usize, ChannelStreamState>,
    producers: &mut Vec<RegionBufferProducer>,
    producer_index: &mut std::collections::HashMap<RegionId, usize>,
    capture_consumers: &mut std::collections::HashMap<CaptureId, CaptureConsumerState>,
    sample_cache: &LruCache,
    metrics: &IOMetrics,
    state: &AtomicU8,
    config: &BufferConfig,
    sample_rate: f64,
    pdc_manager: &Option<Arc<PdcManager>>,
    is_paused: &mut bool,
    buffer_margin: &mut f64,
    flush_threshold: usize,
) {
    match cmd {
        ButlerCommand::Run => {
            *is_paused = false;
            state.store(ButlerState::Running as u8, Ordering::SeqCst);
        }
        ButlerCommand::Pause => {
            *is_paused = true;
            state.store(ButlerState::Paused as u8, Ordering::SeqCst);
        }
        ButlerCommand::WaitForCompletion => {
            flush_all_captures(capture_consumers, metrics, flush_threshold, true);
        }

        ButlerCommand::StreamAudioFile {
            channel_index,
            file_path,
            offset_samples,
        } => {
            handle_stream_audio_file(
                channel_index,
                file_path,
                offset_samples,
                stream_states,
                producers,
                producer_index,
                sample_cache,
                metrics,
                sample_rate,
                pdc_manager,
            );
        }
        ButlerCommand::StopStreaming { channel_index } => {
            if let Some(mut stream_state) = stream_states.get_mut(&channel_index) {
                stream_state.stop_streaming();
            }
        }

        ButlerCommand::SeekStream {
            channel_index,
            position_samples,
        } => {
            handle_seek_stream(
                channel_index,
                position_samples,
                stream_states,
                producers,
                producer_index,
                sample_cache,
                metrics,
                config,
            );
        }

        ButlerCommand::SetLoopRange {
            channel_index,
            start_samples,
            end_samples,
            crossfade_samples,
        } => {
            handle_set_loop_range(
                channel_index,
                start_samples,
                end_samples,
                crossfade_samples,
                stream_states,
                producers,
                producer_index,
                sample_cache,
            );
        }

        ButlerCommand::ClearLoopRange { channel_index } => {
            if let Some(mut stream_state) = stream_states.get_mut(&channel_index) {
                stream_state.clear_loop_range();
            }
        }

        ButlerCommand::SetVarispeed {
            channel_index,
            direction,
            speed,
        } => {
            if let Some(mut stream_state) = stream_states.get_mut(&channel_index) {
                stream_state.set_varispeed(Varispeed { direction, speed });
            }
        }

        ButlerCommand::UpdatePdcPreroll {
            channel_index,
            new_preroll,
        } => {
            handle_update_pdc_preroll(
                channel_index,
                new_preroll,
                stream_states,
                producers,
                producer_index,
                sample_cache,
                metrics,
                config,
            );
        }

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
            if let Some(mut state) = capture_consumers.remove(&capture_id) {
                flush_capture(&mut state, metrics, usize::MAX);
                if let Some(writer) = state.writer.take() {
                    let _ = writer.finalize();
                }
            }
        }
        ButlerCommand::Flush(req) => {
            if let Some(state) = capture_consumers.get_mut(&req.capture_id) {
                flush_capture(state, metrics, usize::MAX);
            }
        }
        ButlerCommand::FlushAll => {
            flush_all_captures(capture_consumers, metrics, flush_threshold, true);
        }

        ButlerCommand::SetBufferMargin { margin } => {
            *buffer_margin = margin.clamp(0.5, 3.0);
        }

        ButlerCommand::Shutdown => {
            flush_all_captures(capture_consumers, metrics, flush_threshold, true);
        }
    }
}

/// Handle StreamAudioFile command.
#[allow(clippy::too_many_arguments)]
fn handle_stream_audio_file(
    channel_index: usize,
    file_path: std::path::PathBuf,
    offset_samples: usize,
    stream_states: &DashMap<usize, ChannelStreamState>,
    producers: &mut Vec<RegionBufferProducer>,
    producer_index: &mut std::collections::HashMap<RegionId, usize>,
    sample_cache: &LruCache,
    metrics: &IOMetrics,
    sample_rate: f64,
    pdc_manager: &Option<Arc<PdcManager>>,
) {
    use super::prefetch::RegionBuffer;
    use parking_lot::Mutex;
    use tutti_core::Wave;

    let wave = if let Some(cached) = sample_cache.get(&file_path) {
        metrics.record_cache_hit();
        cached
    } else {
        metrics.record_cache_miss();
        match Wave::load(&file_path) {
            Ok(w) => {
                let arc_wave = Arc::new(w);
                let bytes = arc_wave.len() as u64 * arc_wave.channels() as u64 * 4;
                metrics.record_read(bytes);
                sample_cache.insert(file_path.clone(), arc_wave.clone());
                arc_wave
            }
            Err(_) => {
                return;
            }
        }
    };

    let file_length = wave.len() as u64;

    let buffer_capacity = calculate_buffer_size(file_length, sample_rate);
    let region_id = RegionId::generate();

    let (producer, consumer) =
        RegionBuffer::with_capacity(region_id, file_path.clone(), buffer_capacity);

    let pdc_preroll = pdc_manager
        .as_ref()
        .filter(|pdc| pdc.is_enabled())
        .map(|pdc| pdc.get_channel_compensation(channel_index) as u64)
        .unwrap_or(0);

    let adjusted_offset = (offset_samples as u64).saturating_sub(pdc_preroll);
    producer.set_file_position(adjusted_offset);

    let idx = producers.len();
    producers.push(producer);
    producer_index.insert(region_id, idx);

    stream_states.entry(channel_index).or_default();

    let file_sr = wave.sample_rate();
    let src_ratio = if (file_sr - sample_rate).abs() < 0.01 {
        1.0
    } else {
        (file_sr / sample_rate) as f32
    };

    if let Some(mut stream_state) = stream_states.get_mut(&channel_index) {
        stream_state.start_streaming(Arc::new(Mutex::new(consumer)));
        stream_state.set_pdc_preroll(pdc_preroll);
        stream_state.shared_state().set_src_ratio(src_ratio);
    }
}

/// Handle SeekStream command.
#[allow(clippy::too_many_arguments)]
fn handle_seek_stream(
    channel_index: usize,
    position_samples: u64,
    stream_states: &DashMap<usize, ChannelStreamState>,
    producers: &mut [RegionBufferProducer],
    producer_index: &std::collections::HashMap<RegionId, usize>,
    sample_cache: &LruCache,
    metrics: &IOMetrics,
    config: &BufferConfig,
) {
    if let Some(stream_state) = stream_states.get(&channel_index) {
        if let Some(region_id) = stream_state.region_id() {
            if let Some(&idx) = producer_index.get(&region_id) {
                let producer = &mut producers[idx];
                let crossfade_len = config.seek_crossfade_samples;

                let pdc_preroll = stream_state.pdc_preroll();
                let adjusted_position = position_samples.saturating_sub(pdc_preroll);

                let fadeout = capture_fadeout_samples(&stream_state, crossfade_len);

                stream_state.set_seeking(true);

                stream_state.flush_buffer();
                producer.set_file_position(adjusted_position);

                let fadein = capture_fadein_samples(
                    sample_cache,
                    metrics,
                    producer.file_path(),
                    adjusted_position,
                    crossfade_len,
                );

                if !fadeout.is_empty() && !fadein.is_empty() {
                    stream_state
                        .shared_state()
                        .start_seek_crossfade(fadeout, fadein);
                }

                stream_state.set_seeking(false);
            }
        }
    }
}

/// Handle SetLoopRange command.
#[allow(clippy::too_many_arguments)]
fn handle_set_loop_range(
    channel_index: usize,
    start_samples: u64,
    end_samples: u64,
    crossfade_samples: usize,
    stream_states: &DashMap<usize, ChannelStreamState>,
    producers: &mut [RegionBufferProducer],
    producer_index: &std::collections::HashMap<RegionId, usize>,
    sample_cache: &LruCache,
) {
    if let Some(mut stream_state) = stream_states.get_mut(&channel_index) {
        stream_state.set_loop_range(start_samples, end_samples, crossfade_samples);

        // When setting a loop range, we need to ensure the buffer contains only samples
        // within the loop region. The ring buffer may already contain samples past the
        // loop end, so we need to check the producer's position and flush if necessary.
        if let Some(region_id) = stream_state.region_id() {
            if let Some(&idx) = producer_index.get(&region_id) {
                let producer_pos = producers[idx].file_position();
                // If producer has written past loop end, flush and seek to loop start
                if producer_pos > end_samples {
                    stream_state.flush_buffer();
                    producers[idx].set_file_position(start_samples);
                }

                if crossfade_samples > 0 {
                    let file_path = producers[idx].file_path();
                    if let Some(wave) = sample_cache.get(file_path) {
                        let preloop =
                            capture_samples(&wave, start_samples as usize, crossfade_samples);
                        stream_state.set_preloop_buffer(preloop);
                    }
                }
            }
        }
    }
}

/// Handle UpdatePdcPreroll command.
#[allow(clippy::too_many_arguments)]
fn handle_update_pdc_preroll(
    channel_index: usize,
    new_preroll: u64,
    stream_states: &DashMap<usize, ChannelStreamState>,
    producers: &mut [RegionBufferProducer],
    producer_index: &std::collections::HashMap<RegionId, usize>,
    sample_cache: &LruCache,
    metrics: &IOMetrics,
    config: &BufferConfig,
) {
    if let Some(mut stream_state) = stream_states.get_mut(&channel_index) {
        let old_preroll = stream_state.pdc_preroll();

        if new_preroll != old_preroll {
            if let Some(region_id) = stream_state.region_id() {
                if let Some(&idx) = producer_index.get(&region_id) {
                    let producer = &mut producers[idx];
                    let current_pos = producer.file_position();
                    let new_pos = if new_preroll > old_preroll {
                        current_pos.saturating_sub(new_preroll - old_preroll)
                    } else {
                        current_pos + (old_preroll - new_preroll)
                    };

                    let crossfade_len = config.seek_crossfade_samples;
                    let fadeout = capture_fadeout_samples(&stream_state, crossfade_len);

                    stream_state.set_seeking(true);
                    stream_state.flush_buffer();
                    producer.set_file_position(new_pos);

                    let fadein = capture_fadein_samples(
                        sample_cache,
                        metrics,
                        producer.file_path(),
                        new_pos,
                        crossfade_len,
                    );

                    if !fadeout.is_empty() && !fadein.is_empty() {
                        stream_state
                            .shared_state()
                            .start_seek_crossfade(fadeout, fadein);
                    }

                    stream_state.set_pdc_preroll(new_preroll);
                    stream_state.set_seeking(false);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_region_id_generation() {
        let id1 = RegionId::generate();
        let id2 = RegionId::generate();
        let id3 = RegionId::generate();

        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_capture_id_generation() {
        let id1 = CaptureId::generate();
        let id2 = CaptureId::generate();
        let id3 = CaptureId::generate();
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
    }
}
