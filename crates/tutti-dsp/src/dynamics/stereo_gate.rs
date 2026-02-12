//! Stereo sidechain gate

use tutti_core::Arc;
use tutti_core::AtomicFloat;
use tutti_core::{dsp::DEFAULT_SR, AudioUnit, BufferMut, BufferRef, SignalFrame};

use super::utils::{amplitude_to_db, db_to_amplitude, time_to_coeff};

/// Stereo gate with stereo sidechain input
///
/// ## Inputs
/// - Port 0: Left audio
/// - Port 1: Right audio
/// - Port 2: Left sidechain
/// - Port 3: Right sidechain (optional)
///
/// ## Outputs
/// - Port 0: Gated left
/// - Port 1: Gated right
pub struct StereoSidechainGate {
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

impl StereoSidechainGate {
    pub fn new(threshold_db: f32, attack: f32, hold: f32, release: f32) -> Self {
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

    pub fn with_range(mut self, range_db: f32) -> Self {
        self.range_db = Arc::new(AtomicFloat::new(range_db.min(0.0)));
        self
    }

    pub fn threshold(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.threshold_db)
    }

    pub fn gate_level(&self) -> f32 {
        self.gate_level
    }

    pub fn is_open(&self) -> bool {
        self.gate_level > 0.5
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
}

impl AudioUnit for StereoSidechainGate {
    fn inputs(&self) -> usize {
        4
    }

    fn outputs(&self) -> usize {
        2
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

        let audio_l = input[0];
        let audio_r = if input.len() > 1 { input[1] } else { audio_l };
        let sc_l = if input.len() > 2 { input[2] } else { audio_l };
        let sc_r = if input.len() > 3 { input[3] } else { sc_l };

        let sc_level = sc_l.abs().max(sc_r.abs());
        let input_db = amplitude_to_db(sc_level);
        let threshold = self.threshold_db.get();

        self.envelope = sc_level;

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

        output[0] = audio_l * gain;
        output[1] = audio_r * gain;
    }

    fn process(&mut self, size: usize, input: &BufferRef, output: &mut BufferMut) {
        self.update_coefficients();

        let channels = input.channels();

        for i in 0..size {
            let audio_l = input.at_f32(0, i);
            let audio_r = if channels > 1 {
                input.at_f32(1, i)
            } else {
                audio_l
            };
            let sc_l = if channels > 2 {
                input.at_f32(2, i)
            } else {
                audio_l
            };
            let sc_r = if channels > 3 {
                input.at_f32(3, i)
            } else {
                sc_l
            };

            let sc_level = sc_l.abs().max(sc_r.abs());
            let input_db = amplitude_to_db(sc_level);
            let threshold = self.threshold_db.get();

            let gate_open = input_db >= threshold;

            if gate_open {
                self.hold_counter = self.hold_samples;
                self.gate_level =
                    self.attack_coeff * self.gate_level + (1.0 - self.attack_coeff) * 1.0;
            } else if self.hold_counter > 0 {
                self.hold_counter -= 1;
            } else {
                self.gate_level =
                    self.release_coeff * self.gate_level + (1.0 - self.release_coeff) * 0.0;
            }

            let range_linear = db_to_amplitude(self.range_db.get());
            let gain = range_linear + self.gate_level * (1.0 - range_linear);

            output.set_f32(0, i, audio_l * gain);
            output.set_f32(1, i, audio_r * gain);
        }
    }

    fn get_id(&self) -> u64 {
        0x5353_4347_4154
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }

    fn route(&mut self, input: &SignalFrame, _frequency: f64) -> SignalFrame {
        let mut output = SignalFrame::new(2);
        output.set(0, input.at(0));
        output.set(1, input.at(1));
        output
    }

    fn footprint(&self) -> usize {
        core::mem::size_of::<Self>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stereo_gate_starts_closed() {
        let gate = StereoSidechainGate::new(-30.0, 0.001, 0.01, 0.1);
        assert!(!gate.is_open());
        assert_eq!(gate.gate_level(), 0.0);
    }

    #[test]
    fn test_stereo_gate_opens_on_loud_sidechain() {
        let mut gate = StereoSidechainGate::new(-20.0, 0.0001, 0.01, 0.1);
        gate.set_sample_rate(44100.0);

        let mut output = [0.0f32; 2];

        for _ in 0..500 {
            gate.tick(&[0.5, 0.4, 0.9, 0.9], &mut output);
        }

        assert!(gate.is_open());
        assert!(output[0] > 0.3);
        assert!(output[1] > 0.2);
    }

    #[test]
    fn test_stereo_gate_closes_on_quiet_sidechain() {
        let mut gate = StereoSidechainGate::new(-20.0, 0.001, 0.001, 0.001).with_range(-60.0);
        gate.set_sample_rate(44100.0);

        let mut output = [0.0f32; 2];

        for _ in 0..500 {
            gate.tick(&[0.5, 0.5, 0.9, 0.9], &mut output);
        }
        assert!(gate.is_open());

        for _ in 0..2000 {
            gate.tick(&[0.5, 0.5, 0.01, 0.01], &mut output);
        }

        assert!(!gate.is_open());
        assert!(output[0] < 0.1);
        assert!(output[1] < 0.1);
    }

    #[test]
    fn test_stereo_gate_stereo_linking() {
        let mut gate = StereoSidechainGate::new(-20.0, 0.0001, 0.01, 0.1);
        gate.set_sample_rate(44100.0);

        let mut output = [0.0f32; 2];

        for _ in 0..500 {
            gate.tick(&[0.8, 0.3, 0.9, 0.9], &mut output);
        }

        let ratio = output[1] / output[0];
        assert!(
            (ratio - 0.3 / 0.8).abs() < 0.15,
            "Both channels should have same gate gain, ratio: {}",
            ratio
        );
    }

    #[test]
    fn test_stereo_gate_reset() {
        let mut gate = StereoSidechainGate::new(-20.0, 0.0001, 0.01, 0.1);
        gate.set_sample_rate(44100.0);

        let mut output = [0.0f32; 2];
        for _ in 0..500 {
            gate.tick(&[0.5, 0.5, 0.9, 0.9], &mut output);
        }
        assert!(gate.is_open());

        gate.reset();
        assert!(!gate.is_open());
        assert_eq!(gate.gate_level(), 0.0);
    }
}

impl Clone for StereoSidechainGate {
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
