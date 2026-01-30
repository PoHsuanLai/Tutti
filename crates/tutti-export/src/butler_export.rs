//! Butler-backed export with optimized disk I/O
//!
//! Uses tutti-sampler's Butler thread for non-blocking disk writes.
//! Ideal for long recordings or when you need progress monitoring.
//!
//! # Basic Usage
//!
//! ```ignore
//! use tutti_export::{ButlerExporter, ExportOptions};
//! use tutti_sampler::SamplerSystem;
//!
//! // Create sampler system (provides Butler)
//! let sampler = SamplerSystem::builder(44100.0).build();
//! let exporter = ButlerExporter::new(&sampler);
//!
//! // Export with progress callback
//! exporter.export_blocking_with_progress(
//!     &left,
//!     &right,
//!     "output.wav",
//!     &options,
//!     |progress| {
//!         println!("Progress: {:.0}%", progress * 100.0);
//!     }
//! )?;
//! ```
//!
//! # Async Integration
//!
//! For async contexts, wrap in `spawn_blocking`:
//!
//! ```ignore
//! use tokio::sync::mpsc;
//!
//! let (tx, mut rx) = mpsc::channel(100);
//!
//! let handle = tokio::task::spawn_blocking(move || {
//!     exporter.export_blocking_with_progress(&left, &right, path, &options, |p| {
//!         let _ = tx.blocking_send(p);
//!     })
//! });
//!
//! // In async context
//! while let Some(progress) = rx.recv().await {
//!     update_ui(progress);
//! }
//!
//! handle.await??;
//! ```

use crate::error::{ExportError, Result};
use crate::options::ExportOptions;
use std::path::Path;
use tutti_sampler::SamplerSystem;

/// Chunk size for streaming export (in samples)
const DEFAULT_CHUNK_SIZE: usize = 4410;

/// Butler-backed audio exporter
///
/// Uses tutti-sampler's Butler thread for optimized non-blocking disk writes.
/// The Butler handles async disk I/O in a background thread, preventing audio
/// dropouts during long recordings.
pub struct ButlerExporter<'a> {
    sampler: &'a SamplerSystem,
    chunk_size: usize,
}

impl<'a> ButlerExporter<'a> {
    /// Create a new Butler exporter
    ///
    /// # Arguments
    ///
    /// * `sampler` - Reference to a SamplerSystem (provides Butler thread)
    pub fn new(sampler: &'a SamplerSystem) -> Self {
        Self {
            sampler,
            chunk_size: DEFAULT_CHUNK_SIZE,
        }
    }

    /// Set the chunk size for streaming (in samples)
    ///
    /// Default is 4410 samples (~100ms at 44.1kHz).
    /// Larger chunks = fewer Butler thread wakeups, but higher latency.
    pub fn with_chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = chunk_size;
        self
    }

    /// Export audio to WAV file (blocking)
    ///
    /// This blocks until the export is complete.
    ///
    /// # Arguments
    ///
    /// * `left` - Left channel samples (normalized -1.0 to 1.0)
    /// * `right` - Right channel samples (normalized -1.0 to 1.0)
    /// * `path` - Output file path
    /// * `options` - Export configuration
    ///
    /// # Example
    ///
    /// ```ignore
    /// let exporter = ButlerExporter::new(&sampler);
    /// exporter.export_blocking(&left, &right, "output.wav", &options)?;
    /// ```
    pub fn export_blocking(
        &self,
        left: &[f32],
        right: &[f32],
        path: impl AsRef<Path>,
        options: &ExportOptions,
    ) -> Result<()> {
        self.export_blocking_with_progress(left, right, path, options, |_| {})
    }

    /// Export audio to WAV file with progress callback (blocking)
    ///
    /// The callback is invoked periodically with progress (0.0 to 1.0).
    ///
    /// # Arguments
    ///
    /// * `left` - Left channel samples (normalized -1.0 to 1.0)
    /// * `right` - Right channel samples (normalized -1.0 to 1.0)
    /// * `path` - Output file path
    /// * `options` - Export configuration
    /// * `on_progress` - Callback invoked with progress (0.0 to 1.0)
    ///
    /// # Example
    ///
    /// ```ignore
    /// exporter.export_blocking_with_progress(
    ///     &left,
    ///     &right,
    ///     "output.wav",
    ///     &options,
    ///     |progress| {
    ///         println!("Exporting: {:.0}%", progress * 100.0);
    ///     }
    /// )?;
    /// ```
    pub fn export_blocking_with_progress(
        &self,
        left: &[f32],
        right: &[f32],
        path: impl AsRef<Path>,
        options: &ExportOptions,
        on_progress: impl Fn(f32),
    ) -> Result<()> {
        if left.len() != right.len() {
            return Err(ExportError::InvalidData(
                "Left and right channels have different lengths".into(),
            ));
        }

        let path = path.as_ref();
        let channels = if options.mono { 1 } else { 2 };
        let sample_rate = options.output_sample_rate() as f64;

        // Create and start capture session
        let session =
            self.sampler
                .create_capture(path.to_path_buf(), sample_rate, channels, Some(5.0));
        let mut session = self.sampler.start_capture(session);

        let total_samples = left.len();
        let mut written = 0;
        let producer = session.producer_mut();

        // Stream audio in chunks
        while written < total_samples {
            let chunk_end = (written + self.chunk_size).min(total_samples);
            let chunk_len = chunk_end - written;

            if options.mono {
                // Downmix to mono
                for i in 0..chunk_len {
                    let mono = (left[written + i] + right[written + i]) * 0.5;
                    write_sample_with_retry(producer, (mono, mono))?;
                }
            } else {
                // Stereo
                for i in 0..chunk_len {
                    write_sample_with_retry(producer, (left[written + i], right[written + i]))?;
                }
            }

            written = chunk_end;
            on_progress(written as f32 / total_samples as f32);
        }

        // Finalize - flush all data and close file
        self.sampler.flush_all();
        self.sampler.stop_capture(session.id);
        self.sampler.wait_for_completion();

        Ok(())
    }
}

// =============================================================================
// Internal helpers
// =============================================================================

/// Write a sample to the Butler buffer with retry on full
fn write_sample_with_retry(
    producer: &mut tutti_sampler::butler::CaptureBufferProducer,
    sample: (f32, f32),
) -> Result<()> {
    // Try immediate write
    if !producer.write(sample) {
        // Buffer full, sleep and retry once
        std::thread::sleep(std::time::Duration::from_millis(10));
        if !producer.write(sample) {
            return Err(ExportError::Render(
                "Butler buffer full - disk I/O too slow".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_chunk_size() {
        assert_eq!(DEFAULT_CHUNK_SIZE, 4410);
    }
}
