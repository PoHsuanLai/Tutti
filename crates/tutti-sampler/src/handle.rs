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
}

impl Clone for SamplerHandle {
    fn clone(&self) -> Self {
        Self {
            sampler: self.sampler.clone(),
        }
    }
}
