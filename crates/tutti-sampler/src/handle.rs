//! Fluent handle for sampler operations

use crate::SamplerSystem;
use std::sync::Arc;

/// Fluent handle for sampler operations.
///
/// Works whether or not the sampler is enabled. Methods are no-ops
/// or return graceful errors when disabled.
///
/// # Example
/// ```ignore
/// let sampler = engine.sampler();
/// sampler.stream("file.wav").start();  // No-op if disabled
/// sampler.run();  // No-op if disabled
/// ```
pub struct SamplerHandle {
    sampler: Option<Arc<SamplerSystem>>,
}

impl SamplerHandle {
    /// Create a new handle (internal - use via TuttiEngine)
    #[doc(hidden)]
    pub fn new(sampler: Option<Arc<SamplerSystem>>) -> Self {
        Self { sampler }
    }

    /// Stream an audio file.
    ///
    /// Returns a builder for configuring the stream. When sampler is disabled,
    /// returns a disabled builder that no-ops on start().
    pub fn stream(&self, file_path: impl Into<std::path::PathBuf>) -> crate::StreamBuilder<'_> {
        if let Some(ref sampler) = self.sampler {
            sampler.stream(file_path)
        } else {
            crate::StreamBuilder::disabled()
        }
    }

    /// Resume the butler thread for async I/O.
    ///
    /// No-op when sampler is disabled.
    pub fn run(&self) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.run();
        }
        self
    }

    /// Pause the butler thread.
    ///
    /// No-op when sampler is disabled.
    pub fn pause(&self) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.pause();
        }
        self
    }

    /// Wait for all butler operations to complete.
    ///
    /// No-op when sampler is disabled.
    pub fn wait_for_completion(&self) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.wait_for_completion();
        }
        self
    }

    /// Shutdown the butler thread.
    ///
    /// No-op when sampler is disabled.
    pub fn shutdown(&self) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.shutdown();
        }
        self
    }

    /// Get the sample rate.
    ///
    /// Returns 0.0 when sampler is disabled.
    pub fn sample_rate(&self) -> f64 {
        self.sampler
            .as_ref()
            .map(|s| s.sample_rate())
            .unwrap_or(0.0)
    }

    /// Check if sampler subsystem is enabled.
    pub fn is_enabled(&self) -> bool {
        self.sampler.is_some()
    }

    /// Create an auditioner for quick file preview.
    ///
    /// Returns None when sampler is disabled.
    pub fn auditioner(&self) -> Option<crate::auditioner::Auditioner> {
        self.sampler.as_ref().map(|s| s.auditioner())
    }

    /// Get reference to inner SamplerSystem (advanced use).
    ///
    /// Returns None when sampler is disabled.
    pub fn inner(&self) -> Option<&Arc<SamplerSystem>> {
        self.sampler.as_ref()
    }

    // =========================================================================
    // Convenience methods (delegate to SamplerSystem)
    // =========================================================================

    /// Get cache statistics.
    ///
    /// Returns default (zero) stats when sampler is disabled.
    pub fn cache_stats(&self) -> crate::butler::CacheStats {
        self.sampler
            .as_ref()
            .map(|s| s.cache_stats())
            .unwrap_or_default()
    }

    /// Get I/O metrics snapshot.
    ///
    /// Returns default (zero) metrics when sampler is disabled.
    pub fn io_metrics(&self) -> crate::butler::IOMetricsSnapshot {
        self.sampler
            .as_ref()
            .map(|s| s.io_metrics())
            .unwrap_or_default()
    }

    /// Reset I/O metrics counters.
    ///
    /// No-op when sampler is disabled.
    pub fn reset_io_metrics(&self) {
        if let Some(ref sampler) = self.sampler {
            sampler.reset_io_metrics();
        }
    }

    /// Get buffer fill level for a channel (0.0 to 1.0).
    ///
    /// Returns None when sampler is disabled or channel is not streaming.
    pub fn buffer_fill(&self, channel_index: usize) -> Option<f32> {
        self.sampler.as_ref()?.buffer_fill(channel_index)
    }

    /// Get underrun count for a channel (resets counter).
    ///
    /// Returns 0 when sampler is disabled.
    pub fn take_underruns(&self, channel_index: usize) -> u64 {
        self.sampler
            .as_ref()
            .map(|s| s.take_underruns(channel_index))
            .unwrap_or(0)
    }

    /// Get total underrun count across all channels (resets counters).
    ///
    /// Returns 0 when sampler is disabled.
    pub fn take_all_underruns(&self) -> u64 {
        self.sampler
            .as_ref()
            .map(|s| s.take_all_underruns())
            .unwrap_or(0)
    }

    /// Stream a file to a specific channel.
    ///
    /// Convenience method that pre-sets the channel. Equivalent to:
    /// `sampler.stream(path).channel(channel_index)`
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

    /// Stop streaming for a channel.
    ///
    /// No-op when sampler is disabled.
    pub fn stop_stream(&self, channel_index: usize) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.stop_stream(channel_index);
        }
        self
    }

    /// Seek within a stream to a new position.
    ///
    /// No-op when sampler is disabled.
    pub fn seek(&self, channel_index: usize, position_samples: u64) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.seek(channel_index, position_samples);
        }
        self
    }

    /// Set loop range for a stream (in samples).
    ///
    /// No-op when sampler is disabled.
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

    /// Set loop range with crossfade for smooth transitions.
    ///
    /// No-op when sampler is disabled.
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

    /// Clear loop range for a stream.
    ///
    /// No-op when sampler is disabled.
    pub fn clear_loop_range(&self, channel_index: usize) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.clear_loop_range(channel_index);
        }
        self
    }

    /// Set playback direction for a stream.
    ///
    /// No-op when sampler is disabled.
    pub fn set_direction(&self, channel_index: usize, direction: crate::PlayDirection) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.set_direction(channel_index, direction);
        }
        self
    }

    /// Set playback speed for a stream.
    ///
    /// No-op when sampler is disabled.
    pub fn set_speed(&self, channel_index: usize, speed: f32) -> &Self {
        if let Some(ref sampler) = self.sampler {
            sampler.set_speed(channel_index, speed);
        }
        self
    }

    /// Get a StreamingSamplerUnit for a channel.
    ///
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
