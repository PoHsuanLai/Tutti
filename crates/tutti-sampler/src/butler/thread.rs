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

/// Shared immutable resources used by butler thread functions.
///
/// Groups the Arc'd/shared references that are passed to nearly every
/// butler function. Extracted to avoid 8-13 parameter function signatures.
pub(super) struct ButlerResources {
    pub stream_states: Arc<DashMap<usize, ChannelStreamState>>,
    pub sample_cache: Arc<LruCache>,
    pub metrics: Arc<IOMetrics>,
    pub pdc_manager: Option<Arc<PdcManager>>,
    pub config: BufferConfig,
    pub sample_rate: f64,
}

/// Mutable state local to the butler thread.
pub(super) struct ButlerMutableState {
    pub producers: Vec<RegionBufferProducer>,
    pub producer_index: std::collections::HashMap<RegionId, usize>,
    pub capture_consumers: std::collections::HashMap<CaptureId, CaptureConsumerState>,
    pub is_paused: bool,
    pub buffer_margin: f64,
}

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
    #[allow(dead_code)]
    pub fn new(channel_capacity: usize, sample_rate: f64) -> Self {
        Self::with_config(channel_capacity, sample_rate, BufferConfig::default())
    }

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
        let shutdown = Arc::clone(&self.shutdown);
        let state = Arc::clone(&self.state);

        let res = ButlerResources {
            stream_states: Arc::clone(&self.stream_states),
            sample_cache: Arc::clone(&self.sample_cache),
            metrics: Arc::clone(&self.metrics),
            pdc_manager: self.pdc_manager.clone(),
            config: self.config,
            sample_rate: self.sample_rate,
        };

        let handle = thread::Builder::new()
            .name("dawai-butler".into())
            .spawn(move || {
                let _ = thread_priority::set_current_thread_priority(ThreadPriority::Max);

                butler_loop(rx, res, shutdown, state);
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

    pub fn metrics(&self) -> Arc<IOMetrics> {
        Arc::clone(&self.metrics)
    }

    pub fn cache(&self) -> Arc<LruCache> {
        Arc::clone(&self.sample_cache)
    }
}

impl Drop for ButlerThread {
    fn drop(&mut self) {
        self.stop();
    }
}

fn butler_loop(
    rx: Receiver<ButlerCommand>,
    res: ButlerResources,
    shutdown: Arc<AtomicBool>,
    state: Arc<AtomicU8>,
) {
    let base_chunk_size = res.config.chunk_size;
    let flush_threshold = res.config.flush_threshold;
    let parallel_io = res.config.parallel_io;

    let mut ms = ButlerMutableState {
        producers: Vec::new(),
        producer_index: std::collections::HashMap::new(),
        capture_consumers: std::collections::HashMap::new(),
        is_paused: false,
        buffer_margin: 1.0,
    };

    let mut interleave_buffer: Vec<(f32, f32)> = Vec::with_capacity(base_chunk_size);

    loop {
        if shutdown.load(Ordering::SeqCst) {
            flush_all_captures(
                &mut ms.capture_consumers,
                &res.metrics,
                flush_threshold,
                true,
            );
            break;
        }

        process_commands(&rx, &res, &mut ms, &state, flush_threshold);

        if ms.is_paused {
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        if res.stream_states.is_empty() && ms.capture_consumers.is_empty() {
            thread::sleep(Duration::from_millis(1));
            continue;
        }

        check_pdc_updates(
            &res.pdc_manager,
            &res.stream_states,
            &mut ms.producers,
            &ms.producer_index,
            &res.sample_cache,
            &res.metrics,
            &res.config,
        );

        check_and_handle_loops(
            &res.stream_states,
            &mut ms.producers,
            &ms.producer_index,
            &res.sample_cache,
            &res.metrics,
        );

        if parallel_io && res.stream_states.len() >= 3 {
            refill_all_streams_parallel(
                &res.stream_states,
                &mut ms.producers,
                &ms.producer_index,
                &res.sample_cache,
                &res.metrics,
                base_chunk_size,
                ms.buffer_margin,
            );
        } else {
            refill_all_streams(
                &res.stream_states,
                &mut ms.producers,
                &ms.producer_index,
                &res.sample_cache,
                &res.metrics,
                base_chunk_size,
                ms.buffer_margin,
                &mut interleave_buffer,
            );
        }

        flush_all_captures(
            &mut ms.capture_consumers,
            &res.metrics,
            flush_threshold,
            false,
        );
    }
}

fn process_commands(
    rx: &Receiver<ButlerCommand>,
    res: &ButlerResources,
    ms: &mut ButlerMutableState,
    state: &AtomicU8,
    flush_threshold: usize,
) {
    loop {
        match rx.try_recv() {
            Ok(cmd) => {
                handle_command(cmd, res, ms, state, flush_threshold);
            }
            Err(crossbeam_channel::TryRecvError::Empty) => break,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                flush_all_captures(
                    &mut ms.capture_consumers,
                    &res.metrics,
                    flush_threshold,
                    true,
                );
                return;
            }
        }
    }
}

fn handle_command(
    cmd: ButlerCommand,
    res: &ButlerResources,
    ms: &mut ButlerMutableState,
    state: &AtomicU8,
    flush_threshold: usize,
) {
    match cmd {
        ButlerCommand::Run => {
            ms.is_paused = false;
            state.store(ButlerState::Running as u8, Ordering::SeqCst);
        }
        ButlerCommand::Pause => {
            ms.is_paused = true;
            state.store(ButlerState::Paused as u8, Ordering::SeqCst);
        }
        ButlerCommand::WaitForCompletion => {
            flush_all_captures(
                &mut ms.capture_consumers,
                &res.metrics,
                flush_threshold,
                true,
            );
        }

        ButlerCommand::StreamAudioFile {
            channel_index,
            file_path,
            offset_samples,
        } => {
            handle_stream_audio_file(channel_index, file_path, offset_samples, res, ms);
        }
        ButlerCommand::StopStreaming { channel_index } => {
            if let Some(mut stream_state) = res.stream_states.get_mut(&channel_index) {
                stream_state.stop_streaming();
            }
        }

        ButlerCommand::SeekStream {
            channel_index,
            position_samples,
        } => {
            handle_seek_stream(channel_index, position_samples, res, ms);
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
                res,
                ms,
            );
        }

        ButlerCommand::ClearLoopRange { channel_index } => {
            if let Some(mut stream_state) = res.stream_states.get_mut(&channel_index) {
                stream_state.clear_loop_range();
            }
        }

        ButlerCommand::SetVarispeed {
            channel_index,
            direction,
            speed,
        } => {
            if let Some(mut stream_state) = res.stream_states.get_mut(&channel_index) {
                stream_state.set_varispeed(Varispeed { direction, speed });
            }
        }

        ButlerCommand::UpdatePdcPreroll {
            channel_index,
            new_preroll,
        } => {
            handle_update_pdc_preroll(channel_index, new_preroll, res, ms);
        }

        ButlerCommand::RegisterCapture {
            capture_id,
            consumer,
            file_path,
            sample_rate,
            channels,
        } => {
            let writer = create_wav_writer(&file_path, sample_rate, channels);
            ms.capture_consumers.insert(
                capture_id,
                CaptureConsumerState {
                    consumer,
                    writer,
                    channels,
                },
            );
        }
        ButlerCommand::RemoveCapture(capture_id) => {
            if let Some(mut cap_state) = ms.capture_consumers.remove(&capture_id) {
                flush_capture(&mut cap_state, &res.metrics, usize::MAX);
                if let Some(writer) = cap_state.writer.take() {
                    let _ = writer.finalize();
                }
            }
        }
        ButlerCommand::Flush(req) => {
            if let Some(cap_state) = ms.capture_consumers.get_mut(&req.capture_id) {
                flush_capture(cap_state, &res.metrics, usize::MAX);
            }
        }
        ButlerCommand::FlushAll => {
            flush_all_captures(
                &mut ms.capture_consumers,
                &res.metrics,
                flush_threshold,
                true,
            );
        }

        ButlerCommand::SetBufferMargin { margin } => {
            ms.buffer_margin = margin.clamp(0.5, 3.0);
        }

        ButlerCommand::Shutdown => {
            flush_all_captures(
                &mut ms.capture_consumers,
                &res.metrics,
                flush_threshold,
                true,
            );
        }
    }
}

fn handle_stream_audio_file(
    channel_index: usize,
    file_path: std::path::PathBuf,
    offset_samples: usize,
    res: &ButlerResources,
    ms: &mut ButlerMutableState,
) {
    use super::prefetch::RegionBuffer;
    use parking_lot::Mutex;
    use tutti_core::Wave;

    let wave = if let Some(cached) = res.sample_cache.get(&file_path) {
        res.metrics.record_cache_hit();
        cached
    } else {
        res.metrics.record_cache_miss();
        match Wave::load(&file_path) {
            Ok(w) => {
                let arc_wave = Arc::new(w);
                let bytes = arc_wave.len() as u64 * arc_wave.channels() as u64 * 4;
                res.metrics.record_read(bytes);
                res.sample_cache.insert(file_path.clone(), arc_wave.clone());
                arc_wave
            }
            Err(_) => {
                return;
            }
        }
    };

    let file_length = wave.len() as u64;

    let buffer_capacity = calculate_buffer_size(file_length, res.sample_rate);
    let region_id = RegionId::generate();

    let (producer, consumer) =
        RegionBuffer::with_capacity(region_id, file_path.clone(), buffer_capacity);

    let pdc_preroll = res
        .pdc_manager
        .as_ref()
        .filter(|pdc| pdc.is_enabled())
        .map(|pdc| pdc.get_channel_compensation(channel_index) as u64)
        .unwrap_or(0);

    let adjusted_offset = (offset_samples as u64).saturating_sub(pdc_preroll);
    producer.set_file_position(adjusted_offset);

    let idx = ms.producers.len();
    ms.producers.push(producer);
    ms.producer_index.insert(region_id, idx);

    res.stream_states.entry(channel_index).or_default();

    let file_sr = wave.sample_rate();
    let src_ratio = if (file_sr - res.sample_rate).abs() < 0.01 {
        1.0
    } else {
        (file_sr / res.sample_rate) as f32
    };

    if let Some(mut stream_state) = res.stream_states.get_mut(&channel_index) {
        stream_state.start_streaming(Arc::new(Mutex::new(consumer)));
        stream_state.set_pdc_preroll(pdc_preroll);
        stream_state.shared_state().set_src_ratio(src_ratio);
    }
}

fn handle_seek_stream(
    channel_index: usize,
    position_samples: u64,
    res: &ButlerResources,
    ms: &mut ButlerMutableState,
) {
    if let Some(stream_state) = res.stream_states.get(&channel_index) {
        if let Some(region_id) = stream_state.region_id() {
            if let Some(&idx) = ms.producer_index.get(&region_id) {
                let producer = &mut ms.producers[idx];
                let crossfade_len = res.config.seek_crossfade_samples;

                let pdc_preroll = stream_state.pdc_preroll();
                let adjusted_position = position_samples.saturating_sub(pdc_preroll);

                let fadeout = capture_fadeout_samples(&stream_state, crossfade_len);

                stream_state.set_seeking(true);

                stream_state.flush_buffer();
                producer.set_file_position(adjusted_position);

                let fadein = capture_fadein_samples(
                    &res.sample_cache,
                    &res.metrics,
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

fn handle_set_loop_range(
    channel_index: usize,
    start_samples: u64,
    end_samples: u64,
    crossfade_samples: usize,
    res: &ButlerResources,
    ms: &mut ButlerMutableState,
) {
    if let Some(mut stream_state) = res.stream_states.get_mut(&channel_index) {
        stream_state.set_loop_range(start_samples, end_samples, crossfade_samples);

        if let Some(region_id) = stream_state.region_id() {
            if let Some(&idx) = ms.producer_index.get(&region_id) {
                let producer_pos = ms.producers[idx].file_position();
                if producer_pos > end_samples {
                    stream_state.flush_buffer();
                    ms.producers[idx].set_file_position(start_samples);
                }

                if crossfade_samples > 0 {
                    let file_path = ms.producers[idx].file_path();
                    if let Some(wave) = res.sample_cache.get(file_path) {
                        let preloop =
                            capture_samples(&wave, start_samples as usize, crossfade_samples);
                        stream_state.set_preloop_buffer(preloop);
                    }
                }
            }
        }
    }
}

fn handle_update_pdc_preroll(
    channel_index: usize,
    new_preroll: u64,
    res: &ButlerResources,
    ms: &mut ButlerMutableState,
) {
    if let Some(mut stream_state) = res.stream_states.get_mut(&channel_index) {
        let old_preroll = stream_state.pdc_preroll();

        if new_preroll != old_preroll {
            if let Some(region_id) = stream_state.region_id() {
                if let Some(&idx) = ms.producer_index.get(&region_id) {
                    let producer = &mut ms.producers[idx];
                    let current_pos = producer.file_position();
                    let new_pos = if new_preroll > old_preroll {
                        current_pos.saturating_sub(new_preroll - old_preroll)
                    } else {
                        current_pos + (old_preroll - new_preroll)
                    };

                    let crossfade_len = res.config.seek_crossfade_samples;
                    let fadeout = capture_fadeout_samples(&stream_state, crossfade_len);

                    stream_state.set_seeking(true);
                    stream_state.flush_buffer();
                    producer.set_file_position(new_pos);

                    let fadein = capture_fadein_samples(
                        &res.sample_cache,
                        &res.metrics,
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
