//! Sidechain gate (mono)

use tutti_core::Arc;
use tutti_core::AtomicFloat;
use tutti_core::{dsp::DEFAULT_SR, AudioUnit, BufferMut, BufferRef, SignalFrame};

use super::utils::{amplitude_to_db, db_to_amplitude, time_to_coeff};

/// Gate with external sidechain input
///
/// Uses sidechain signal to open/close the gate on the main audio.
/// Useful for tightening drums, removing bleed, or creative effects.
///
/// ## Inputs
/// - Port 0: Audio signal to gate
/// - Port 1: Sidechain signal (for detection)
///
/// ## Outputs
/// - Port 0: Gated audio
pub struct SidechainGate {
    threshold_db: Arc<AtomicFloat>,
    attack: Arc<AtomicFloat>,
    hold: Arc<AtomicFloat>,
    release: Arc<AtomicFloat>,
    range_db: Arc<AtomicFloat>,

    envelope: f32,
    gate_level: f32,
    hold_counter: usize,
    sample_rate: f64,

    attack_coeff: f32,
    release_coeff: f32,
    hold_samples: usize,
    last_attack: f32,
    last_release: f32,
    last_hold: f32,
}

impl SidechainGate {
    /// Create a new gate. Prefer [`SidechainGate::builder()`].
    pub(crate) fn new(threshold_db: f32, attack: f32, hold: f32, release: f32) -> Self {
        Self {
            threshold_db: Arc::new(AtomicFloat::new(threshold_db)),
            attack: Arc::new(AtomicFloat::new(attack)),
            hold: Arc::new(AtomicFloat::new(hold)),
            release: Arc::new(AtomicFloat::new(release)),
            range_db: Arc::new(AtomicFloat::new(-80.0)),

            envelope: 0.0,
            gate_level: 0.0,
            hold_counter: 0,
            sample_rate: DEFAULT_SR,

            attack_coeff: time_to_coeff(attack, DEFAULT_SR),
            release_coeff: time_to_coeff(release, DEFAULT_SR),
            hold_samples: (hold * DEFAULT_SR as f32) as usize,
            last_attack: attack,
            last_release: release,
            last_hold: hold,
        }
    }

    /// Create a builder for configuring a gate
    pub fn builder() -> SidechainGateBuilder {
        SidechainGateBuilder::default()
    }

    pub fn with_range(mut self, range_db: f32) -> Self {
        self.range_db = Arc::new(AtomicFloat::new(range_db.min(0.0)));
        self
    }

    pub fn threshold(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.threshold_db)
    }

    pub fn attack_time(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.attack)
    }

    pub fn hold_time(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.hold)
    }

    pub fn release_time(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.release)
    }

    pub fn range(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.range_db)
    }

    pub fn is_open(&self) -> bool {
        self.gate_level > 0.5
    }

    pub fn gate_level(&self) -> f32 {
        self.gate_level
    }

    #[inline]
    fn update_coefficients(&mut self) {
        let attack = self.attack.get();
        let release = self.release.get();
        let hold = self.hold.get();

        if (attack - self.last_attack).abs() > 0.00001 {
            self.attack_coeff = time_to_coeff(attack, self.sample_rate);
            self.last_attack = attack;
        }

        if (release - self.last_release).abs() > 0.00001 {
            self.release_coeff = time_to_coeff(release, self.sample_rate);
            self.last_release = release;
        }

        if (hold - self.last_hold).abs() > 0.00001 {
            self.hold_samples = (hold * self.sample_rate as f32) as usize;
            self.last_hold = hold;
        }
    }

    #[inline]
    fn process_sample(&mut self, audio: f32, sidechain: f32) -> f32 {
        let input_level = sidechain.abs();
        let input_db = amplitude_to_db(input_level);
        let threshold = self.threshold_db.get();

        self.envelope = input_level;

        let gate_open = input_db >= threshold;

        if gate_open {
            self.hold_counter = self.hold_samples;
            self.gate_level = self.attack_coeff * self.gate_level + (1.0 - self.attack_coeff) * 1.0;
        } else if self.hold_counter > 0 {
            self.hold_counter -= 1;
        } else {
            self.gate_level =
                self.release_coeff * self.gate_level + (1.0 - self.release_coeff) * 0.0;
        }

        let range_linear = db_to_amplitude(self.range_db.get());
        let gain = range_linear + self.gate_level * (1.0 - range_linear);

        audio * gain
    }
}

impl AudioUnit for SidechainGate {
    fn inputs(&self) -> usize {
        2
    }

    fn outputs(&self) -> usize {
        1
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
        self.gate_level = 0.0;
        self.hold_counter = 0;
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
        self.attack_coeff = time_to_coeff(self.attack.get(), sample_rate);
        self.release_coeff = time_to_coeff(self.release.get(), sample_rate);
        self.hold_samples = (self.hold.get() * sample_rate as f32) as usize;
    }

    #[inline]
    fn tick(&mut self, input: &[f32], output: &mut [f32]) {
        self.update_coefficients();
        let audio = input[0];
        let sidechain = if input.len() > 1 { input[1] } else { audio };
        output[0] = self.process_sample(audio, sidechain);
    }

    fn process(&mut self, size: usize, input: &BufferRef, output: &mut BufferMut) {
        self.update_coefficients();

        let has_sidechain = input.channels() > 1;

        for i in 0..size {
            let audio = input.at_f32(0, i);
            let sidechain = if has_sidechain {
                input.at_f32(1, i)
            } else {
                audio
            };
            output.set_f32(0, i, self.process_sample(audio, sidechain));
        }
    }

    fn get_id(&self) -> u64 {
        0x5343_4741_5445
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }

    fn route(&mut self, input: &SignalFrame, _frequency: f64) -> SignalFrame {
        let mut output = SignalFrame::new(1);
        output.set(0, input.at(0));
        output
    }

    fn footprint(&self) -> usize {
        core::mem::size_of::<Self>()
    }
}

impl Clone for SidechainGate {
    fn clone(&self) -> Self {
        Self {
            threshold_db: Arc::clone(&self.threshold_db),
            attack: Arc::clone(&self.attack),
            hold: Arc::clone(&self.hold),
            release: Arc::clone(&self.release),
            range_db: Arc::clone(&self.range_db),
            envelope: self.envelope,
            gate_level: self.gate_level,
            hold_counter: self.hold_counter,
            sample_rate: self.sample_rate,
            attack_coeff: self.attack_coeff,
            release_coeff: self.release_coeff,
            hold_samples: self.hold_samples,
            last_attack: self.last_attack,
            last_release: self.last_release,
            last_hold: self.last_hold,
        }
    }
}

/// Builder for configuring a SidechainGate with fluent API.
#[derive(Clone, Debug)]
pub struct SidechainGateBuilder {
    threshold_db: f32,
    attack_seconds: f32,
    hold_seconds: f32,
    release_seconds: f32,
    range_db: f32,
}

impl Default for SidechainGateBuilder {
    fn default() -> Self {
        Self {
            threshold_db: -30.0,
            attack_seconds: 0.001,
            hold_seconds: 0.01,
            release_seconds: 0.1,
            range_db: -80.0,
        }
    }
}

impl SidechainGateBuilder {
    /// Set the threshold in decibels (-60.0 to -10.0 typical)
    pub fn threshold_db(mut self, db: f32) -> Self {
        self.threshold_db = db;
        self
    }

    /// Set the attack time in seconds (0.0001 to 0.01 typical)
    pub fn attack_seconds(mut self, seconds: f32) -> Self {
        self.attack_seconds = seconds.max(0.0);
        self
    }

    /// Set the hold time in seconds (0.001 to 0.1 typical)
    pub fn hold_seconds(mut self, seconds: f32) -> Self {
        self.hold_seconds = seconds.max(0.0);
        self
    }

    /// Set the release time in seconds (0.01 to 1.0 typical)
    pub fn release_seconds(mut self, seconds: f32) -> Self {
        self.release_seconds = seconds.max(0.0);
        self
    }

    /// Set the range (depth) in decibels (must be <= 0.0)
    pub fn range_db(mut self, db: f32) -> Self {
        self.range_db = db.min(0.0);
        self
    }

    /// Build the configured SidechainGate
    pub fn build(self) -> SidechainGate {
        SidechainGate::new(
            self.threshold_db,
            self.attack_seconds,
            self.hold_seconds,
            self.release_seconds,
        )
        .with_range(self.range_db)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gate_starts_closed() {
        let gate = SidechainGate::new(-30.0, 0.001, 0.01, 0.1);
        assert!(!gate.is_open());
        assert_eq!(gate.gate_level(), 0.0);
    }

    #[test]
    fn test_gate_opens_on_loud_sidechain() {
        let mut gate = SidechainGate::new(-20.0, 0.0001, 0.01, 0.1);
        gate.set_sample_rate(44100.0);

        let mut output = [0.0f32];

        for _ in 0..500 {
            gate.tick(&[0.5, 0.9], &mut output);
        }

        assert!(gate.is_open());
        assert!(output[0] > 0.3);
    }

    #[test]
    fn test_gate_closes_on_quiet_sidechain() {
        let mut gate = SidechainGate::new(-20.0, 0.001, 0.001, 0.001).with_range(-60.0);
        gate.set_sample_rate(44100.0);

        let mut output = [0.0f32];

        for _ in 0..500 {
            gate.tick(&[0.5, 0.9], &mut output);
        }
        assert!(gate.is_open());

        for _ in 0..2000 {
            gate.tick(&[0.5, 0.01], &mut output);
        }

        assert!(!gate.is_open());
        assert!(output[0] < 0.1);
    }

    #[test]
    fn test_gate_builder_validates_range() {
        let gate = SidechainGate::builder().range_db(10.0).build();
        assert_eq!(gate.range().get(), 0.0);
    }

    #[test]
    fn test_gate_range_attenuates_rather_than_mutes() {
        let mut gate = SidechainGate::new(-20.0, 0.001, 0.001, 0.001).with_range(-12.0);
        gate.set_sample_rate(44100.0);

        let mut output = [0.0f32];

        for _ in 0..2000 {
            gate.tick(&[0.5, 0.01], &mut output);
        }

        assert!(!gate.is_open());
        assert!(output[0] > 0.05, "With -12dB range, signal should be attenuated not muted: {}", output[0]);
        assert!(output[0] < 0.5);
    }

    #[test]
    fn test_gate_reset() {
        let mut gate = SidechainGate::new(-20.0, 0.0001, 0.01, 0.1);
        gate.set_sample_rate(44100.0);

        let mut output = [0.0f32];
        for _ in 0..500 {
            gate.tick(&[0.5, 0.9], &mut output);
        }
        assert!(gate.is_open());

        gate.reset();
        assert!(!gate.is_open());
        assert_eq!(gate.gate_level(), 0.0);
    }
}
