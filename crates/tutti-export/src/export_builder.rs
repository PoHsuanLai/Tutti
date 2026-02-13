use crate::handle::ExportHandle;
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
        let options = ExportOptions {
            source_sample_rate: sample_rate.round() as u32,
            ..Default::default()
        };
        Self {
            net,
            sample_rate,
            duration_seconds: None,
            options,
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
        self.to_file_with_progress(path, |_| {})
    }

    pub fn to_file_with_progress(
        self,
        path: impl AsRef<Path>,
        on_progress: impl Fn(ExportProgress),
    ) -> Result<()> {
        let options = self.options.clone();
        let (left, right) = self.render_impl(&on_progress)?;

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

    /// Start a non-blocking background export, returning a handle to poll progress.
    ///
    /// The export runs on a dedicated thread. Poll [`ExportHandle::progress()`]
    /// each frame to get status updates, or call [`ExportHandle::wait()`] to block.
    ///
    /// ```ignore
    /// let mut handle = engine.export()
    ///     .duration_seconds(60.0)
    ///     .format(AudioFormat::Wav)
    ///     .start("output.wav");
    ///
    /// loop {
    ///     match handle.progress() {
    ///         ExportStatus::Running(p) => println!("{:.0}%", p.progress * 100.0),
    ///         ExportStatus::Complete => break,
    ///         ExportStatus::Failed(e) => { eprintln!("{}", e); break; }
    ///         ExportStatus::Pending => {}
    ///     }
    /// }
    /// ```
    pub fn start(self, path: impl AsRef<Path>) -> ExportHandle {
        let path = path.as_ref().to_path_buf();
        let (tx, rx) = crossbeam_channel::bounded(64);

        let thread = std::thread::Builder::new()
            .name("tutti-export".into())
            .spawn(move || {
                self.to_file_with_progress(&path, |p| {
                    let _ = tx.try_send(p); // drop if full — UI will catch up
                })
            })
            .expect("failed to spawn export thread");

        ExportHandle::new(rx, thread)
    }

    pub fn render(self) -> Result<(Vec<f32>, Vec<f32>, f64)> {
        let sample_rate = self.sample_rate;
        let (left, right) = self.render_impl(&|_| {})?;
        Ok((left, right, sample_rate))
    }

    pub fn render_with_progress(
        self,
        on_progress: impl Fn(ExportProgress),
    ) -> Result<(Vec<f32>, Vec<f32>, f64)> {
        let sample_rate = self.sample_rate;
        let (left, right) = self.render_impl(&on_progress)?;
        Ok((left, right, sample_rate))
    }

    /// Block-based offline render. Processes audio in blocks of up to 64 samples
    /// (MAX_BUFFER_SIZE) for efficient SIMD processing. If an ExportContext is
    /// present, advances its timeline so transport-aware nodes (samplers, MIDI
    /// synths) produce correct output.
    fn render_impl(self, on_progress: &impl Fn(ExportProgress)) -> Result<(Vec<f32>, Vec<f32>)> {
        use tutti_core::{BufferRef, BufferVec, MAX_BUFFER_SIZE};

        let duration = self.duration_seconds.ok_or_else(|| {
            crate::ExportError::InvalidOptions(
                "Duration not set. Use .duration_seconds() or .duration_beats()".into(),
            )
        })?;

        let mut net = self.net;
        net.set_sample_rate(self.sample_rate);

        let latency_samples = if self.compensate_latency {
            net.latency().unwrap_or(0.0).floor() as usize
        } else {
            0
        };
        let extra_duration = latency_samples as f64 / self.sample_rate;

        let total_samples = ((duration + extra_duration) * self.sample_rate).round() as usize;
        let output_length = (duration * self.sample_rate).round() as usize;

        let mut left = Vec::with_capacity(output_length);
        let mut right = Vec::with_capacity(output_length);

        let mut buffer = BufferVec::new(net.outputs().max(2));
        let progress_interval = (self.sample_rate * 0.5) as usize;
        let mut next_progress = progress_interval;
        let empty_input = BufferRef::new(&[]);

        on_progress(ExportProgress {
            phase: ExportPhase::Rendering,
            progress: 0.0,
        });

        // If context exists, advance by 1 sample so the first processed sample
        // sees beat position "1 sample in" (matches advance-then-tick semantics).
        if let Some(ref context) = self.context {
            context.timeline.advance(1);
        }

        let mut i = 0;
        while i < total_samples {
            let block_size = (total_samples - i).min(MAX_BUFFER_SIZE);

            let mut buffer_mut = buffer.buffer_mut();
            net.process(block_size, &empty_input, &mut buffer_mut);

            // Advance timeline after processing (for next block's position)
            if let Some(ref context) = self.context {
                context.timeline.advance(block_size);
            }

            for j in 0..block_size {
                let sample_idx = i + j;
                if sample_idx >= latency_samples && left.len() < output_length {
                    left.push(buffer_mut.at_f32(0, j));
                    right.push(if net.outputs() >= 2 {
                        buffer_mut.at_f32(1, j)
                    } else {
                        buffer_mut.at_f32(0, j)
                    });
                }
            }

            i += block_size;

            if i >= next_progress || i >= total_samples {
                on_progress(ExportProgress {
                    phase: ExportPhase::Rendering,
                    progress: i as f32 / total_samples as f32,
                });
                next_progress += progress_interval;
            }
        }

        Ok((left, right))
    }
}
