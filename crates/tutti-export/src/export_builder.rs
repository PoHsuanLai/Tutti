//! Export builder for rendering audio from the engine.

use crate::{AudioFormat, ExportOptions, NormalizationMode, Result};
use std::path::Path;
use tutti_core::AudioUnit;

/// Export progress information.
#[derive(Debug, Clone, Copy)]
pub struct ExportProgress {
    /// Current phase.
    pub phase: ExportPhase,
    /// Progress within current phase (0.0 to 1.0).
    pub progress: f32,
}

/// Export phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportPhase {
    /// Rendering the audio graph.
    Rendering,
    /// Processing (resampling, normalization, dithering).
    Processing,
    /// Encoding and writing to file.
    Encoding,
}

/// Builder for exporting audio from the engine.
///
/// # Example
/// ```ignore
/// engine.export()
///     .duration_seconds(10.0)
///     .format(AudioFormat::Flac)
///     .normalize(NormalizationMode::lufs(-14.0))
///     .to_file("output.flac")?;
/// ```
pub struct ExportBuilder {
    net: tutti_core::dsp::Net,
    sample_rate: f64,
    duration_seconds: Option<f64>,
    options: ExportOptions,
    /// When true, trim the initial latency from the rendered audio.
    compensate_latency: bool,
}

impl ExportBuilder {
    /// Create a new export builder.
    pub fn new(net: tutti_core::dsp::Net, sample_rate: f64) -> Self {
        Self {
            net,
            sample_rate,
            duration_seconds: None,
            options: ExportOptions::default(),
            compensate_latency: false,
        }
    }

    /// Set the duration to export in seconds.
    pub fn duration_seconds(mut self, seconds: f64) -> Self {
        self.duration_seconds = Some(seconds);
        self
    }

    /// Set the duration to export in beats.
    pub fn duration_beats(mut self, beats: f64, tempo: f64) -> Self {
        let seconds = (beats / tempo) * 60.0;
        self.duration_seconds = Some(seconds);
        self
    }

    /// Set the audio format.
    pub fn format(mut self, format: AudioFormat) -> Self {
        self.options.format = format;
        self
    }

    /// Set the bit depth.
    pub fn bit_depth(mut self, bit_depth: crate::BitDepth) -> Self {
        self.options.bit_depth = bit_depth;
        self
    }

    /// Set normalization mode.
    pub fn normalize(mut self, mode: NormalizationMode) -> Self {
        self.options.normalization = mode;
        self
    }

    /// Trim initial latency from the rendered audio.
    ///
    /// When enabled, the graph's causal latency (from look-ahead limiters,
    /// linear-phase filters, etc.) is measured and the corresponding number
    /// of leading silent samples are removed from the output. The render
    /// duration is extended internally so the exported audio is still the
    /// requested length.
    ///
    /// Disable this when exporting stems that need to stay time-aligned
    /// with other stems (the pre-delay keeps them in sync).
    pub fn compensate_latency(mut self, enabled: bool) -> Self {
        self.compensate_latency = enabled;
        self
    }

    /// Render and export to a file.
    pub fn to_file(self, path: impl AsRef<Path>) -> Result<()> {
        let options = self.options.clone();
        let (left, right) = self.render_internal()?;
        let path = path.as_ref();
        crate::export_to_file(
            path.to_str()
                .ok_or_else(|| crate::ExportError::InvalidOptions("Invalid path".into()))?,
            &left,
            &right,
            &options,
        )
    }

    /// Render and export to a file with progress callback.
    ///
    /// The callback receives `ExportProgress` with granular updates:
    /// - `Rendering`: Progress updates roughly every 0.5 seconds of audio
    /// - `Processing`: Updates at 0%, 33%, 66%, 100% (resample, normalize, dither)
    /// - `Encoding`: Updates at 0% and 100%
    pub fn to_file_with_progress(
        self,
        path: impl AsRef<Path>,
        on_progress: impl Fn(ExportProgress),
    ) -> Result<()> {
        let options = self.options.clone();
        let (left, right) = self.render_internal_with_progress(&on_progress)?;

        let path = path.as_ref();
        crate::export_to_file_with_progress(
            path.to_str()
                .ok_or_else(|| crate::ExportError::InvalidOptions("Invalid path".into()))?,
            &left,
            &right,
            &options,
            on_progress,
        )
    }

    /// Render to buffers without exporting.
    pub fn render(self) -> Result<(Vec<f32>, Vec<f32>, f64)> {
        let sample_rate = self.sample_rate;
        let (left, right) = self.render_internal()?;
        Ok((left, right, sample_rate))
    }

    /// Render to buffers with progress callback.
    pub fn render_with_progress(
        self,
        on_progress: impl Fn(ExportProgress),
    ) -> Result<(Vec<f32>, Vec<f32>, f64)> {
        let sample_rate = self.sample_rate;
        let (left, right) = self.render_internal_with_progress(&on_progress)?;
        Ok((left, right, sample_rate))
    }

    fn render_internal(self) -> Result<(Vec<f32>, Vec<f32>)> {
        self.render_internal_with_progress(&|_| {})
    }

    fn render_internal_with_progress(
        self,
        on_progress: &impl Fn(ExportProgress),
    ) -> Result<(Vec<f32>, Vec<f32>)> {
        let duration = self.duration_seconds.ok_or_else(|| {
            crate::ExportError::InvalidOptions(
                "Duration not set. Use .duration_seconds() or .duration_beats()".into(),
            )
        })?;

        let mut render_net = self.net;
        render_net.set_sample_rate(self.sample_rate);

        // Measure latency and extend render duration if compensating
        let latency_samples = if self.compensate_latency {
            render_net.latency().unwrap_or(0.0).floor() as usize
        } else {
            0
        };
        let extra_duration = latency_samples as f64 / self.sample_rate;

        let wave = tutti_core::dsp::Wave::render_with_progress(
            self.sample_rate,
            duration + extra_duration,
            &mut render_net,
            |p| {
                on_progress(ExportProgress {
                    phase: ExportPhase::Rendering,
                    progress: p,
                });
            },
        );

        let output_length = (duration * self.sample_rate).round() as usize;
        let mut left = Vec::with_capacity(output_length);
        let mut right = Vec::with_capacity(output_length);

        for i in 0..output_length {
            left.push(wave.at(0, i + latency_samples));
            right.push(wave.at(1, i + latency_samples));
        }

        Ok((left, right))
    }
}
