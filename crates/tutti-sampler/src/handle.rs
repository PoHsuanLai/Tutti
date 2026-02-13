//! Fluent handle for sampler operations

use crate::SamplerSystem;
use std::sync::Arc;

/// Fluent handle for sampler operations.
///
/// Works whether or not the sampler is enabled. Methods are no-ops
/// or return graceful errors when disabled.
pub struct SamplerHandle {
    sampler: Option<Arc<SamplerSystem>>,
}

impl SamplerHandle {
    #[doc(hidden)]
    pub fn new(sampler: Option<Arc<SamplerSystem>>) -> Self {
        Self { sampler }
    }

    /// Returns a builder for configuring the stream.
    /// Returns a disabled builder that no-ops on start() when sampler is disabled.
    pub fn stream(&self, file_path: impl Into<std::path::PathBuf>) -> crate::StreamBuilder<'_> {
        if let Some(ref sampler) = self.sampler {
            sampler.stream(file_path)
        } else {
            crate::StreamBuilder::disabled()
        }
    }

    pub fn run(&self) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.run();
        }
        self
    }

    pub fn pause(&self) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.pause();
        }
        self
    }

    pub fn wait_for_completion(&self) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.wait_for_completion();
        }
        self
    }

    pub fn shutdown(&self) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.shutdown();
        }
        self
    }

    /// Returns 0.0 when sampler is disabled.
    pub fn sample_rate(&self) -> f64 {
        self.sampler
            .as_ref()
            .map(|s| s.sample_rate())
            .unwrap_or(0.0)
    }

    pub fn is_enabled(&self) -> bool {
        self.sampler.is_some()
    }

    /// Returns None when sampler is disabled.
    pub fn auditioner(&self) -> Option<crate::auditioner::Auditioner> {
        self.sampler.as_ref().map(|s| s.auditioner())
    }

    /// Returns None when sampler is disabled.
    pub fn inner(&self) -> Option<&Arc<SamplerSystem>> {
        self.sampler.as_ref()
    }

    pub fn start_recording(
        &self,
        channel_index: usize,
        source: crate::RecordingSource,
        mode: crate::RecordingMode,
        current_beat: f64,
    ) -> crate::error::Result<()> {
        let sampler = self.sampler.as_ref().ok_or_else(|| {
            crate::error::Error::Recording("Sampler subsystem is disabled".to_string())
        })?;
        sampler
            .recording()
            .start_recording(channel_index, source, mode, current_beat)
    }

    pub fn stop_recording(
        &self,
        channel_index: usize,
    ) -> crate::error::Result<crate::RecordedData> {
        let sampler = self.sampler.as_ref().ok_or_else(|| {
            crate::error::Error::Recording("Sampler subsystem is disabled".to_string())
        })?;
        sampler.recording().stop_recording(channel_index)
    }

    pub fn is_channel_recording(&self, channel_index: usize) -> bool {
        self.sampler
            .as_ref()
            .map(|s| s.recording().is_recording(channel_index))
            .unwrap_or(false)
    }

    pub fn has_active_recording(&self) -> bool {
        self.sampler
            .as_ref()
            .map(|s| s.recording().has_active_recording())
            .unwrap_or(false)
    }

    pub fn list_input_devices(&self) -> Vec<crate::audio_input::InputDeviceInfo> {
        self.sampler
            .as_ref()
            .map(|s| s.audio_input().list_input_devices())
            .unwrap_or_default()
    }

    pub fn select_input_device(&self, device_index: usize) -> crate::error::Result<()> {
        let sampler = self.sampler.as_ref().ok_or_else(|| {
            crate::error::Error::Recording("Sampler subsystem is disabled".to_string())
        })?;
        sampler.audio_input().select_device(device_index)
    }

    /// Gain range: 0.0 to 2.0.
    pub fn set_input_gain(&self, gain: f32) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.audio_input().set_gain(gain);
        }
        self
    }

    pub fn set_input_monitoring(&self, enabled: bool) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.audio_input().set_monitoring(enabled);
        }
        self
    }

    pub fn input_peak_level(&self) -> f32 {
        self.sampler
            .as_ref()
            .map(|s| s.audio_input().peak_level())
            .unwrap_or(0.0)
    }

    pub fn cache_stats(&self) -> crate::butler::CacheStats {
        self.sampler
            .as_ref()
            .map(|s| s.cache_stats())
            .unwrap_or_default()
    }

    pub fn io_metrics(&self) -> crate::butler::IOMetricsSnapshot {
        self.sampler
            .as_ref()
            .map(|s| s.io_metrics())
            .unwrap_or_default()
    }

    pub fn reset_io_metrics(&self) {
        if let Some(ref sampler) = self.sampler {
            sampler.reset_io_metrics();
        }
    }

    /// Returns fill level 0.0..1.0, or None if disabled/not streaming.
    pub fn buffer_fill(&self, channel_index: usize) -> Option<f32> {
        self.sampler.as_ref()?.buffer_fill(channel_index)
    }

    /// Returns and resets underrun count for a channel.
    pub fn take_underruns(&self, channel_index: usize) -> u64 {
        self.sampler
            .as_ref()
            .map(|s| s.take_underruns(channel_index))
            .unwrap_or(0)
    }

    /// Returns and resets total underrun count across all channels.
    pub fn take_all_underruns(&self) -> u64 {
        self.sampler
            .as_ref()
            .map(|s| s.take_all_underruns())
            .unwrap_or(0)
    }

    /// Stream a file to a specific channel.
    pub fn stream_file(
        &self,
        channel_index: usize,
        file_path: impl Into<std::path::PathBuf>,
    ) -> crate::StreamBuilder<'_> {
        if let Some(ref sampler) = self.sampler {
            sampler.stream_file(channel_index, file_path)
        } else {
            crate::StreamBuilder::disabled()
        }
    }

    pub fn stop_stream(&self, channel_index: usize) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.stop_stream(channel_index);
        }
        self
    }

    pub fn seek(&self, channel_index: usize, position_samples: u64) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.seek(channel_index, position_samples);
        }
        self
    }

    pub fn set_loop_range(
        &self,
        channel_index: usize,
        start_samples: u64,
        end_samples: u64,
    ) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.set_loop_range(channel_index, start_samples, end_samples);
        }
        self
    }

    pub fn set_loop_range_with_crossfade(
        &self,
        channel_index: usize,
        start_samples: u64,
        end_samples: u64,
        crossfade_samples: usize,
    ) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.set_loop_range_with_crossfade(
                channel_index,
                start_samples,
                end_samples,
                crossfade_samples,
            );
        }
        self
    }

    pub fn clear_loop_range(&self, channel_index: usize) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.clear_loop_range(channel_index);
        }
        self
    }

    pub fn set_direction(&self, channel_index: usize, direction: crate::PlayDirection) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.set_direction(channel_index, direction);
        }
        self
    }

    pub fn set_speed(&self, channel_index: usize, speed: f32) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.set_speed(channel_index, speed);
        }
        self
    }

    /// Returns None when sampler is disabled or channel is not streaming.
    pub fn streaming_unit(&self, channel_index: usize) -> Option<crate::StreamingSamplerUnit> {
        self.sampler.as_ref()?.streaming_unit(channel_index)
    }
}

impl Clone for SamplerHandle {
    fn clone(&self) -> Self {
        Self {
            sampler: self.sampler.clone(),
        }
    }
}
