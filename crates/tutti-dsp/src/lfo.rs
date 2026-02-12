//! Low Frequency Oscillator (LFO) node.

use tutti_core::Arc;
use tutti_core::AtomicFloat;
use tutti_core::{
    dsp::{Signal, DEFAULT_SR},
    AudioUnit, BufferMut, BufferRef, SignalFrame,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LfoShape {
    #[default]
    Sine,
    Triangle,
    Square,
    Sawtooth,
    SawtoothDown,
    Random,
    RandomSmooth,
}

impl LfoShape {
    #[inline]
    pub(crate) fn evaluate(&self, phase: f32) -> f32 {
        match self {
            LfoShape::Sine => (phase * core::f32::consts::TAU).sin(),
            LfoShape::Triangle => {
                let p = phase * 4.0;
                if p < 1.0 {
                    p
                } else if p < 3.0 {
                    2.0 - p
                } else {
                    p - 4.0
                }
            }
            LfoShape::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoShape::Sawtooth => phase * 2.0 - 1.0,
            LfoShape::SawtoothDown => 1.0 - phase * 2.0,
            LfoShape::Random | LfoShape::RandomSmooth => 0.0,
        }
    }

    pub fn all() -> &'static [LfoShape] {
        &[
            LfoShape::Sine,
            LfoShape::Triangle,
            LfoShape::Square,
            LfoShape::Sawtooth,
            LfoShape::SawtoothDown,
            LfoShape::Random,
            LfoShape::RandomSmooth,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            LfoShape::Sine => "Sine",
            LfoShape::Triangle => "Triangle",
            LfoShape::Square => "Square",
            LfoShape::Sawtooth => "Sawtooth",
            LfoShape::SawtoothDown => "Saw Down",
            LfoShape::Random => "Random",
            LfoShape::RandomSmooth => "Random (Smooth)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfoMode {
    FreeRunning,
    BeatSynced,
}

pub struct LfoNode {
    shape: LfoShape,
    mode: LfoMode,
    frequency: Arc<AtomicFloat>,
    depth: Arc<AtomicFloat>,
    phase_offset: Arc<AtomicFloat>,
    phase: f32,
    sample_rate: f64,
    random_state: RandomState,
}

#[derive(Debug, Clone)]
struct RandomState {
    current: f32,
    previous: f32,
    last_phase: f32,
    seed: u32,
}

impl Default for RandomState {
    fn default() -> Self {
        Self {
            current: 0.0,
            previous: 0.0,
            last_phase: 0.0,
            seed: 12345,
        }
    }
}

impl RandomState {
    fn next(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    fn update_for_phase(&mut self, phase: f32) {
        if phase < self.last_phase - 0.5 {
            self.previous = self.current;
            self.current = self.next();
        }
        self.last_phase = phase;
    }

    fn get_random(&self) -> f32 {
        self.current
    }

    fn get_random_smooth(&self, phase: f32) -> f32 {
        self.previous + (self.current - self.previous) * phase
    }
}

impl LfoNode {
    pub fn new(shape: LfoShape, frequency_hz: f32) -> Self {
        Self {
            shape,
            mode: LfoMode::FreeRunning,
            frequency: Arc::new(AtomicFloat::new(frequency_hz)),
            depth: Arc::new(AtomicFloat::new(1.0)),
            phase_offset: Arc::new(AtomicFloat::new(0.0)),
            phase: 0.0,
            sample_rate: DEFAULT_SR,
            random_state: RandomState::default(),
        }
    }

    pub fn new_beat_synced(shape: LfoShape, beats_per_cycle: f32) -> Self {
        Self {
            shape,
            mode: LfoMode::BeatSynced,
            frequency: Arc::new(AtomicFloat::new(beats_per_cycle)),
            depth: Arc::new(AtomicFloat::new(1.0)),
            phase_offset: Arc::new(AtomicFloat::new(0.0)),
            phase: 0.0,
            sample_rate: DEFAULT_SR,
            random_state: RandomState::default(),
        }
    }

    pub fn frequency(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.frequency)
    }

    pub fn depth(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.depth)
    }

    pub fn phase_offset(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.phase_offset)
    }

    pub fn set_frequency(&self, freq: f32) {
        self.frequency.set(freq);
    }

    pub fn set_depth(&self, depth: f32) {
        self.depth.set(depth.clamp(0.0, 1.0));
    }

    pub fn set_phase_offset(&self, offset: f32) {
        self.phase_offset.set(offset % 1.0);
    }

    #[inline]
    fn evaluate(&mut self, phase: f32) -> f32 {
        let depth = self.depth.get();

        match self.shape {
            LfoShape::Random => {
                self.random_state.update_for_phase(phase);
                self.random_state.get_random() * depth
            }
            LfoShape::RandomSmooth => {
                self.random_state.update_for_phase(phase);
                self.random_state.get_random_smooth(phase) * depth
            }
            _ => self.shape.evaluate(phase) * depth,
        }
    }
}

impl AudioUnit for LfoNode {
    fn inputs(&self) -> usize {
        match self.mode {
            LfoMode::FreeRunning => 0,
            LfoMode::BeatSynced => 1,
        }
    }

    fn outputs(&self) -> usize {
        1
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.random_state = RandomState::default();
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
    }

    #[inline]
    fn tick(&mut self, input: &[f32], output: &mut [f32]) {
        let phase_offset = self.phase_offset.get();

        let phase = match self.mode {
            LfoMode::FreeRunning => {
                let freq = self.frequency.get();
                self.phase += freq / self.sample_rate as f32;
                if self.phase >= 1.0 {
                    self.phase -= 1.0;
                }
                (self.phase + phase_offset) % 1.0
            }
            LfoMode::BeatSynced => {
                let beat = input[0];
                let beats_per_cycle = self.frequency.get();
                if beats_per_cycle > 0.0 {
                    ((beat / beats_per_cycle) + phase_offset) % 1.0
                } else {
                    phase_offset
                }
            }
        };

        output[0] = self.evaluate(phase);
    }

    fn process(&mut self, size: usize, input: &BufferRef, output: &mut BufferMut) {
        let phase_offset = self.phase_offset.get();

        match self.mode {
            LfoMode::FreeRunning => {
                let freq = self.frequency.get();
                let phase_increment = freq / self.sample_rate as f32;

                for i in 0..size {
                    let phase = (self.phase + phase_offset) % 1.0;
                    output.set_f32(0, i, self.evaluate(phase));

                    self.phase += phase_increment;
                    if self.phase >= 1.0 {
                        self.phase -= 1.0;
                    }
                }
            }
            LfoMode::BeatSynced => {
                let beats_per_cycle = self.frequency.get();

                for i in 0..size {
                    let beat = input.at_f32(0, i);
                    let phase = if beats_per_cycle > 0.0 {
                        ((beat / beats_per_cycle) + phase_offset) % 1.0
                    } else {
                        phase_offset
                    };
                    output.set_f32(0, i, self.evaluate(phase));
                }
            }
        }
    }

    fn get_id(&self) -> u64 {
        0x_4C46_4F5F_4E4F_4445
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }

    fn route(&mut self, input: &SignalFrame, _frequency: f64) -> SignalFrame {
        let mut output = SignalFrame::new(1);

        if self.mode == LfoMode::BeatSynced {
            if let Signal::Value(beat) = input.at(0) {
                let beats_per_cycle = self.frequency.get() as f64;
                let phase_offset = self.phase_offset.get() as f64;
                let phase = if beats_per_cycle > 0.0 {
                    ((beat / beats_per_cycle) + phase_offset) % 1.0
                } else {
                    phase_offset
                };
                let value = self.shape.evaluate(phase as f32) * self.depth.get();
                output.set(0, Signal::Value(value as f64));
            }
        } else {
            let phase_offset = self.phase_offset.get();
            let value = self.shape.evaluate(self.phase + phase_offset) * self.depth.get();
            output.set(0, Signal::Value(value as f64));
        }

        output
    }

    fn footprint(&self) -> usize {
        core::mem::size_of::<Self>()
    }
}

impl Clone for LfoNode {
    fn clone(&self) -> Self {
        Self {
            shape: self.shape,
            mode: self.mode,
            frequency: Arc::clone(&self.frequency),
            depth: Arc::clone(&self.depth),
            phase_offset: Arc::clone(&self.phase_offset),
            phase: self.phase,
            sample_rate: self.sample_rate,
            random_state: self.random_state.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lfo_shapes() {
        let sine_val = LfoShape::Sine.evaluate(0.25);
        assert!((sine_val - 1.0).abs() < 0.01);

        let square_val = LfoShape::Square.evaluate(0.25);
        assert_eq!(square_val, 1.0);

        let square_val2 = LfoShape::Square.evaluate(0.75);
        assert_eq!(square_val2, -1.0);

        let tri_val = LfoShape::Triangle.evaluate(0.25);
        assert!((tri_val - 1.0).abs() < 0.01);

        let saw_val = LfoShape::Sawtooth.evaluate(0.5);
        assert!((saw_val - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_free_running_lfo() {
        let mut lfo = LfoNode::new(LfoShape::Sine, 1.0);
        lfo.set_sample_rate(100.0);

        let mut output = [0.0f32];

        for _ in 0..25 {
            lfo.tick(&[], &mut output);
        }

        assert!(
            (output[0] - 1.0).abs() < 0.1,
            "Expected ~1.0, got {}",
            output[0]
        );
    }

    #[test]
    fn test_beat_synced_lfo() {
        let mut lfo = LfoNode::new_beat_synced(LfoShape::Sine, 4.0);

        let mut output = [0.0f32];

        lfo.tick(&[1.0], &mut output);
        assert!(
            (output[0] - 1.0).abs() < 0.01,
            "Expected 1.0, got {}",
            output[0]
        );

        lfo.tick(&[2.0], &mut output);
        assert!(
            (output[0] - 0.0).abs() < 0.01,
            "Expected 0.0, got {}",
            output[0]
        );
    }

    #[test]
    fn test_depth_control() {
        let mut lfo = LfoNode::new(LfoShape::Square, 1.0);
        lfo.set_depth(0.5);

        let mut output = [0.0f32];
        lfo.tick(&[], &mut output);

        assert!((output[0] - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_phase_offset() {
        let mut lfo = LfoNode::new(LfoShape::Sine, 1.0);
        lfo.set_sample_rate(100.0);
        lfo.set_phase_offset(0.25);

        let mut output = [0.0f32];
        lfo.tick(&[], &mut output);

        assert!(
            (output[0] - 1.0).abs() < 0.1,
            "Expected ~1.0, got {}",
            output[0]
        );
    }

    #[test]
    fn test_sawtooth_down_shape() {
        let val_start = LfoShape::SawtoothDown.evaluate(0.0);
        assert!((val_start - 1.0).abs() < 0.01);

        let val_mid = LfoShape::SawtoothDown.evaluate(0.5);
        assert!((val_mid - 0.0).abs() < 0.01);

        let val_end = LfoShape::SawtoothDown.evaluate(1.0);
        assert!((val_end - (-1.0)).abs() < 0.01);
    }

    #[test]
    fn test_lfo_reset() {
        let mut lfo = LfoNode::new(LfoShape::Sine, 1.0);
        lfo.set_sample_rate(100.0);

        let mut output = [0.0f32];
        for _ in 0..50 {
            lfo.tick(&[], &mut output);
        }

        lfo.reset();

        let mut output_after = [0.0f32];
        lfo.tick(&[], &mut output_after);
        assert!(
            output_after[0].abs() < 0.1,
            "After reset, LFO should start near zero"
        );
    }

    #[test]
    fn test_random_produces_different_values() {
        let mut lfo = LfoNode::new(LfoShape::Random, 1.0);
        lfo.set_sample_rate(100.0);

        let mut values = Vec::new();
        let mut output = [0.0f32];

        for _ in 0..500 {
            lfo.tick(&[], &mut output);
            values.push(output[0]);
        }

        let unique: std::collections::HashSet<u32> =
            values.iter().map(|v| (v * 1000.0) as u32).collect();

        assert!(
            unique.len() > 1,
            "Random LFO should produce different values, got {}",
            unique.len()
        );
    }
}
