//! Sidechain compressor (mono)

use tutti_core::Arc;
use tutti_core::AtomicFloat;
use tutti_core::{dsp::DEFAULT_SR, AudioUnit, BufferMut, BufferRef, SignalFrame};

use super::utils::{amplitude_to_db, db_to_amplitude, time_to_coeff};

/// Compressor with external sidechain input (2-in, 1-out).
pub struct SidechainCompressor {
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

impl SidechainCompressor {
    /// Create a new compressor. Prefer [`SidechainCompressor::builder()`].
    pub(crate) fn new(threshold_db: f32, ratio: f32, attack: f32, release: f32) -> Self {
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

    /// Create a builder for configuring a compressor
    pub fn builder() -> SidechainCompressorBuilder {
        SidechainCompressorBuilder::default()
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

    pub fn knee_width(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.knee_db)
    }

    pub fn set_threshold(&self, db: f32) {
        self.threshold_db.set(db);
    }

    pub fn set_ratio(&self, ratio: f32) {
        self.ratio.set(ratio.max(1.0));
    }

    pub fn set_attack(&self, seconds: f32) {
        self.attack.set(seconds.max(0.0));
    }

    pub fn set_release(&self, seconds: f32) {
        self.release.set(seconds.max(0.0));
    }

    pub fn set_makeup(&self, db: f32) {
        self.makeup_db.set(db);
    }

    pub fn gain_reduction_db(&self) -> f32 {
        self.gain_reduction
    }

    pub fn envelope_level(&self) -> f32 {
        self.envelope
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

    #[inline]
    fn process_sample(&mut self, audio: f32, sidechain: f32) -> f32 {
        let input_level = sidechain.abs();
        let input_db = amplitude_to_db(input_level);
        let target_reduction = self.compute_gain_reduction(input_db);

        if target_reduction > self.gain_reduction {
            self.gain_reduction = self.attack_coeff * self.gain_reduction
                + (1.0 - self.attack_coeff) * target_reduction;
        } else {
            self.gain_reduction = self.release_coeff * self.gain_reduction
                + (1.0 - self.release_coeff) * target_reduction;
        }

        self.envelope = input_level;

        let gain = db_to_amplitude(-self.gain_reduction + self.makeup_db.get());
        audio * gain
    }
}

impl AudioUnit for SidechainCompressor {
    fn inputs(&self) -> usize {
        2
    }

    fn outputs(&self) -> usize {
        1
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
        0x5343_434F_4D50
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

impl Clone for SidechainCompressor {
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

/// Builder for configuring a SidechainCompressor with fluent API.
#[derive(Clone, Debug)]
pub struct SidechainCompressorBuilder {
    threshold_db: f32,
    ratio: f32,
    attack_seconds: f32,
    release_seconds: f32,
    makeup_db: f32,
    knee_db: f32,
}

impl Default for SidechainCompressorBuilder {
    fn default() -> Self {
        Self {
            threshold_db: -20.0,
            ratio: 4.0,
            attack_seconds: 0.005,
            release_seconds: 0.1,
            makeup_db: 0.0,
            knee_db: 0.0,
        }
    }
}

impl SidechainCompressorBuilder {
    /// Set the threshold in decibels (-60.0 to 0.0 dB typical)
    pub fn threshold_db(mut self, db: f32) -> Self {
        self.threshold_db = db;
        self
    }

    /// Set the compression ratio (must be >= 1.0)
    pub fn ratio(mut self, ratio: f32) -> Self {
        self.ratio = ratio.max(1.0);
        self
    }

    /// Set the attack time in seconds (0.0001 to 0.1 typical)
    pub fn attack_seconds(mut self, seconds: f32) -> Self {
        self.attack_seconds = seconds.max(0.0);
        self
    }

    /// Set the release time in seconds (0.01 to 1.0 typical)
    pub fn release_seconds(mut self, seconds: f32) -> Self {
        self.release_seconds = seconds.max(0.0);
        self
    }

    /// Set soft knee width in decibels (0.0 = hard knee)
    pub fn soft_knee_db(mut self, db: f32) -> Self {
        self.knee_db = db.max(0.0);
        self
    }

    /// Set makeup gain in decibels
    pub fn makeup_gain_db(mut self, db: f32) -> Self {
        self.makeup_db = db;
        self
    }

    /// Build the configured SidechainCompressor
    pub fn build(self) -> SidechainCompressor {
        SidechainCompressor::new(
            self.threshold_db,
            self.ratio,
            self.attack_seconds,
            self.release_seconds,
        )
        .with_soft_knee(self.knee_db)
        .with_makeup(self.makeup_db)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compressor_reduces_gain_on_loud_sidechain() {
        let mut comp = SidechainCompressor::new(-20.0, 4.0, 0.0001, 0.1);
        comp.set_sample_rate(44100.0);

        let mut output = [0.0f32];

        for _ in 0..1000 {
            comp.tick(&[0.5, 0.9], &mut output);
        }

        assert!(comp.gain_reduction_db() > 0.0);
        assert!(output[0] < 0.5);
    }

    #[test]
    fn test_compressor_no_reduction_below_threshold() {
        let mut comp = SidechainCompressor::new(-10.0, 4.0, 0.001, 0.1);
        comp.set_sample_rate(44100.0);

        let mut output = [0.0f32];

        for _ in 0..1000 {
            comp.tick(&[0.5, 0.1], &mut output);
        }

        assert!(comp.gain_reduction_db() < 1.0);
    }

    #[test]
    fn test_compressor_soft_knee_differs_from_hard_knee() {
        let mut hard = SidechainCompressor::new(-20.0, 4.0, 0.0001, 0.1);
        hard.set_sample_rate(44100.0);

        let mut soft = SidechainCompressor::new(-20.0, 4.0, 0.0001, 0.1).with_soft_knee(12.0);
        soft.set_sample_rate(44100.0);

        let mut hard_out = [0.0f32];
        let mut soft_out = [0.0f32];

        for _ in 0..1000 {
            hard.tick(&[0.5, 0.15], &mut hard_out);
            soft.tick(&[0.5, 0.15], &mut soft_out);
        }

        assert!(
            (hard_out[0] - soft_out[0]).abs() > 0.001,
            "Soft knee should produce different output near threshold: hard={}, soft={}",
            hard_out[0],
            soft_out[0]
        );
    }

    #[test]
    fn test_compressor_makeup_gain() {
        let mut comp = SidechainCompressor::new(-20.0, 4.0, 0.0001, 0.1).with_makeup(6.0);
        comp.set_sample_rate(44100.0);

        let mut comp_no_makeup = SidechainCompressor::new(-20.0, 4.0, 0.0001, 0.1);
        comp_no_makeup.set_sample_rate(44100.0);

        let mut output = [0.0f32];
        let mut output_no_makeup = [0.0f32];

        for _ in 0..1000 {
            comp.tick(&[0.5, 0.9], &mut output);
            comp_no_makeup.tick(&[0.5, 0.9], &mut output_no_makeup);
        }

        assert!(output[0] > output_no_makeup[0]);
    }

    #[test]
    fn test_compressor_reset() {
        let mut comp = SidechainCompressor::new(-20.0, 4.0, 0.0001, 0.1);
        comp.set_sample_rate(44100.0);

        let mut output = [0.0f32];
        for _ in 0..1000 {
            comp.tick(&[0.5, 0.9], &mut output);
        }
        assert!(comp.gain_reduction_db() > 0.0);

        comp.reset();
        assert_eq!(comp.gain_reduction_db(), 0.0);
        assert_eq!(comp.envelope_level(), 0.0);
    }

    #[test]
    fn test_compressor_ratio_clamps_to_minimum() {
        let comp = SidechainCompressor::new(-20.0, 4.0, 0.001, 0.1);
        comp.set_ratio(0.5);
        assert_eq!(comp.ratio().get(), 1.0);
    }
}
