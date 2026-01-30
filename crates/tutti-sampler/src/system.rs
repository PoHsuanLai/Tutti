//! Sampler system - unified API for streaming, recording, and butler operations.

use crate::butler::{
    ButlerCommand, CaptureBuffer, CaptureBufferProducer, CaptureId, FlushPriority, FlushRequest,
    RegionBufferProducer, RegionId,
};
use crate::error::Result;
use crossbeam_channel::Sender;
use std::path::PathBuf;
use tutti_core::Wave;

/// Complete sampler system with butler thread.
pub struct SamplerSystem {
    butler_tx: Sender<ButlerCommand>,
    butler: Option<crate::butler::ButlerThread>,
    recording: std::sync::Arc<crate::recording::manager::RecordingManager>,
    automation: std::sync::Arc<crate::recording::automation_manager::AutomationManager>,
    audio_input: std::sync::Arc<crate::audio_input::manager::AudioInputManager>,
    sample_rate: f64,
    sample_cache: std::sync::Arc<dashmap::DashMap<std::path::PathBuf, std::sync::Arc<Wave>>>,
}

impl SamplerSystem {
    /// Create a new sampler system builder.
    pub fn builder(sample_rate: f64) -> SamplerSystemBuilder {
        SamplerSystemBuilder { sample_rate }
    }

    /// Get the sample rate.
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    // Butler operations

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

    // Capture/Recording

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

        let (producer, consumer) = CaptureBuffer::new(
            id,
            file_path.clone(),
            sample_rate,
            channels,
            buffer_ms as f32,
        );

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
    pub fn flush_capture(&self, id: CaptureId, file_path: impl Into<PathBuf>) -> &Self {
        let _ = self.butler_tx.send(ButlerCommand::Flush(FlushRequest {
            capture_id: id,
            file_path: file_path.into(),
            priority: FlushPriority::Normal,
        }));
        self
    }

    /// Flush all capture buffers.
    pub fn flush_all(&self) -> &Self {
        let _ = self.butler_tx.send(ButlerCommand::FlushAll);
        self
    }

    // Region streaming

    /// Register a region buffer producer.
    pub fn register_region(&self, region_id: RegionId, producer: RegionBufferProducer) -> &Self {
        let _ = self.butler_tx.send(ButlerCommand::RegisterProducer {
            region_id,
            producer,
        });
        self
    }

    /// Remove a region buffer.
    pub fn remove_region(&self, region_id: RegionId) -> &Self {
        let _ = self.butler_tx.send(ButlerCommand::RemoveRegion(region_id));
        self
    }

    /// Seek a region to a new sample position.
    pub fn seek_region(&self, region_id: RegionId, sample_offset: usize) -> &Self {
        let _ = self.butler_tx.send(ButlerCommand::SeekRegion {
            region_id,
            sample_offset,
        });
        self
    }

    // File streaming

    /// Stream an audio file to a channel.
    pub fn stream_file(
        &self,
        channel_index: usize,
        file_path: impl Into<PathBuf>,
    ) -> StreamBuilder {
        StreamBuilder {
            butler_tx: self.butler_tx.clone(),
            channel_index,
            file_path: file_path.into(),
            start_sample: 0,
            duration_samples: usize::MAX,
            offset_samples: 0,
            speed: 1.0,
            gain: 1.0,
        }
    }

    /// Stop streaming for a channel.
    pub fn stop_stream(&self, channel_index: usize) -> &Self {
        let _ = self
            .butler_tx
            .send(ButlerCommand::StopStreaming { channel_index });
        self
    }

    /// Get butler command sender for advanced integration.
    ///
    /// This provides direct access to the butler command channel for advanced
    /// use cases like tutti-export that need custom butler communication.
    /// Most users should use the high-level SamplerSystem API instead.
    pub fn command_sender(&self) -> Sender<ButlerCommand> {
        self.butler_tx.clone()
    }

    /// Get sample cache (for preloading audio files).
    pub fn sample_cache(
        &self,
    ) -> &std::sync::Arc<dashmap::DashMap<std::path::PathBuf, std::sync::Arc<Wave>>> {
        &self.sample_cache
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
}

impl Drop for SamplerSystem {
    fn drop(&mut self) {
        // Shutdown butler thread gracefully
        if let Some(mut butler) = self.butler.take() {
            butler.stop();
        }
    }
}

/// Builder for SamplerSystem.
pub struct SamplerSystemBuilder {
    sample_rate: f64,
}

impl SamplerSystemBuilder {
    /// Build the sampler system (starts butler thread).
    pub fn build(self) -> Result<SamplerSystem> {
        // Create sample cache for loaded audio files
        let sample_cache = std::sync::Arc::new(dashmap::DashMap::new());

        // Create and start butler thread
        let mut butler = crate::butler::ButlerThread::new(
            sample_cache.clone(),
            256, // command channel capacity
            self.sample_rate,
        );

        let butler_tx = butler.command_sender();

        // Start the butler thread
        butler.start();

        // Create RecordingManager
        let recording = std::sync::Arc::new(crate::recording::manager::RecordingManager::new(
            64, // max recording channels
            butler_tx.clone(),
            self.sample_rate,
        ));

        // Create AutomationManager
        let automation =
            std::sync::Arc::new(crate::recording::automation_manager::AutomationManager::new());

        // Create AudioInputManager
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
            sample_cache,
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

/// Builder for file streaming configuration.
pub struct StreamBuilder {
    butler_tx: Sender<ButlerCommand>,
    channel_index: usize,
    file_path: PathBuf,
    start_sample: usize,
    duration_samples: usize,
    offset_samples: usize,
    speed: f32,
    gain: f32,
}

impl StreamBuilder {
    /// Set start sample position.
    pub fn start_sample(mut self, sample: usize) -> Self {
        self.start_sample = sample;
        self
    }

    /// Set duration in samples.
    pub fn duration_samples(mut self, samples: usize) -> Self {
        self.duration_samples = samples;
        self
    }

    /// Set offset in samples.
    pub fn offset_samples(mut self, offset: usize) -> Self {
        self.offset_samples = offset;
        self
    }

    /// Set playback speed.
    pub fn speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// Set gain.
    pub fn gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }

    /// Start streaming.
    pub fn start(self) {
        let _ = self.butler_tx.send(ButlerCommand::StreamAudioFile {
            channel_index: self.channel_index,
            file_path: self.file_path,
            start_sample: self.start_sample,
            duration_samples: self.duration_samples,
            offset_samples: self.offset_samples,
            speed: self.speed,
            gain: self.gain,
        });
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
            .start_sample(1000)
            .duration_samples(44100)
            .speed(1.5)
            .gain(0.8)
            .start();
    }

    #[test]
    fn test_tutti_export_compatibility() {
        // Verify that the API needed by tutti-export is exposed
        let sampler = SamplerSystem::builder(44100.0).build().unwrap();

        // tutti-export needs command_sender() for direct butler communication
        let _command_sender = sampler.command_sender();

        // tutti-export needs to create capture sessions
        let session = sampler.create_capture("/tmp/export.wav", 44100.0, 2, Some(5.0));
        assert_eq!(session.id.0, session.id.0); // CaptureId is public

        // tutti-export needs to start/stop captures
        let session = sampler.start_capture(session);
        sampler.stop_capture(session.id);

        // tutti-export needs flush operations
        sampler.flush_all();
        sampler.wait_for_completion();
    }

    #[test]
    fn test_all_managers_initialized() {
        // Verify that all managers are properly initialized
        let sampler = SamplerSystem::builder(44100.0).build().unwrap();

        // Butler thread running
        assert!(sampler.butler.is_some());
        assert_eq!(sampler.sample_rate(), 44100.0);

        // Recording manager accessible
        let _recording = sampler.recording();

        // Automation manager accessible
        let automation = sampler.automation();
        assert!(automation.is_enabled()); // Enabled by default

        // Audio input manager accessible
        let audio_input = sampler.audio_input();
        assert!(!audio_input.is_capturing());

        // Sample cache accessible
        let cache = sampler.sample_cache();
        assert!(cache.is_empty());
    }
}
