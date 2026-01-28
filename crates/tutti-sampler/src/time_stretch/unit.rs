//! Time-stretching audio unit wrapper.

use std::sync::Arc;
use tutti_core::{AudioUnit, BufferMut, BufferRef, SignalFrame};

use super::phase_vocoder::PhaseVocoderProcessor;
use super::types::{AtomicF32, FftSize, TimeStretchAlgorithm};

/// Real-time time-stretching and pitch-shifting unit.
pub struct TimeStretchUnit {
    /// The source audio unit
    source: Box<dyn AudioUnit>,

    /// Phase vocoder processor (stereo: left channel)
    processor_left: PhaseVocoderProcessor,
    /// Phase vocoder processor (stereo: right channel)
    processor_right: PhaseVocoderProcessor,

    /// Time stretch factor (atomic for lock-free updates)
    /// 1.0 = normal, >1 = slower, <1 = faster
    stretch_factor: Arc<AtomicF32>,

    /// Pitch shift in cents (atomic for lock-free updates)
    /// 0 = no shift, +100 = up 1 semitone, -100 = down 1 semitone
    pitch_cents: Arc<AtomicF32>,

    /// Whether time-stretching is enabled
    enabled: bool,

    /// Current algorithm (for future granular support)
    _algorithm: TimeStretchAlgorithm,

    /// Sample rate
    sample_rate: f64,

    /// Intermediate buffer for source output
    source_buffer: Vec<f32>,
}

impl TimeStretchUnit {
    /// Create a new time-stretch unit wrapping a source
    ///
    /// # Arguments
    ///
    /// * `source` - The source AudioUnit to stretch (must have 2 outputs for stereo)
    /// * `sample_rate` - Audio sample rate in Hz
    pub fn new(source: Box<dyn AudioUnit>, sample_rate: f64) -> Self {
        Self::with_fft_size(source, sample_rate, FftSize::default())
    }

    /// Create a new time-stretch unit with custom FFT size
    ///
    /// # Arguments
    ///
    /// * `source` - The source AudioUnit to stretch
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `fft_size` - FFT size preset (affects latency/quality trade-off)
    pub fn with_fft_size(source: Box<dyn AudioUnit>, sample_rate: f64, fft_size: FftSize) -> Self {
        Self {
            source,
            processor_left: PhaseVocoderProcessor::new(fft_size, sample_rate),
            processor_right: PhaseVocoderProcessor::new(fft_size, sample_rate),
            stretch_factor: Arc::new(AtomicF32::new(1.0)),
            pitch_cents: Arc::new(AtomicF32::new(0.0)),
            enabled: true,
            _algorithm: TimeStretchAlgorithm::PhaseLocked,
            sample_rate,
            source_buffer: vec![0.0; 2], // Stereo output
        }
    }

    /// Set the time stretch factor
    ///
    /// * 1.0 = normal speed
    /// * 2.0 = half speed (stretched to 2x duration)
    /// * 0.5 = double speed (compressed to half duration)
    ///
    /// Range: 0.25 to 4.0
    pub fn set_stretch_factor(&self, factor: f32) {
        let clamped = factor.clamp(0.25, 4.0);
        self.stretch_factor.store(clamped);
    }

    /// Get the current stretch factor
    pub fn stretch_factor(&self) -> f32 {
        self.stretch_factor.load()
    }

    /// Get a clone of the stretch factor Arc for external control
    pub fn stretch_factor_arc(&self) -> Arc<AtomicF32> {
        Arc::clone(&self.stretch_factor)
    }

    /// Set the pitch shift in cents
    ///
    /// * 0 = no shift
    /// * +100 = up 1 semitone
    /// * +1200 = up 1 octave
    /// * -100 = down 1 semitone
    ///
    /// Range: -2400 to +2400 (±2 octaves)
    pub fn set_pitch_cents(&self, cents: f32) {
        let clamped = cents.clamp(-2400.0, 2400.0);
        self.pitch_cents.store(clamped);
    }

    /// Get the current pitch shift in cents
    pub fn pitch_cents(&self) -> f32 {
        self.pitch_cents.load()
    }

    /// Get a clone of the pitch cents Arc for external control
    pub fn pitch_cents_arc(&self) -> Arc<AtomicF32> {
        Arc::clone(&self.pitch_cents)
    }

    /// Enable or disable time-stretching
    ///
    /// When disabled, audio passes through unchanged (lower CPU usage).
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if time-stretching is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Check if any processing is actually active
    ///
    /// Returns false if stretch=1.0 and pitch=0.0 (passthrough mode)
    pub fn is_processing(&self) -> bool {
        if !self.enabled {
            return false;
        }
        let stretch = self.stretch_factor.load();
        let pitch = self.pitch_cents.load();
        (stretch - 1.0).abs() > 0.001 || pitch.abs() > 0.5
    }

    /// Get the processing latency in samples
    pub fn latency_samples(&self) -> usize {
        self.processor_left.latency_samples()
    }

    /// Get the FFT size
    pub fn fft_size(&self) -> usize {
        self.processor_left.fft_size()
    }

    /// Get access to the source unit
    pub fn source(&self) -> &dyn AudioUnit {
        &*self.source
    }

    /// Get mutable access to the source unit
    pub fn source_mut(&mut self) -> &mut dyn AudioUnit {
        &mut *self.source
    }

    /// Calculate pitch shift ratio from cents
    #[inline]
    fn pitch_ratio(&self) -> f32 {
        2.0_f32.powf(self.pitch_cents.load() / 1200.0)
    }
}

impl Clone for TimeStretchUnit {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            processor_left: self.processor_left.clone(),
            processor_right: self.processor_right.clone(),
            stretch_factor: Arc::new(AtomicF32::new(self.stretch_factor.load())),
            pitch_cents: Arc::new(AtomicF32::new(self.pitch_cents.load())),
            enabled: self.enabled,
            _algorithm: self._algorithm,
            sample_rate: self.sample_rate,
            source_buffer: self.source_buffer.clone(),
        }
    }
}

impl AudioUnit for TimeStretchUnit {
    fn inputs(&self) -> usize {
        self.source.inputs()
    }

    fn outputs(&self) -> usize {
        2 // Always stereo output
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
        // Get source output
        self.source.tick(input, &mut self.source_buffer);

        if !self.is_processing() {
            // Passthrough mode
            if output.len() >= 2 {
                output[0] = self.source_buffer[0];
                output[1] = if self.source_buffer.len() >= 2 {
                    self.source_buffer[1]
                } else {
                    self.source_buffer[0]
                };
            }
            return;
        }

        // Get parameters
        let stretch = self.stretch_factor.load();
        let pitch_ratio = self.pitch_ratio();

        // Push source output to processors
        self.processor_left.push_input(&[self.source_buffer[0]]);
        let right_sample = if self.source_buffer.len() >= 2 {
            self.source_buffer[1]
        } else {
            self.source_buffer[0]
        };
        self.processor_right.push_input(&[right_sample]);

        // Process
        self.processor_left.process(stretch, pitch_ratio);
        self.processor_right.process(stretch, pitch_ratio);

        // Pop output
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
        // Collect source samples
        let mut source_left = vec![0.0f32; size];
        let mut source_right = vec![0.0f32; size];

        // Process source
        let has_inputs = self.source.inputs() > 0;
        let mut input_sample = [0.0f32];
        for i in 0..size {
            if has_inputs {
                input_sample[0] = input.at_f32(0, i);
                self.source.tick(&input_sample, &mut self.source_buffer);
            } else {
                self.source.tick(&[], &mut self.source_buffer);
            }
            source_left[i] = self.source_buffer[0];
            source_right[i] = if self.source_buffer.len() >= 2 {
                self.source_buffer[1]
            } else {
                self.source_buffer[0]
            };
        }

        if !self.is_processing() {
            // Passthrough mode
            for i in 0..size {
                output.set_f32(0, i, source_left[i]);
                output.set_f32(1, i, source_right[i]);
            }
            return;
        }

        // Get parameters
        let stretch = self.stretch_factor.load();
        let pitch_ratio = self.pitch_ratio();

        // Push all source samples to processors
        self.processor_left.push_input(&source_left);
        self.processor_right.push_input(&source_right);

        // Process
        self.processor_left.process(stretch, pitch_ratio);
        self.processor_right.process(stretch, pitch_ratio);

        // Pop output
        let mut out_left = vec![0.0f32; size];
        let mut out_right = vec![0.0f32; size];

        let left_count = self.processor_left.pop_output(&mut out_left);
        let right_count = self.processor_right.pop_output(&mut out_right);

        // Write to output buffer
        for i in 0..size {
            output.set_f32(0, i, if i < left_count { out_left[i] } else { 0.0 });
            output.set_f32(1, i, if i < right_count { out_right[i] } else { 0.0 });
        }
    }

    fn get_id(&self) -> u64 {
        // Combine source ID with a marker for time-stretch
        const TIME_STRETCH_MARKER: u64 = 0xABC0_0000_0000_0000;
        self.source.get_id() ^ TIME_STRETCH_MARKER
    }

    fn route(&mut self, input: &SignalFrame, frequency: f64) -> SignalFrame {
        self.source.route(input, frequency)
    }

    fn footprint(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.source.footprint()
            + self.processor_left.fft_size() * 8 * 2 // Approximate processor footprint
    }

    fn allocate(&mut self) {
        self.source.allocate();
        // Processors are already allocated during construction
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Simple passthrough unit for testing
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
    fn test_creation() {
        let source = Box::new(PassthroughUnit);
        let unit = TimeStretchUnit::new(source, 44100.0);

        assert_eq!(unit.outputs(), 2);
        assert!((unit.stretch_factor() - 1.0).abs() < 0.001);
        assert!(unit.pitch_cents().abs() < 0.001);
    }

    #[test]
    fn test_set_parameters() {
        let source = Box::new(PassthroughUnit);
        let unit = TimeStretchUnit::new(source, 44100.0);

        unit.set_stretch_factor(2.0);
        assert!((unit.stretch_factor() - 2.0).abs() < 0.001);

        unit.set_pitch_cents(-200.0);
        assert!((unit.pitch_cents() - (-200.0)).abs() < 0.001);
    }

    #[test]
    fn test_parameter_clamping() {
        let source = Box::new(PassthroughUnit);
        let unit = TimeStretchUnit::new(source, 44100.0);

        unit.set_stretch_factor(10.0);
        assert!((unit.stretch_factor() - 4.0).abs() < 0.001);

        unit.set_stretch_factor(0.1);
        assert!((unit.stretch_factor() - 0.25).abs() < 0.001);

        unit.set_pitch_cents(5000.0);
        assert!((unit.pitch_cents() - 2400.0).abs() < 0.001);
    }

    #[test]
    fn test_passthrough_mode() {
        let source = Box::new(PassthroughUnit);
        let mut unit = TimeStretchUnit::new(source, 44100.0);

        // With default parameters, should be passthrough
        assert!(!unit.is_processing());

        let mut output = [0.0f32; 2];
        unit.tick(&[], &mut output);

        // After warming up the processor, we should get output
        // Note: There's latency, so first samples may be zero
    }

    #[test]
    fn test_enabled_flag() {
        let source = Box::new(PassthroughUnit);
        let mut unit = TimeStretchUnit::new(source, 44100.0);

        unit.set_stretch_factor(2.0);
        assert!(unit.is_processing());

        unit.set_enabled(false);
        assert!(!unit.is_processing());

        unit.set_enabled(true);
        assert!(unit.is_processing());
    }

    #[test]
    fn test_arc_access() {
        let source = Box::new(PassthroughUnit);
        let unit = TimeStretchUnit::new(source, 44100.0);

        let stretch_arc = unit.stretch_factor_arc();
        let pitch_arc = unit.pitch_cents_arc();

        // Modify through Arc
        stretch_arc.store(1.5);
        pitch_arc.store(-100.0);

        // Verify changes visible through unit
        assert!((unit.stretch_factor() - 1.5).abs() < 0.001);
        assert!((unit.pitch_cents() - (-100.0)).abs() < 0.001);
    }

    #[test]
    fn test_clone() {
        let source = Box::new(PassthroughUnit);
        let unit1 = TimeStretchUnit::new(source, 44100.0);
        unit1.set_stretch_factor(1.5);
        unit1.set_pitch_cents(-200.0);

        let unit2 = unit1.clone();

        assert!((unit2.stretch_factor() - 1.5).abs() < 0.001);
        assert!((unit2.pitch_cents() - (-200.0)).abs() < 0.001);

        // Clones should have independent parameters
        unit1.set_stretch_factor(2.0);
        assert!((unit1.stretch_factor() - 2.0).abs() < 0.001);
        assert!((unit2.stretch_factor() - 1.5).abs() < 0.001);
    }
}
