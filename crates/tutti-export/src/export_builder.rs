//! Export builder for rendering audio from the engine

use crate::{AudioFormat, ExportOptions, NormalizationMode, Result};
use std::path::Path;
use tutti_core::AudioUnit;

/// Builder for exporting audio from the engine.
///
/// Created via `engine.export()`.
///
/// # Example
/// ```ignore
/// engine.export()
///     .duration_seconds(10.0)
///     .format(AudioFormat::Flac)
///     .normalize(NormalizationMode::Lufs(-14.0))
///     .to_file("output.flac")?;
/// ```
pub struct ExportBuilder {
    net: tutti_core::dsp::Net,
    sample_rate: f64,
    duration_seconds: Option<f64>,
    options: ExportOptions,
}

impl ExportBuilder {
    /// Create a new export builder.
    ///
    /// Takes a cloned Net from the audio engine and its sample rate.
    pub fn new(net: tutti_core::dsp::Net, sample_rate: f64) -> Self {
        Self {
            net,
            sample_rate,
            duration_seconds: None,
            options: ExportOptions::default(),
        }
    }

    /// Set the duration to export in seconds.
    pub fn duration_seconds(mut self, seconds: f64) -> Self {
        self.duration_seconds = Some(seconds);
        self
    }

    /// Set the duration to export in beats (uses transport tempo).
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

    /// Render the audio and export to a file.
    ///
    /// The format is auto-detected from the file extension.
    pub fn to_file(self, path: impl AsRef<Path>) -> Result<()> {
        let duration = self.duration_seconds
            .ok_or_else(|| crate::ExportError::InvalidOptions("Duration not set. Use .duration_seconds() or .duration_beats()".into()))?;

        // Render the Net offline
        let mut render_net = self.net;
        render_net.set_sample_rate(self.sample_rate);

        // Render audio
        let wave = tutti_core::dsp::Wave::render(self.sample_rate, duration, &mut render_net);

        // Convert to separate L/R buffers
        let mut left = Vec::with_capacity(wave.length());
        let mut right = Vec::with_capacity(wave.length());

        for i in 0..wave.length() {
            left.push(wave.at(0, i));
            right.push(wave.at(1, i));
        }

        // Export using tutti-export
        let path = path.as_ref();
        crate::export_to_file(
            path.to_str().ok_or_else(|| crate::ExportError::InvalidOptions("Invalid path".into()))?,
            &left,
            &right,
            &self.options,
        )?;

        Ok(())
    }

    /// Render the audio to buffers without exporting to a file.
    ///
    /// Returns `(left_channel, right_channel, sample_rate)`.
    pub fn render(self) -> Result<(Vec<f32>, Vec<f32>, f64)> {
        let duration = self.duration_seconds
            .ok_or_else(|| crate::ExportError::InvalidOptions("Duration not set. Use .duration_seconds() or .duration_beats()".into()))?;

        // Render the Net offline
        let mut render_net = self.net;
        render_net.set_sample_rate(self.sample_rate);

        // Render audio
        let wave = tutti_core::dsp::Wave::render(self.sample_rate, duration, &mut render_net);

        // Convert to separate L/R buffers
        let mut left = Vec::with_capacity(wave.length());
        let mut right = Vec::with_capacity(wave.length());

        for i in 0..wave.length() {
            left.push(wave.at(0, i));
            right.push(wave.at(1, i));
        }

        Ok((left, right, self.sample_rate))
    }
}
