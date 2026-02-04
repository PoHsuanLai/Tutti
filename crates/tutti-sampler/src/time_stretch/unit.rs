//! Time-stretching audio unit wrapper.

use std::sync::Arc;
use tutti_core::{AtomicFloat, AudioUnit, BufferMut, BufferRef, SignalFrame};

use super::granular::{GrainSize, GranularProcessor};
use super::phase_vocoder::PhaseVocoderProcessor;
use super::types::{FftSize, TimeStretchAlgorithm};

/// Internal processor enum for algorithm switching
enum Processor {
    PhaseVocoder(PhaseVocoderProcessor),
    Granular(GranularProcessor),
}

impl Processor {
    fn latency_samples(&self) -> usize {
        match self {
            Processor::PhaseVocoder(p) => p.latency_samples(),
            Processor::Granular(p) => p.latency_samples(),
        }
    }

    fn reset(&mut self) {
        match self {
            Processor::PhaseVocoder(p) => p.reset(),
            Processor::Granular(p) => p.reset(),
        }
    }

    fn set_sample_rate(&mut self, sr: f64) {
        match self {
            Processor::PhaseVocoder(p) => p.set_sample_rate(sr),
            Processor::Granular(p) => p.set_sample_rate(sr),
        }
    }

    fn push_input(&mut self, samples: &[f32]) {
        match self {
            Processor::PhaseVocoder(p) => p.push_input(samples),
            Processor::Granular(p) => p.push_input(samples),
        }
    }

    fn process(&mut self, stretch: f32, pitch_ratio: f32) {
        match self {
            Processor::PhaseVocoder(p) => p.process(stretch, pitch_ratio),
            Processor::Granular(p) => p.process(stretch, pitch_ratio),
        }
    }

    fn pop_output(&mut self, output: &mut [f32]) -> usize {
        match self {
            Processor::PhaseVocoder(p) => p.pop_output(output),
            Processor::Granular(p) => p.pop_output(output),
        }
    }
}

impl Clone for Processor {
    fn clone(&self) -> Self {
        match self {
            Processor::PhaseVocoder(p) => Processor::PhaseVocoder(p.clone()),
            Processor::Granular(p) => Processor::Granular(p.clone()),
        }
    }
}

/// Real-time time-stretching and pitch-shifting unit.
pub struct TimeStretchUnit {
    source: Box<dyn AudioUnit>,
    processor_left: Processor,
    processor_right: Processor,
    stretch_factor: Arc<AtomicFloat>,
    pitch_cents: Arc<AtomicFloat>,
    enabled: bool,
    algorithm: TimeStretchAlgorithm,
    sample_rate: f64,
    source_buffer: Vec<f32>,
    scratch_left: Vec<f32>,
    scratch_right: Vec<f32>,
    scratch_out_left: Vec<f32>,
    scratch_out_right: Vec<f32>,
}

impl TimeStretchUnit {
    /// Create with phase vocoder algorithm (default)
    pub fn new(source: Box<dyn AudioUnit>, sample_rate: f64) -> Self {
        Self::with_fft_size(source, sample_rate, FftSize::default())
    }

    /// Create with custom FFT size (phase vocoder)
    pub fn with_fft_size(source: Box<dyn AudioUnit>, sample_rate: f64, fft_size: FftSize) -> Self {
        Self {
            source,
            processor_left: Processor::PhaseVocoder(PhaseVocoderProcessor::new(
                fft_size,
                sample_rate,
            )),
            processor_right: Processor::PhaseVocoder(PhaseVocoderProcessor::new(
                fft_size,
                sample_rate,
            )),
            stretch_factor: Arc::new(AtomicFloat::new(1.0)),
            pitch_cents: Arc::new(AtomicFloat::new(0.0)),
            enabled: true,
            algorithm: TimeStretchAlgorithm::PhaseVocoder,
            sample_rate,
            source_buffer: vec![0.0; 2],
            scratch_left: Vec::new(),
            scratch_right: Vec::new(),
            scratch_out_left: Vec::new(),
            scratch_out_right: Vec::new(),
        }
    }

    /// Create with granular algorithm (better for drums/transients)
    ///
    /// Note: Granular does NOT support pitch shifting - use phase vocoder for that.
    pub fn with_granular(
        source: Box<dyn AudioUnit>,
        sample_rate: f64,
        grain_size: GrainSize,
    ) -> Self {
        Self {
            source,
            processor_left: Processor::Granular(GranularProcessor::new(grain_size, sample_rate)),
            processor_right: Processor::Granular(GranularProcessor::new(grain_size, sample_rate)),
            stretch_factor: Arc::new(AtomicFloat::new(1.0)),
            pitch_cents: Arc::new(AtomicFloat::new(0.0)),
            enabled: true,
            algorithm: TimeStretchAlgorithm::Granular,
            sample_rate,
            source_buffer: vec![0.0; 2],
            scratch_left: Vec::new(),
            scratch_right: Vec::new(),
            scratch_out_left: Vec::new(),
            scratch_out_right: Vec::new(),
        }
    }

    /// Get the current algorithm
    pub fn algorithm(&self) -> TimeStretchAlgorithm {
        self.algorithm
    }

    /// Set stretch factor (1.0 = normal, 2.0 = half speed, 0.5 = double speed)
    pub fn set_stretch_factor(&self, factor: f32) {
        self.stretch_factor.set(factor.clamp(0.25, 4.0));
    }

    /// Get current stretch factor
    pub fn stretch_factor(&self) -> f32 {
        self.stretch_factor.get()
    }

    /// Get Arc for lock-free external control
    pub fn stretch_factor_arc(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.stretch_factor)
    }

    /// Set pitch shift in cents (only works with PhaseVocoder algorithm)
    pub fn set_pitch_cents(&self, cents: f32) {
        self.pitch_cents.set(cents.clamp(-2400.0, 2400.0));
    }

    /// Get current pitch shift in cents
    pub fn pitch_cents(&self) -> f32 {
        self.pitch_cents.get()
    }

    /// Get Arc for lock-free external control
    pub fn pitch_cents_arc(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.pitch_cents)
    }

    /// Enable/disable processing
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Check if processing is active (not passthrough)
    pub fn is_processing(&self) -> bool {
        if !self.enabled {
            return false;
        }
        let stretch = self.stretch_factor.get();
        let pitch = self.pitch_cents.get();
        (stretch - 1.0).abs() > 0.001 || pitch.abs() > 0.5
    }

    /// Get latency in samples
    pub fn latency_samples(&self) -> usize {
        self.processor_left.latency_samples()
    }

    /// Get source unit
    pub fn source(&self) -> &dyn AudioUnit {
        &*self.source
    }

    /// Get mutable source unit
    pub fn source_mut(&mut self) -> &mut dyn AudioUnit {
        &mut *self.source
    }

    #[inline]
    fn pitch_ratio(&self) -> f32 {
        2.0_f32.powf(self.pitch_cents.get() / 1200.0)
    }
}

impl Clone for TimeStretchUnit {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            processor_left: self.processor_left.clone(),
            processor_right: self.processor_right.clone(),
            stretch_factor: Arc::new(AtomicFloat::new(self.stretch_factor.get())),
            pitch_cents: Arc::new(AtomicFloat::new(self.pitch_cents.get())),
            enabled: self.enabled,
            algorithm: self.algorithm,
            sample_rate: self.sample_rate,
            source_buffer: self.source_buffer.clone(),
            scratch_left: self.scratch_left.clone(),
            scratch_right: self.scratch_right.clone(),
            scratch_out_left: self.scratch_out_left.clone(),
            scratch_out_right: self.scratch_out_right.clone(),
        }
    }
}

impl AudioUnit for TimeStretchUnit {
    fn inputs(&self) -> usize {
        self.source.inputs()
    }

    fn outputs(&self) -> usize {
        2
    }

    fn reset(&mut self) {
        self.source.reset();
        self.processor_left.reset();
        self.processor_right.reset();
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
        self.source.set_sample_rate(sample_rate);
        self.processor_left.set_sample_rate(sample_rate);
        self.processor_right.set_sample_rate(sample_rate);
    }

    fn tick(&mut self, input: &[f32], output: &mut [f32]) {
        self.source.tick(input, &mut self.source_buffer);

        if !self.is_processing() {
            if output.len() >= 2 {
                output[0] = self.source_buffer[0];
                output[1] = self
                    .source_buffer
                    .get(1)
                    .copied()
                    .unwrap_or(self.source_buffer[0]);
            }
            return;
        }

        let stretch = self.stretch_factor.get();
        let pitch_ratio = self.pitch_ratio();

        self.processor_left.push_input(&[self.source_buffer[0]]);
        let right = self
            .source_buffer
            .get(1)
            .copied()
            .unwrap_or(self.source_buffer[0]);
        self.processor_right.push_input(&[right]);

        self.processor_left.process(stretch, pitch_ratio);
        self.processor_right.process(stretch, pitch_ratio);

        if output.len() >= 2 {
            let mut left = [0.0f32];
            let mut right = [0.0f32];
            self.processor_left.pop_output(&mut left);
            self.processor_right.pop_output(&mut right);
            output[0] = left[0];
            output[1] = right[0];
        }
    }

    fn process(&mut self, size: usize, input: &BufferRef, output: &mut BufferMut) {
        if self.scratch_left.len() < size {
            self.scratch_left.resize(size, 0.0);
            self.scratch_right.resize(size, 0.0);
            self.scratch_out_left.resize(size, 0.0);
            self.scratch_out_right.resize(size, 0.0);
        }

        let has_inputs = self.source.inputs() > 0;
        let mut input_sample = [0.0f32];
        for i in 0..size {
            if has_inputs {
                input_sample[0] = input.at_f32(0, i);
                self.source.tick(&input_sample, &mut self.source_buffer);
            } else {
                self.source.tick(&[], &mut self.source_buffer);
            }
            self.scratch_left[i] = self.source_buffer[0];
            self.scratch_right[i] = self
                .source_buffer
                .get(1)
                .copied()
                .unwrap_or(self.source_buffer[0]);
        }

        if !self.is_processing() {
            for i in 0..size {
                output.set_f32(0, i, self.scratch_left[i]);
                output.set_f32(1, i, self.scratch_right[i]);
            }
            return;
        }

        let stretch = self.stretch_factor.get();
        let pitch_ratio = self.pitch_ratio();

        self.processor_left.push_input(&self.scratch_left[..size]);
        self.processor_right.push_input(&self.scratch_right[..size]);

        self.processor_left.process(stretch, pitch_ratio);
        self.processor_right.process(stretch, pitch_ratio);

        self.scratch_out_left[..size].fill(0.0);
        self.scratch_out_right[..size].fill(0.0);

        let left_count = self
            .processor_left
            .pop_output(&mut self.scratch_out_left[..size]);
        let right_count = self
            .processor_right
            .pop_output(&mut self.scratch_out_right[..size]);

        for i in 0..size {
            output.set_f32(
                0,
                i,
                if i < left_count {
                    self.scratch_out_left[i]
                } else {
                    0.0
                },
            );
            output.set_f32(
                1,
                i,
                if i < right_count {
                    self.scratch_out_right[i]
                } else {
                    0.0
                },
            );
        }
    }

    fn get_id(&self) -> u64 {
        const TIME_STRETCH_MARKER: u64 = 0xABC0_0000_0000_0000;
        self.source.get_id() ^ TIME_STRETCH_MARKER
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn route(&mut self, input: &SignalFrame, frequency: f64) -> SignalFrame {
        self.source.route(input, frequency)
    }

    fn footprint(&self) -> usize {
        std::mem::size_of::<Self>() + self.source.footprint()
    }

    fn allocate(&mut self) {
        self.source.allocate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PassthroughUnit;

    impl AudioUnit for PassthroughUnit {
        fn inputs(&self) -> usize {
            0
        }
        fn outputs(&self) -> usize {
            2
        }
        fn reset(&mut self) {}
        fn set_sample_rate(&mut self, _: f64) {}
        fn tick(&mut self, _: &[f32], output: &mut [f32]) {
            if output.len() >= 2 {
                output[0] = 0.5;
                output[1] = 0.5;
            }
        }
        fn process(&mut self, size: usize, _: &BufferRef, output: &mut BufferMut) {
            for i in 0..size {
                output.set_f32(0, i, 0.5);
                output.set_f32(1, i, 0.5);
            }
        }
        fn get_id(&self) -> u64 {
            12345
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
        fn route(&mut self, _: &SignalFrame, _: f64) -> SignalFrame {
            SignalFrame::new(2)
        }
        fn footprint(&self) -> usize {
            0
        }
    }

    impl Clone for PassthroughUnit {
        fn clone(&self) -> Self {
            PassthroughUnit
        }
    }

    #[test]
    fn test_phase_vocoder_creation() {
        let unit = TimeStretchUnit::new(Box::new(PassthroughUnit), 44100.0);
        assert_eq!(unit.algorithm(), TimeStretchAlgorithm::PhaseVocoder);
        assert_eq!(unit.outputs(), 2);
    }

    #[test]
    fn test_granular_creation() {
        let unit =
            TimeStretchUnit::with_granular(Box::new(PassthroughUnit), 44100.0, GrainSize::Medium);
        assert_eq!(unit.algorithm(), TimeStretchAlgorithm::Granular);
    }

    #[test]
    fn test_set_parameters() {
        let unit = TimeStretchUnit::new(Box::new(PassthroughUnit), 44100.0);

        unit.set_stretch_factor(2.0);
        assert!((unit.stretch_factor() - 2.0).abs() < 0.001);

        unit.set_pitch_cents(-200.0);
        assert!((unit.pitch_cents() - (-200.0)).abs() < 0.001);
    }

    #[test]
    fn test_parameter_clamping() {
        let unit = TimeStretchUnit::new(Box::new(PassthroughUnit), 44100.0);

        unit.set_stretch_factor(10.0);
        assert!((unit.stretch_factor() - 4.0).abs() < 0.001);

        unit.set_stretch_factor(0.1);
        assert!((unit.stretch_factor() - 0.25).abs() < 0.001);
    }

    #[test]
    fn test_passthrough_mode() {
        let mut unit = TimeStretchUnit::new(Box::new(PassthroughUnit), 44100.0);
        assert!(!unit.is_processing());

        let mut output = [0.0f32; 2];
        unit.tick(&[], &mut output);
    }

    #[test]
    fn test_enabled_flag() {
        let mut unit = TimeStretchUnit::new(Box::new(PassthroughUnit), 44100.0);

        unit.set_stretch_factor(2.0);
        assert!(unit.is_processing());

        unit.set_enabled(false);
        assert!(!unit.is_processing());

        unit.set_enabled(true);
        assert!(unit.is_processing());
    }

    #[test]
    fn test_clone() {
        let unit1 = TimeStretchUnit::new(Box::new(PassthroughUnit), 44100.0);
        unit1.set_stretch_factor(1.5);

        let unit2 = unit1.clone();
        assert!((unit2.stretch_factor() - 1.5).abs() < 0.001);

        unit1.set_stretch_factor(2.0);
        assert!((unit1.stretch_factor() - 2.0).abs() < 0.001);
        assert!((unit2.stretch_factor() - 1.5).abs() < 0.001);
    }
}
