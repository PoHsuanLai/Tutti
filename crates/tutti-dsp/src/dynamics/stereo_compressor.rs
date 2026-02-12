//! Stereo sidechain compressor

use tutti_core::Arc;
use tutti_core::AtomicFloat;
use tutti_core::{dsp::DEFAULT_SR, AudioUnit, BufferMut, BufferRef, SignalFrame};

use super::utils::{amplitude_to_db, db_to_amplitude, time_to_coeff};

/// Stereo compressor with stereo sidechain input
///
/// Links both channels for consistent stereo imaging while using
/// external sidechain for detection.
///
/// ## Inputs
/// - Port 0: Left audio
/// - Port 1: Right audio
/// - Port 2: Left sidechain (or mono sidechain)
/// - Port 3: Right sidechain (optional, uses left if not connected)
///
/// ## Outputs
/// - Port 0: Compressed left
/// - Port 1: Compressed right
pub struct StereoSidechainCompressor {
    threshold_db: Arc<AtomicFloat>,
    ratio: Arc<AtomicFloat>,
    attack: Arc<AtomicFloat>,
    release: Arc<AtomicFloat>,
    makeup_db: Arc<AtomicFloat>,
    knee_db: Arc<AtomicFloat>,

    envelope: f32,
    gain_reduction: f32,
    sample_rate: f64,

    attack_coeff: f32,
    release_coeff: f32,
    last_attack: f32,
    last_release: f32,
}

impl StereoSidechainCompressor {
    pub fn new(threshold_db: f32, ratio: f32, attack: f32, release: f32) -> Self {
        Self {
            threshold_db: Arc::new(AtomicFloat::new(threshold_db)),
            ratio: Arc::new(AtomicFloat::new(ratio.max(1.0))),
            attack: Arc::new(AtomicFloat::new(attack)),
            release: Arc::new(AtomicFloat::new(release)),
            makeup_db: Arc::new(AtomicFloat::new(0.0)),
            knee_db: Arc::new(AtomicFloat::new(0.0)),

            envelope: 0.0,
            gain_reduction: 0.0,
            sample_rate: DEFAULT_SR,

            attack_coeff: time_to_coeff(attack, DEFAULT_SR),
            release_coeff: time_to_coeff(release, DEFAULT_SR),
            last_attack: attack,
            last_release: release,
        }
    }

    pub fn with_soft_knee(mut self, knee_db: f32) -> Self {
        self.knee_db = Arc::new(AtomicFloat::new(knee_db.max(0.0)));
        self
    }

    pub fn with_makeup(mut self, makeup_db: f32) -> Self {
        self.makeup_db = Arc::new(AtomicFloat::new(makeup_db));
        self
    }

    pub fn threshold(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.threshold_db)
    }

    pub fn ratio(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.ratio)
    }

    pub fn attack_time(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.attack)
    }

    pub fn release_time(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.release)
    }

    pub fn makeup_gain(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.makeup_db)
    }

    pub fn gain_reduction_db(&self) -> f32 {
        self.gain_reduction
    }

    #[inline]
    fn update_coefficients(&mut self) {
        let attack = self.attack.get();
        let release = self.release.get();

        if (attack - self.last_attack).abs() > 0.00001 {
            self.attack_coeff = time_to_coeff(attack, self.sample_rate);
            self.last_attack = attack;
        }

        if (release - self.last_release).abs() > 0.00001 {
            self.release_coeff = time_to_coeff(release, self.sample_rate);
            self.last_release = release;
        }
    }

    #[inline]
    fn compute_gain_reduction(&self, input_db: f32) -> f32 {
        let threshold = self.threshold_db.get();
        let ratio = self.ratio.get();
        let knee = self.knee_db.get();

        if knee <= 0.0 {
            let over_db = (input_db - threshold).max(0.0);
            over_db * (1.0 - 1.0 / ratio)
        } else {
            let half_knee = knee / 2.0;
            let below = threshold - half_knee;
            let above = threshold + half_knee;

            if input_db <= below {
                0.0
            } else if input_db >= above {
                let over_db = input_db - threshold;
                over_db * (1.0 - 1.0 / ratio)
            } else {
                let x = input_db - below;
                let slope = (1.0 - 1.0 / ratio) / (2.0 * knee);
                slope * x * x
            }
        }
    }
}

impl AudioUnit for StereoSidechainCompressor {
    fn inputs(&self) -> usize {
        4
    }

    fn outputs(&self) -> usize {
        2
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
        self.gain_reduction = 0.0;
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
        self.attack_coeff = time_to_coeff(self.attack.get(), sample_rate);
        self.release_coeff = time_to_coeff(self.release.get(), sample_rate);
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

        let target_reduction = self.compute_gain_reduction(input_db);

        if target_reduction > self.gain_reduction {
            self.gain_reduction = self.attack_coeff * self.gain_reduction
                + (1.0 - self.attack_coeff) * target_reduction;
        } else {
            self.gain_reduction = self.release_coeff * self.gain_reduction
                + (1.0 - self.release_coeff) * target_reduction;
        }

        self.envelope = sc_level;

        let gain = db_to_amplitude(-self.gain_reduction + self.makeup_db.get());
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
            let target_reduction = self.compute_gain_reduction(input_db);

            if target_reduction > self.gain_reduction {
                self.gain_reduction = self.attack_coeff * self.gain_reduction
                    + (1.0 - self.attack_coeff) * target_reduction;
            } else {
                self.gain_reduction = self.release_coeff * self.gain_reduction
                    + (1.0 - self.release_coeff) * target_reduction;
            }

            let gain = db_to_amplitude(-self.gain_reduction + self.makeup_db.get());
            output.set_f32(0, i, audio_l * gain);
            output.set_f32(1, i, audio_r * gain);
        }
    }

    fn get_id(&self) -> u64 {
        0x5353_4343_4F4D
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

impl Clone for StereoSidechainCompressor {
    fn clone(&self) -> Self {
        Self {
            threshold_db: Arc::clone(&self.threshold_db),
            ratio: Arc::clone(&self.ratio),
            attack: Arc::clone(&self.attack),
            release: Arc::clone(&self.release),
            makeup_db: Arc::clone(&self.makeup_db),
            knee_db: Arc::clone(&self.knee_db),
            envelope: self.envelope,
            gain_reduction: self.gain_reduction,
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
    fn test_stereo_compressor_reduces_on_loud_sidechain() {
        let mut comp = StereoSidechainCompressor::new(-20.0, 4.0, 0.0001, 0.1);
        comp.set_sample_rate(44100.0);

        let mut output = [0.0f32, 0.0f32];

        for _ in 0..1000 {
            comp.tick(&[0.5, 0.5, 0.9, 0.9], &mut output);
        }

        assert!((output[0] - output[1]).abs() < 0.001);
        assert!(output[0] < 0.5);
    }

    #[test]
    fn test_stereo_compressor_no_reduction_below_threshold() {
        let mut comp = StereoSidechainCompressor::new(-10.0, 4.0, 0.001, 0.1);
        comp.set_sample_rate(44100.0);

        let mut output = [0.0f32; 2];

        for _ in 0..1000 {
            comp.tick(&[0.5, 0.5, 0.1, 0.1], &mut output);
        }

        assert!(comp.gain_reduction_db() < 1.0);
    }

    #[test]
    fn test_stereo_compressor_soft_knee_differs_from_hard_knee() {
        let mut hard = StereoSidechainCompressor::new(-20.0, 4.0, 0.0001, 0.1);
        hard.set_sample_rate(44100.0);

        let mut soft = StereoSidechainCompressor::new(-20.0, 4.0, 0.0001, 0.1).with_soft_knee(12.0);
        soft.set_sample_rate(44100.0);

        let mut hard_out = [0.0f32; 2];
        let mut soft_out = [0.0f32; 2];

        for _ in 0..1000 {
            hard.tick(&[0.5, 0.5, 0.15, 0.15], &mut hard_out);
            soft.tick(&[0.5, 0.5, 0.15, 0.15], &mut soft_out);
        }

        assert!(
            (hard_out[0] - soft_out[0]).abs() > 0.001,
            "Soft knee should produce different output near threshold: hard={}, soft={}",
            hard_out[0],
            soft_out[0]
        );
    }

    #[test]
    fn test_stereo_compressor_makeup_gain() {
        let mut comp = StereoSidechainCompressor::new(-20.0, 4.0, 0.0001, 0.1).with_makeup(6.0);
        comp.set_sample_rate(44100.0);

        let mut comp_no_makeup = StereoSidechainCompressor::new(-20.0, 4.0, 0.0001, 0.1);
        comp_no_makeup.set_sample_rate(44100.0);

        let mut output = [0.0f32; 2];
        let mut output_no_makeup = [0.0f32; 2];

        for _ in 0..1000 {
            comp.tick(&[0.5, 0.5, 0.9, 0.9], &mut output);
            comp_no_makeup.tick(&[0.5, 0.5, 0.9, 0.9], &mut output_no_makeup);
        }

        assert!(output[0] > output_no_makeup[0]);
    }

    #[test]
    fn test_stereo_compressor_reset() {
        let mut comp = StereoSidechainCompressor::new(-20.0, 4.0, 0.0001, 0.1);
        comp.set_sample_rate(44100.0);

        let mut output = [0.0f32; 2];
        for _ in 0..1000 {
            comp.tick(&[0.5, 0.5, 0.9, 0.9], &mut output);
        }
        assert!(comp.gain_reduction_db() > 0.0);

        comp.reset();
        assert_eq!(comp.gain_reduction_db(), 0.0);
    }
}
