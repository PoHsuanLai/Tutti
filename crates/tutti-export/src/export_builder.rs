use crate::{AudioFormat, ExportOptions, NormalizationMode, Result};
use std::path::Path;
use tutti_core::{AudioUnit, ExportContext};

#[derive(Debug, Clone, Copy)]
pub struct ExportProgress {
    pub phase: ExportPhase,
    /// Progress within current phase (0.0 to 1.0).
    pub progress: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportPhase {
    Rendering,
    Processing,
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
    compensate_latency: bool,
    /// Export context for timeline and MIDI isolation.
    context: Option<ExportContext>,
}

impl ExportBuilder {
    pub fn new(net: tutti_core::dsp::Net, sample_rate: f64) -> Self {
        Self {
            net,
            sample_rate,
            duration_seconds: None,
            options: ExportOptions::default(),
            compensate_latency: false,
            context: None,
        }
    }

    /// Set export context for proper MIDI and timeline handling.
    ///
    /// When set, the export will use the context's timeline for beat
    /// position and MIDI snapshot for non-destructive event delivery.
    pub fn with_context(mut self, context: ExportContext) -> Self {
        self.context = Some(context);
        self
    }

    pub fn duration_seconds(mut self, seconds: f64) -> Self {
        self.duration_seconds = Some(seconds);
        self
    }

    pub fn duration_beats(mut self, beats: f64, tempo: f64) -> Self {
        self.duration_seconds = Some((beats / tempo) * 60.0);
        self
    }

    pub fn format(mut self, format: AudioFormat) -> Self {
        self.options.format = format;
        self
    }

    pub fn bit_depth(mut self, bit_depth: crate::BitDepth) -> Self {
        self.options.bit_depth = bit_depth;
        self
    }

    pub fn normalize(mut self, mode: NormalizationMode) -> Self {
        self.options.normalization = mode;
        self
    }

    /// Trim initial latency (from look-ahead limiters, linear-phase filters, etc.)
    /// from the rendered audio. Disable when exporting stems that need time-alignment.
    pub fn compensate_latency(mut self, enabled: bool) -> Self {
        self.compensate_latency = enabled;
        self
    }

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

    pub fn render(self) -> Result<(Vec<f32>, Vec<f32>, f64)> {
        let sample_rate = self.sample_rate;
        let (left, right) = self.render_internal()?;
        Ok((left, right, sample_rate))
    }

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

        let latency_samples = if self.compensate_latency {
            render_net.latency().unwrap_or(0.0).floor() as usize
        } else {
            0
        };
        let extra_duration = latency_samples as f64 / self.sample_rate;

        // If we have export context, use custom render loop that advances timeline
        if let Some(context) = self.context {
            return Self::render_with_context_impl(
                render_net,
                self.sample_rate,
                duration,
                extra_duration,
                latency_samples,
                context,
                on_progress,
            );
        }

        // Fallback: use Wave::render for simple cases (no MIDI/automation)
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

    /// Render with export context, advancing timeline per sample.
    ///
    /// This allows MIDI synths and automation lanes to read from the
    /// export timeline instead of the live transport.
    fn render_with_context_impl(
        mut net: tutti_core::dsp::Net,
        sample_rate: f64,
        duration: f64,
        extra_duration: f64,
        latency_samples: usize,
        context: ExportContext,
        on_progress: &impl Fn(ExportProgress),
    ) -> Result<(Vec<f32>, Vec<f32>)> {
        let total_samples = ((duration + extra_duration) * sample_rate).round() as usize;
        let output_length = (duration * sample_rate).round() as usize;

        let mut left = Vec::with_capacity(output_length);
        let mut right = Vec::with_capacity(output_length);

        let mut output = [0.0f32; 2];
        let progress_interval = total_samples / 100;

        for i in 0..total_samples {
            // Advance the export timeline by 1 sample
            context.timeline.advance(1);

            // Process one sample
            net.tick(&[], &mut output);

            // Skip latency compensation samples
            if i >= latency_samples && left.len() < output_length {
                left.push(output[0]);
                right.push(output[1]);
            }

            // Progress callback
            if progress_interval > 0 && i % progress_interval == 0 {
                on_progress(ExportProgress {
                    phase: ExportPhase::Rendering,
                    progress: i as f32 / total_samples as f32,
                });
            }
        }

        // Final progress
        on_progress(ExportProgress {
            phase: ExportPhase::Rendering,
            progress: 1.0,
        });

        Ok((left, right))
    }
}
