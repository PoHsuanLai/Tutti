//! Sampler system - unified API for streaming, recording, and butler operations.

use crate::butler::{
    BufferConfig, ButlerCommand, CacheStats, CaptureBuffer, CaptureBufferProducer, CaptureId,
    FlushRequest, IOMetricsSnapshot, LruCache, PlayDirection, TransportBridge,
};
use crate::error::Result;
use crossbeam_channel::Sender;
use std::path::PathBuf;
use std::sync::Arc;
use tutti_core::{PdcManager, TransportManager};

/// Complete sampler system with butler thread.
pub struct SamplerSystem {
    butler_tx: Sender<ButlerCommand>,
    butler: Option<crate::butler::ButlerThread>,
    recording: std::sync::Arc<crate::recording::manager::RecordingManager>,
    automation: std::sync::Arc<crate::recording::automation_manager::AutomationManager>,
    audio_input: std::sync::Arc<crate::audio_input::manager::AudioInputManager>,
    sample_rate: f64,
    transport_bridge: Option<TransportBridge>,
    pdc_manager: Option<Arc<PdcManager>>,
}

impl SamplerSystem {
    /// Create a new sampler system builder.
    pub fn builder(sample_rate: f64) -> SamplerSystemBuilder {
        SamplerSystemBuilder {
            sample_rate,
            buffer_config: BufferConfig::default(),
            pdc_manager: None,
        }
    }

    /// Get the sample rate.
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// Resume butler processing.
    pub fn run(&self) -> &Self {
        let _ = self.butler_tx.try_send(ButlerCommand::Run);
        self
    }

    /// Pause butler processing.
    pub fn pause(&self) -> &Self {
        let _ = self.butler_tx.try_send(ButlerCommand::Pause);
        self
    }

    /// Wait for butler to complete current work.
    pub fn wait_for_completion(&self) -> &Self {
        let _ = self.butler_tx.send(ButlerCommand::WaitForCompletion);
        self
    }

    /// Shutdown the butler thread.
    pub fn shutdown(&self) -> &Self {
        let _ = self.butler_tx.send(ButlerCommand::Shutdown);
        self
    }

    /// Create a new capture session for recording.
    pub fn create_capture(
        &self,
        file_path: impl Into<PathBuf>,
        sample_rate: f64,
        channels: usize,
        buffer_seconds: Option<f64>,
    ) -> CaptureSession {
        let buffer_ms = buffer_seconds.unwrap_or(5.0) * 1000.0;
        let id = CaptureId::generate();
        let file_path = file_path.into();

        let (producer, consumer) =
            CaptureBuffer::new(id, file_path.clone(), sample_rate, buffer_ms as f32);

        CaptureSession {
            id,
            producer,
            consumer: Some(consumer),
            file_path,
            sample_rate,
            channels,
        }
    }

    /// Start a capture session.
    pub fn start_capture(&self, mut session: CaptureSession) -> CaptureSession {
        let consumer = session
            .consumer
            .take()
            .expect("Capture session already started");

        let _ = self.butler_tx.send(ButlerCommand::RegisterCapture {
            capture_id: session.id,
            consumer,
            file_path: session.file_path.clone(),
            sample_rate: session.sample_rate,
            channels: session.channels,
        });

        session
    }

    /// Stop a capture session.
    pub fn stop_capture(&self, id: CaptureId) -> &Self {
        let _ = self.butler_tx.send(ButlerCommand::RemoveCapture(id));
        self
    }

    /// Flush a capture buffer to disk.
    pub fn flush_capture(&self, id: CaptureId) -> &Self {
        let _ = self
            .butler_tx
            .send(ButlerCommand::Flush(FlushRequest::new(id)));
        self
    }

    /// Flush all capture buffers.
    pub fn flush_all(&self) -> &Self {
        let _ = self.butler_tx.send(ButlerCommand::FlushAll);
        self
    }

    /// Stream an audio file to a specific channel.
    ///
    /// Convenience method that pre-sets the channel. Equivalent to:
    /// `sampler.stream(path).channel(channel_index)`
    pub fn stream_file(
        &self,
        channel_index: usize,
        file_path: impl Into<PathBuf>,
    ) -> crate::stream_builder::StreamBuilder<'_> {
        crate::stream_builder::StreamBuilder::new(self, file_path).channel(channel_index)
    }

    /// Internal: send stream command to butler thread.
    pub(crate) fn send_stream_command(
        &self,
        channel_index: usize,
        file_path: PathBuf,
        offset_samples: usize,
    ) {
        let _ = self.butler_tx.send(ButlerCommand::StreamAudioFile {
            channel_index,
            file_path,
            offset_samples,
        });
    }

    /// Stop streaming for a channel.
    pub fn stop_stream(&self, channel_index: usize) -> &Self {
        let _ = self
            .butler_tx
            .send(ButlerCommand::StopStreaming { channel_index });
        self
    }

    /// Seek within a stream to a new position.
    pub fn seek(&self, channel_index: usize, position_samples: u64) -> &Self {
        let _ = self.butler_tx.send(ButlerCommand::SeekStream {
            channel_index,
            position_samples,
        });
        self
    }

    /// Set loop range for a stream (in samples).
    pub fn set_loop_range(
        &self,
        channel_index: usize,
        start_samples: u64,
        end_samples: u64,
    ) -> &Self {
        self.set_loop_range_with_crossfade(channel_index, start_samples, end_samples, 0)
    }

    /// Set loop range with crossfade for smooth transitions.
    pub fn set_loop_range_with_crossfade(
        &self,
        channel_index: usize,
        start_samples: u64,
        end_samples: u64,
        crossfade_samples: usize,
    ) -> &Self {
        let _ = self.butler_tx.send(ButlerCommand::SetLoopRange {
            channel_index,
            start_samples,
            end_samples,
            crossfade_samples,
        });
        self
    }

    /// Clear loop range for a stream.
    pub fn clear_loop_range(&self, channel_index: usize) -> &Self {
        let _ = self
            .butler_tx
            .send(ButlerCommand::ClearLoopRange { channel_index });
        self
    }

    /// Set playback direction for a stream.
    pub fn set_direction(&self, channel_index: usize, direction: PlayDirection) -> &Self {
        let _ = self.butler_tx.send(ButlerCommand::SetVarispeed {
            channel_index,
            direction,
            speed: 1.0,
        });
        self
    }

    /// Set playback speed for a stream.
    pub fn set_speed(&self, channel_index: usize, speed: f32) -> &Self {
        let direction = if speed < 0.0 {
            PlayDirection::Reverse
        } else {
            PlayDirection::Forward
        };
        let _ = self.butler_tx.send(ButlerCommand::SetVarispeed {
            channel_index,
            direction,
            speed: speed.abs(),
        });
        self
    }

    /// Get LRU cache for audio files.
    ///
    /// The cache automatically evicts least-recently-used entries when
    /// limits are exceeded.
    pub fn cache(&self) -> Option<Arc<LruCache>> {
        self.butler.as_ref().map(|b| b.cache())
    }

    /// Get cache statistics.
    pub fn cache_stats(&self) -> CacheStats {
        self.butler
            .as_ref()
            .map(|b| b.cache().stats())
            .unwrap_or(CacheStats {
                entries: 0,
                bytes: 0,
                max_entries: 0,
                max_bytes: 0,
            })
    }

    /// Get I/O metrics snapshot.
    ///
    /// Returns statistics about disk I/O operations including:
    /// - Bytes read/written
    /// - Read/write operation counts
    /// - Cache hit/miss rates
    /// - Low buffer events
    pub fn io_metrics(&self) -> IOMetricsSnapshot {
        self.butler
            .as_ref()
            .map(|b| b.metrics().snapshot())
            .unwrap_or_default()
    }

    /// Reset I/O metrics counters.
    pub fn reset_io_metrics(&self) {
        if let Some(butler) = self.butler.as_ref() {
            butler.metrics().reset();
        }
    }

    /// Get buffer fill level for a channel (0.0 to 1.0).
    ///
    /// Returns the current buffer fullness for disk streaming.
    /// Values near 0.0 indicate the buffer is nearly empty and may underrun.
    /// Returns `None` if the channel is not streaming.
    pub fn buffer_fill(&self, channel_index: usize) -> Option<f32> {
        let butler = self.butler.as_ref()?;
        let states = butler.stream_states();
        states
            .get(&channel_index)
            .map(|s| s.shared_state().buffer_fill())
    }

    /// Get the recording manager (for MIDI/audio recording).
    pub fn recording(&self) -> &std::sync::Arc<crate::recording::manager::RecordingManager> {
        &self.recording
    }

    /// Get the automation manager (for parameter automation).
    pub fn automation(
        &self,
    ) -> &std::sync::Arc<crate::recording::automation_manager::AutomationManager> {
        &self.automation
    }

    /// Get the audio input manager (for hardware audio input).
    pub fn audio_input(&self) -> &std::sync::Arc<crate::audio_input::manager::AudioInputManager> {
        &self.audio_input
    }

    /// Bind to a transport manager for synchronized playback.
    ///
    /// The transport bridge polls the transport manager and translates
    /// state changes (play/stop/locate/loop) to butler commands.
    /// Commands are broadcast to ALL active streaming channels.
    pub fn bind_transport(&mut self, transport: Arc<TransportManager>) {
        self.transport_bridge = None;

        let stream_states = self
            .butler
            .as_ref()
            .map(|b| b.stream_states())
            .unwrap_or_else(|| std::sync::Arc::new(dashmap::DashMap::new()));

        let bridge = TransportBridge::new(
            transport,
            self.butler_tx.clone(),
            stream_states,
            self.sample_rate,
        );
        self.transport_bridge = Some(bridge);
    }

    /// Unbind from transport manager.
    pub fn unbind_transport(&mut self) {
        self.transport_bridge = None;
    }

    /// Check if transport is bound.
    pub fn has_transport(&self) -> bool {
        self.transport_bridge.is_some()
    }

    /// Get a StreamingSamplerUnit for a channel.
    ///
    /// Returns None if:
    /// - The channel is not currently streaming
    /// - The butler thread is not running
    ///
    /// The returned unit can be inserted into a FunDSP graph for audio playback.
    /// It will automatically receive varispeed and seeking updates from the butler.
    ///
    /// # Example
    /// ```ignore
    /// // Start streaming first
    /// sampler.stream_file(0, "audio.wav").start();
    ///
    /// // Get the audio unit for the graph (auto-plays, like SamplerUnit)
    /// if let Some(mut unit) = sampler.streaming_unit(0) {
    ///     // Insert unit into audio graph and call tick()...
    /// }
    /// ```
    pub fn streaming_unit(&self, channel_index: usize) -> Option<crate::StreamingSamplerUnit> {
        let butler = self.butler.as_ref()?;
        let stream_states = butler.stream_states();
        let stream_state = stream_states.get(&channel_index)?;

        if !stream_state.is_streaming() {
            return None;
        }

        let consumer = stream_state.consumer()?;
        let shared_state = stream_state.shared_state();

        Some(crate::StreamingSamplerUnit::new(consumer, shared_state))
    }

    /// Check if a channel is currently streaming.
    pub fn is_streaming(&self, channel_index: usize) -> bool {
        let Some(butler) = self.butler.as_ref() else {
            return false;
        };
        let stream_states = butler.stream_states();
        stream_states
            .get(&channel_index)
            .map(|s| s.is_streaming())
            .unwrap_or(false)
    }

    /// Get underrun count for a channel (resets counter after reading).
    ///
    /// Returns the number of buffer underruns since the last call.
    /// Useful for monitoring disk I/O performance.
    pub fn take_underruns(&self, channel_index: usize) -> u64 {
        let Some(butler) = self.butler.as_ref() else {
            return 0;
        };
        butler
            .stream_states()
            .get(&channel_index)
            .map(|s| s.shared_state().take_underrun_count())
            .unwrap_or(0)
    }

    /// Get total underruns across all channels (resets counters after reading).
    ///
    /// Returns the sum of underruns from all active streaming channels.
    pub fn take_all_underruns(&self) -> u64 {
        let Some(butler) = self.butler.as_ref() else {
            return 0;
        };
        let states = butler.stream_states();
        let mut total = 0u64;
        for entry in states.iter() {
            total += entry.value().shared_state().take_underrun_count();
        }
        total
    }

    /// Get current PDC preroll for a channel in samples.
    ///
    /// Returns the number of samples the butler is pre-rolling this stream
    /// to compensate for plugin delay.
    pub fn channel_pdc_preroll(&self, channel_index: usize) -> Option<u64> {
        let butler = self.butler.as_ref()?;
        let states = butler.stream_states();
        states.get(&channel_index).map(|s| s.pdc_preroll())
    }

    /// Manually update PDC preroll for a channel.
    ///
    /// Normally PDC is managed automatically via the PdcManager, but this
    /// allows manual override for testing or special cases.
    pub fn update_pdc_preroll(&self, channel_index: usize, new_preroll: u64) -> &Self {
        let _ = self.butler_tx.send(ButlerCommand::UpdatePdcPreroll {
            channel_index,
            new_preroll,
        });
        self
    }

    /// Check if PDC is enabled.
    pub fn is_pdc_enabled(&self) -> bool {
        self.pdc_manager
            .as_ref()
            .map(|pdc| pdc.is_enabled())
            .unwrap_or(false)
    }

    /// Get the PDC manager (if set).
    pub fn pdc_manager(&self) -> Option<&Arc<PdcManager>> {
        self.pdc_manager.as_ref()
    }

    /// Create a new Auditioner for preview playback.
    ///
    /// The Auditioner uses a reserved internal channel for streaming
    /// and the LRU cache for instant preview of recently-accessed files.
    pub fn auditioner(self: &Arc<Self>) -> crate::auditioner::Auditioner {
        crate::auditioner::Auditioner::new(Arc::clone(self))
    }

    /// Stream an audio file with fluent API.
    ///
    /// # Example
    /// ```ignore
    /// sampler.stream("long_audio.wav")
    ///     .channel(0)
    ///     .gain(0.8)
    ///     .speed(1.5)
    ///     .start();
    /// ```
    pub fn stream(
        &self,
        file_path: impl Into<std::path::PathBuf>,
    ) -> crate::stream_builder::StreamBuilder<'_> {
        crate::stream_builder::StreamBuilder::new(self, file_path)
    }

    /// Record audio with fluent API.
    ///
    /// # Example
    /// ```ignore
    /// let session = sampler.record("output.wav")
    ///     .channels(2)
    ///     .buffer_seconds(5.0)
    ///     .start();
    ///
    /// // Audio callback writes to session.producer
    ///
    /// // Later...
    /// sampler.stop_capture(session.id);
    /// sampler.flush_capture(session.id, "final.wav");
    /// ```
    pub fn record(
        &self,
        file_path: impl Into<std::path::PathBuf>,
    ) -> crate::stream_builder::RecordBuilder<'_> {
        crate::stream_builder::RecordBuilder::new(self, file_path)
    }
}

impl Drop for SamplerSystem {
    fn drop(&mut self) {
        self.transport_bridge = None;

        if let Some(mut butler) = self.butler.take() {
            butler.stop();
        }
    }
}

/// Builder for SamplerSystem.
pub struct SamplerSystemBuilder {
    sample_rate: f64,
    buffer_config: BufferConfig,
    pdc_manager: Option<Arc<PdcManager>>,
}

impl SamplerSystemBuilder {
    /// Set buffer duration in seconds (default: 10.0).
    pub fn buffer_seconds(mut self, seconds: f64) -> Self {
        self.buffer_config = BufferConfig::with_buffer_seconds(seconds);
        self
    }

    /// Set full buffer configuration.
    pub fn buffer_config(mut self, config: BufferConfig) -> Self {
        self.buffer_config = config;
        self
    }

    /// Set PDC manager for automatic delay compensation.
    ///
    /// When set, the butler thread will automatically apply preroll
    /// to streams based on channel latency compensation values.
    pub fn pdc_manager(mut self, pdc: Arc<PdcManager>) -> Self {
        self.pdc_manager = Some(pdc);
        self
    }

    /// Build the sampler system (starts butler thread).
    pub fn build(self) -> Result<SamplerSystem> {
        let mut butler = crate::butler::ButlerThread::with_config(
            256, // command channel capacity
            self.sample_rate,
            self.buffer_config,
        );

        if let Some(ref pdc) = self.pdc_manager {
            butler = butler.with_pdc(Arc::clone(pdc));
        }

        let butler_tx = butler.command_sender();

        butler.start();

        let recording = std::sync::Arc::new(crate::recording::manager::RecordingManager::new(
            64, // max recording channels
            butler_tx.clone(),
            self.sample_rate,
        ));

        let automation =
            std::sync::Arc::new(crate::recording::automation_manager::AutomationManager::new());

        let audio_input = std::sync::Arc::new(crate::audio_input::manager::AudioInputManager::new(
            self.sample_rate as u32,
        ));

        Ok(SamplerSystem {
            butler_tx,
            butler: Some(butler),
            recording,
            automation,
            audio_input,
            sample_rate: self.sample_rate,
            transport_bridge: None,
            pdc_manager: self.pdc_manager,
        })
    }
}

/// A capture session for recording or export.
pub struct CaptureSession {
    /// Capture ID.
    pub id: CaptureId,
    producer: CaptureBufferProducer,
    consumer: Option<crate::butler::CaptureBufferConsumer>,
    file_path: PathBuf,
    sample_rate: f64,
    channels: usize,
}

impl CaptureSession {
    /// Get file path.
    pub fn file_path(&self) -> &PathBuf {
        &self.file_path
    }

    /// Get sample rate.
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// Get channel count.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Check if started.
    pub fn is_started(&self) -> bool {
        self.consumer.is_none()
    }

    /// Get mutable producer.
    pub fn producer_mut(&mut self) -> &mut CaptureBufferProducer {
        &mut self.producer
    }

    /// Get producer.
    pub fn producer(&self) -> &CaptureBufferProducer {
        &self.producer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sampler_system_creation() {
        let sampler = SamplerSystem::builder(44100.0).build().unwrap();
        assert_eq!(sampler.sample_rate(), 44100.0);
    }

    #[test]
    fn test_create_capture_session() {
        let sampler = SamplerSystem::builder(44100.0).build().unwrap();
        let session = sampler.create_capture("/tmp/test.wav", 44100.0, 2, None);

        assert!(!session.is_started());
        assert_eq!(session.sample_rate(), 44100.0);
        assert_eq!(session.channels(), 2);
    }

    #[test]
    fn test_stream_builder() {
        let sampler = SamplerSystem::builder(44100.0).build().unwrap();

        sampler
            .stream_file(0, "/path/to/file.wav")
            .offset_samples(1000)
            .start();
    }

    #[test]
    fn test_tutti_export_compatibility() {
        let sampler = SamplerSystem::builder(44100.0).build().unwrap();

        let session = sampler.create_capture("/tmp/export.wav", 44100.0, 2, Some(5.0));
        assert_eq!(session.id.0, session.id.0);

        let session = sampler.start_capture(session);
        sampler.stop_capture(session.id);

        sampler.flush_all();
        sampler.wait_for_completion();
    }

    #[test]
    fn test_all_managers_initialized() {
        let sampler = SamplerSystem::builder(44100.0).build().unwrap();

        assert!(sampler.butler.is_some());
        assert_eq!(sampler.sample_rate(), 44100.0);

        let _recording = sampler.recording();

        let automation = sampler.automation();
        assert!(automation.is_enabled());

        let audio_input = sampler.audio_input();
        assert!(!audio_input.is_capturing());

        let cache = sampler.cache();
        assert!(cache.is_some());
        assert!(cache.unwrap().is_empty());
    }

    #[test]
    fn test_io_metrics() {
        let sampler = SamplerSystem::builder(44100.0).build().unwrap();

        let metrics = sampler.io_metrics();
        assert_eq!(metrics.bytes_read, 0);
        assert_eq!(metrics.bytes_written, 0);
        assert_eq!(metrics.cache_hits, 0);
        assert_eq!(metrics.cache_misses, 0);

        assert!((metrics.cache_hit_rate() - 1.0).abs() < 0.001);

        sampler.reset_io_metrics();
    }

    #[test]
    fn test_cache_stats() {
        let sampler = SamplerSystem::builder(44100.0).build().unwrap();

        let stats = sampler.cache_stats();
        assert_eq!(stats.entries, 0);
        assert_eq!(stats.bytes, 0);
        assert_eq!(stats.max_entries, 64);
        assert_eq!(stats.max_bytes, 1024 * 1024 * 1024);
    }

    #[test]
    fn test_streaming_unit_not_streaming() {
        let sampler = SamplerSystem::builder(44100.0).build().unwrap();

        assert!(!sampler.is_streaming(0));
        assert!(sampler.streaming_unit(0).is_none());
    }

    #[test]
    fn test_underrun_monitoring() {
        let sampler = SamplerSystem::builder(44100.0).build().unwrap();

        assert_eq!(sampler.take_underruns(0), 0);
        assert_eq!(sampler.take_all_underruns(), 0);
    }

    #[test]
    fn test_pdc_disabled_by_default() {
        let sampler = SamplerSystem::builder(44100.0).build().unwrap();

        assert!(!sampler.is_pdc_enabled());
        assert!(sampler.pdc_manager().is_none());
        assert!(sampler.channel_pdc_preroll(0).is_none());
    }

    #[test]
    fn test_pdc_manager_integration() {
        use tutti_core::PdcManager;

        let pdc = Arc::new(PdcManager::new(4, 2));
        pdc.set_channel_latency(0, 100);
        pdc.set_channel_latency(1, 200);

        let sampler = SamplerSystem::builder(44100.0)
            .pdc_manager(pdc.clone())
            .build()
            .unwrap();

        assert!(sampler.is_pdc_enabled());
        assert!(sampler.pdc_manager().is_some());
        assert_eq!(pdc.max_latency(), 200);
        assert_eq!(pdc.get_channel_compensation(0), 100);
        assert_eq!(pdc.get_channel_compensation(1), 0);
    }

    #[test]
    fn test_update_pdc_preroll_command() {
        let sampler = SamplerSystem::builder(44100.0).build().unwrap();

        sampler.update_pdc_preroll(0, 500);
    }
}
