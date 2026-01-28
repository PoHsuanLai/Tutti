//! Envelope follower node.

use std::sync::Arc;
use tutti_core::AtomicFloat;
use tutti_core::{
    dsp::{Signal, DEFAULT_SR},
    AudioUnit, BufferMut, BufferRef, SignalFrame,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnvelopeMode {
    #[default]
    Peak,
    Rms,
}

pub struct EnvelopeFollowerNode {
    attack_time: Arc<AtomicFloat>,
    release_time: Arc<AtomicFloat>,
    gain: Arc<AtomicFloat>,
    mode: EnvelopeMode,
    envelope: f32,
    rms_sum: f32,
    rms_count: usize,
    rms_window: usize,
    sample_rate: f64,
    attack_coeff: f32,
    release_coeff: f32,
    last_attack: f32,
    last_release: f32,
}

impl EnvelopeFollowerNode {
    pub fn new(attack_time: f32, release_time: f32) -> Self {
        Self {
            attack_time: Arc::new(AtomicFloat::new(attack_time)),
            release_time: Arc::new(AtomicFloat::new(release_time)),
            gain: Arc::new(AtomicFloat::new(1.0)),
            mode: EnvelopeMode::Peak,
            envelope: 0.0,
            rms_sum: 0.0,
            rms_count: 0,
            rms_window: 1024,
            sample_rate: DEFAULT_SR,
            attack_coeff: Self::time_to_coeff(attack_time, DEFAULT_SR),
            release_coeff: Self::time_to_coeff(release_time, DEFAULT_SR),
            last_attack: attack_time,
            last_release: release_time,
        }
    }

    pub fn new_rms(attack_time: f32, release_time: f32, window_ms: f32) -> Self {
        let mut node = Self::new(attack_time, release_time);
        node.mode = EnvelopeMode::Rms;
        node.rms_window = (window_ms * 0.001 * DEFAULT_SR as f32) as usize;
        node.rms_window = std::cmp::max(node.rms_window, 1);
        node
    }

    #[inline]
    fn time_to_coeff(time: f32, sample_rate: f64) -> f32 {
        if time <= 0.0 {
            1.0
        } else {
            (-1.0 / (time * sample_rate as f32)).exp()
        }
    }

    #[inline]
    fn update_coefficients(&mut self) {
        let attack = self.attack_time.get();
        let release = self.release_time.get();

        if (attack - self.last_attack).abs() > 0.0001 {
            self.attack_coeff = Self::time_to_coeff(attack, self.sample_rate);
            self.last_attack = attack;
        }

        if (release - self.last_release).abs() > 0.0001 {
            self.release_coeff = Self::time_to_coeff(release, self.sample_rate);
            self.last_release = release;
        }
    }

    pub fn attack_time(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.attack_time)
    }

    pub fn release_time(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.release_time)
    }

    pub fn gain(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.gain)
    }

    pub fn set_attack_time(&self, time: f32) {
        self.attack_time.set(time.max(0.0));
    }

    pub fn set_release_time(&self, time: f32) {
        self.release_time.set(time.max(0.0));
    }

    pub fn set_gain(&self, gain: f32) {
        self.gain.set(gain);
    }

    pub fn set_mode(&mut self, mode: EnvelopeMode) {
        self.mode = mode;
    }

    pub fn current_envelope(&self) -> f32 {
        self.envelope
    }

    #[inline]
    fn process_peak(&mut self, input: f32) -> f32 {
        let input_level = input.abs() * self.gain.get();

        if input_level > self.envelope {
            // Attack: envelope rises toward input
            self.envelope =
                self.attack_coeff * self.envelope + (1.0 - self.attack_coeff) * input_level;
        } else {
            // Release: envelope falls
            self.envelope =
                self.release_coeff * self.envelope + (1.0 - self.release_coeff) * input_level;
        }

        self.envelope
    }

    #[inline]
    fn process_rms(&mut self, input: f32) -> f32 {
        let input_level = input * self.gain.get();

        // Accumulate squared samples
        self.rms_sum += input_level * input_level;
        self.rms_count += 1;

        // Calculate RMS when window is full
        if self.rms_count >= self.rms_window {
            let rms = (self.rms_sum / self.rms_window as f32).sqrt();
            self.rms_sum = 0.0;
            self.rms_count = 0;

            // Apply smoothing to RMS value
            if rms > self.envelope {
                self.envelope = self.attack_coeff * self.envelope + (1.0 - self.attack_coeff) * rms;
            } else {
                self.envelope =
                    self.release_coeff * self.envelope + (1.0 - self.release_coeff) * rms;
            }
        }

        self.envelope
    }
}

impl AudioUnit for EnvelopeFollowerNode {
    fn inputs(&self) -> usize {
        1 // Audio input
    }

    fn outputs(&self) -> usize {
        1 // Envelope output
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
        self.rms_sum = 0.0;
        self.rms_count = 0;
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
        // Recalculate coefficients
        self.attack_coeff = Self::time_to_coeff(self.attack_time.get(), sample_rate);
        self.release_coeff = Self::time_to_coeff(self.release_time.get(), sample_rate);
        // Update RMS window size
        // Assume default 10ms window if using RMS mode
        if self.mode == EnvelopeMode::Rms {
            self.rms_window = (0.01 * sample_rate) as usize;
            self.rms_window = std::cmp::max(self.rms_window, 1);
        }
    }

    #[inline]
    fn tick(&mut self, input: &[f32], output: &mut [f32]) {
        self.update_coefficients();

        output[0] = match self.mode {
            EnvelopeMode::Peak => self.process_peak(input[0]),
            EnvelopeMode::Rms => self.process_rms(input[0]),
        };
    }

    fn process(&mut self, size: usize, input: &BufferRef, output: &mut BufferMut) {
        self.update_coefficients();

        match self.mode {
            EnvelopeMode::Peak => {
                for i in 0..size {
                    let out = self.process_peak(input.at_f32(0, i));
                    output.set_f32(0, i, out);
                }
            }
            EnvelopeMode::Rms => {
                for i in 0..size {
                    let out = self.process_rms(input.at_f32(0, i));
                    output.set_f32(0, i, out);
                }
            }
        }
    }

    fn get_id(&self) -> u64 {
        const ENVELOPE_FOLLOWER_ID: u64 = 0x_454E_565F_464F_4C4C; // "ENV_FOLL"
        ENVELOPE_FOLLOWER_ID
    }

    fn route(&mut self, input: &SignalFrame, _frequency: f64) -> SignalFrame {
        let mut output = SignalFrame::new(1);
        if let Signal::Value(val) = input.at(0) {
            // Rough approximation for routing
            output.set(0, Signal::Value(val.abs()));
        } else {
            output.set(0, Signal::Value(0.0));
        }
        output
    }

    fn footprint(&self) -> usize {
        core::mem::size_of::<Self>()
    }
}

impl Clone for EnvelopeFollowerNode {
    fn clone(&self) -> Self {
        Self {
            attack_time: Arc::clone(&self.attack_time),
            release_time: Arc::clone(&self.release_time),
            gain: Arc::clone(&self.gain),
            mode: self.mode,
            envelope: self.envelope,
            rms_sum: self.rms_sum,
            rms_count: self.rms_count,
            rms_window: self.rms_window,
            sample_rate: self.sample_rate,
            attack_coeff: self.attack_coeff,
            release_coeff: self.release_coeff,
            last_attack: self.last_attack,
            last_release: self.last_release,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_envelope_follower_creation() {
        let env = EnvelopeFollowerNode::new(0.01, 0.1);
        assert_eq!(env.inputs(), 1);
        assert_eq!(env.outputs(), 1);
        assert_eq!(env.current_envelope(), 0.0);
    }

    #[test]
    fn test_envelope_rises_on_signal() {
        let mut env = EnvelopeFollowerNode::new(0.001, 0.1); // Very fast attack
        env.set_sample_rate(44100.0);

        let mut output = [0.0f32];

        // Feed some audio samples
        for _ in 0..1000 {
            env.tick(&[0.5], &mut output);
        }

        // Envelope should have risen significantly
        assert!(output[0] > 0.3, "Envelope should rise, got {}", output[0]);
    }

    #[test]
    fn test_envelope_falls_on_silence() {
        let mut env = EnvelopeFollowerNode::new(0.001, 0.01); // Fast attack, fast release
        env.set_sample_rate(44100.0);

        let mut output = [0.0f32];

        // First, get envelope up
        for _ in 0..1000 {
            env.tick(&[0.8], &mut output);
        }
        let peak = output[0];

        // Now feed silence
        for _ in 0..2000 {
            env.tick(&[0.0], &mut output);
        }

        // Envelope should have fallen
        assert!(
            output[0] < peak * 0.5,
            "Envelope should fall, got {}",
            output[0]
        );
    }

    #[test]
    fn test_gain_control() {
        let mut env = EnvelopeFollowerNode::new(0.001, 0.1);
        env.set_sample_rate(44100.0);
        env.set_gain(2.0);

        let mut env2 = EnvelopeFollowerNode::new(0.001, 0.1);
        env2.set_sample_rate(44100.0);
        env2.set_gain(1.0);

        let mut output1 = [0.0f32];
        let mut output2 = [0.0f32];

        // Feed same signal to both
        for _ in 0..1000 {
            env.tick(&[0.5], &mut output1);
            env2.tick(&[0.5], &mut output2);
        }

        // Higher gain should result in higher envelope
        assert!(
            output1[0] > output2[0],
            "Higher gain should give higher envelope"
        );
    }

    #[test]
    fn test_rms_mode() {
        let mut env = EnvelopeFollowerNode::new_rms(0.001, 0.1, 10.0); // 10ms window

        let mut output = [0.0f32];

        // Feed some audio
        for i in 0..2000 {
            // Sine-like input
            let input = (i as f32 * 0.1).sin() * 0.5;
            env.tick(&[input], &mut output);
        }

        // RMS envelope should be non-zero
        assert!(output[0] > 0.0, "RMS envelope should be positive");
    }
}
