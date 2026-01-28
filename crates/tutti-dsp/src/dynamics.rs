//! Dynamics processors with sidechain support.

use tutti_core::AtomicFloat;
use tutti_core::{AudioUnit, BufferRef, BufferMut, SignalFrame, dsp::DEFAULT_SR};
use std::sync::Arc;

/// Convert linear amplitude to decibels
#[inline]
fn amplitude_to_db(amp: f32) -> f32 {
    if amp <= 0.0 {
        -96.0 // Floor
    } else {
        20.0 * amp.log10()
    }
}

/// Convert decibels to linear amplitude
#[inline]
fn db_to_amplitude(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Calculate smoothing coefficient from time constant
#[inline]
fn time_to_coeff(time_seconds: f32, sample_rate: f64) -> f32 {
    if time_seconds <= 0.0 {
        1.0
    } else {
        (-1.0 / (time_seconds * sample_rate as f32)).exp()
    }
}

/// Compressor with external sidechain input (2-in, 1-out).
pub struct SidechainCompressor {
    threshold_db: Arc<AtomicFloat>,
    ratio: Arc<AtomicFloat>,
    attack: Arc<AtomicFloat>,
    release: Arc<AtomicFloat>,
    makeup_db: Arc<AtomicFloat>,
    knee_db: Arc<AtomicFloat>,

    // Internal state
    envelope: f32,
    gain_reduction: f32,
    sample_rate: f64,
    attack_coeff: f32,
    release_coeff: f32,
    last_attack: f32,
    last_release: f32,
}

impl SidechainCompressor {
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
            // Hard knee
            let over_db = (input_db - threshold).max(0.0);
            over_db * (1.0 - 1.0 / ratio)
        } else {
            // Soft knee
            let half_knee = knee / 2.0;
            let below = threshold - half_knee;
            let above = threshold + half_knee;

            if input_db <= below {
                0.0
            } else if input_db >= above {
                let over_db = input_db - threshold;
                over_db * (1.0 - 1.0 / ratio)
            } else {
                // In the knee region - quadratic interpolation
                let x = input_db - below;
                let slope = (1.0 - 1.0 / ratio) / (2.0 * knee);
                slope * x * x
            }
        }
    }


    #[inline]
    fn process_sample(&mut self, audio: f32, sidechain: f32) -> f32 {
        // Detect level from sidechain (peak detection)
        let input_level = sidechain.abs();
        let input_db = amplitude_to_db(input_level);

        // Calculate target gain reduction
        let target_reduction = self.compute_gain_reduction(input_db);

        // Apply envelope (attack/release smoothing)
        if target_reduction > self.gain_reduction {
            // Attack
            self.gain_reduction = self.attack_coeff * self.gain_reduction
                + (1.0 - self.attack_coeff) * target_reduction;
        } else {
            // Release
            self.gain_reduction = self.release_coeff * self.gain_reduction
                + (1.0 - self.release_coeff) * target_reduction;
        }

        // Store envelope for metering
        self.envelope = input_level;

        // Apply gain reduction + makeup gain
        let gain = db_to_amplitude(-self.gain_reduction + self.makeup_db.get());
        audio * gain
    }
}

impl AudioUnit for SidechainCompressor {
    fn inputs(&self) -> usize {
        2 // audio + sidechain
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
        const SIDECHAIN_COMPRESSOR_ID: u64 = 0x5343_434F_4D50; // "SCCOMP"
        SIDECHAIN_COMPRESSOR_ID
    }

    fn route(&mut self, input: &SignalFrame, _frequency: f64) -> SignalFrame {
        // Compressor doesn't significantly alter frequency response
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
    // Parameters (shared with both channels)
    threshold_db: Arc<AtomicFloat>,
    ratio: Arc<AtomicFloat>,
    attack: Arc<AtomicFloat>,
    release: Arc<AtomicFloat>,
    makeup_db: Arc<AtomicFloat>,
    knee_db: Arc<AtomicFloat>,

    // Internal state
    envelope: f32,
    gain_reduction: f32,
    sample_rate: f64,

    // Cached coefficients
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
        4 // L audio, R audio, L sidechain, R sidechain
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

        // Link detection: use max of both sidechain channels
        let sc_level = sc_l.abs().max(sc_r.abs());
        let input_db = amplitude_to_db(sc_level);

        // Calculate target gain reduction
        let target_reduction = self.compute_gain_reduction(input_db);

        // Apply envelope
        if target_reduction > self.gain_reduction {
            self.gain_reduction = self.attack_coeff * self.gain_reduction
                + (1.0 - self.attack_coeff) * target_reduction;
        } else {
            self.gain_reduction = self.release_coeff * self.gain_reduction
                + (1.0 - self.release_coeff) * target_reduction;
        }

        self.envelope = sc_level;

        // Apply same gain to both channels (linked)
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
        const STEREO_SC_COMP_ID: u64 = 0x5353_4343_4F4D; // "SSCCOM"
        STEREO_SC_COMP_ID
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
///
/// ## Parameters
/// - `threshold_db`: Level above which gate opens
/// - `attack`: Time to fully open gate
/// - `hold`: Time to hold gate open after level drops
/// - `release`: Time to fully close gate
/// - `range_db`: Attenuation when closed (0 = full mute, -20 = some signal passes)
pub struct SidechainGate {
    threshold_db: Arc<AtomicFloat>,
    attack: Arc<AtomicFloat>,
    hold: Arc<AtomicFloat>,
    release: Arc<AtomicFloat>,
    range_db: Arc<AtomicFloat>,

    // Internal state
    envelope: f32,
    gate_level: f32, // 0.0 = closed, 1.0 = open
    hold_counter: usize,
    sample_rate: f64,

    // Cached coefficients
    attack_coeff: f32,
    release_coeff: f32,
    hold_samples: usize,
    last_attack: f32,
    last_release: f32,
    last_hold: f32,
}

impl SidechainGate {
    pub fn new(threshold_db: f32, attack: f32, hold: f32, release: f32) -> Self {
        Self {
            threshold_db: Arc::new(AtomicFloat::new(threshold_db)),
            attack: Arc::new(AtomicFloat::new(attack)),
            hold: Arc::new(AtomicFloat::new(hold)),
            release: Arc::new(AtomicFloat::new(release)),
            range_db: Arc::new(AtomicFloat::new(-80.0)), // Near full mute by default

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

    // Parameter accessors
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

        // Gate state machine
        let gate_open = input_db >= threshold;

        if gate_open {
            // Gate should be open
            self.hold_counter = self.hold_samples;
            // Attack - open the gate
            self.gate_level =
                self.attack_coeff * self.gate_level + (1.0 - self.attack_coeff) * 1.0;
        } else if self.hold_counter > 0 {
            // In hold phase - keep gate open
            self.hold_counter -= 1;
        } else {
            // Release - close the gate
            self.gate_level =
                self.release_coeff * self.gate_level + (1.0 - self.release_coeff) * 0.0;
        }

        // Apply gating with range
        let range_linear = db_to_amplitude(self.range_db.get());
        let gain = range_linear + self.gate_level * (1.0 - range_linear);

        audio * gain
    }
}

impl AudioUnit for SidechainGate {
    fn inputs(&self) -> usize {
        2 // audio + sidechain
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
        const SIDECHAIN_GATE_ID: u64 = 0x5343_4741_5445; // "SCGATE"
        SIDECHAIN_GATE_ID
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
        const STEREO_SC_GATE_ID: u64 = 0x5353_4347_4154; // "SSCGAT"
        STEREO_SC_GATE_ID
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compressor_creation() {
        let comp = SidechainCompressor::new(-20.0, 4.0, 0.001, 0.1);
        assert_eq!(comp.inputs(), 2);
        assert_eq!(comp.outputs(), 1);
        assert_eq!(comp.gain_reduction_db(), 0.0);
    }

    #[test]
    fn test_compressor_reduces_gain_on_loud_sidechain() {
        let mut comp = SidechainCompressor::new(-20.0, 4.0, 0.0001, 0.1);
        comp.set_sample_rate(44100.0);

        let mut output = [0.0f32];

        // Feed loud sidechain signal
        for _ in 0..1000 {
            comp.tick(&[0.5, 0.9], &mut output); // audio=0.5, sidechain=0.9 (loud)
        }

        // Should have some gain reduction
        assert!(
            comp.gain_reduction_db() > 0.0,
            "Expected gain reduction, got {}",
            comp.gain_reduction_db()
        );
        // Output should be quieter than input
        assert!(output[0] < 0.5, "Output should be compressed");
    }

    #[test]
    fn test_compressor_no_reduction_below_threshold() {
        let mut comp = SidechainCompressor::new(-10.0, 4.0, 0.001, 0.1);
        comp.set_sample_rate(44100.0);

        let mut output = [0.0f32];

        // Feed quiet sidechain signal (below -10dB threshold)
        for _ in 0..1000 {
            comp.tick(&[0.5, 0.1], &mut output); // sidechain at -20dB
        }

        // Should have minimal gain reduction
        assert!(
            comp.gain_reduction_db() < 1.0,
            "Unexpected gain reduction: {}",
            comp.gain_reduction_db()
        );
    }

    #[test]
    fn test_gate_creation() {
        let gate = SidechainGate::new(-30.0, 0.001, 0.01, 0.1);
        assert_eq!(gate.inputs(), 2);
        assert_eq!(gate.outputs(), 1);
        assert!(!gate.is_open());
    }

    #[test]
    fn test_gate_opens_on_loud_sidechain() {
        let mut gate = SidechainGate::new(-20.0, 0.0001, 0.01, 0.1);
        gate.set_sample_rate(44100.0);

        let mut output = [0.0f32];

        // Feed loud sidechain
        for _ in 0..500 {
            gate.tick(&[0.5, 0.9], &mut output);
        }

        assert!(gate.is_open(), "Gate should be open");
        assert!(output[0] > 0.3, "Audio should pass through: {}", output[0]);
    }

    #[test]
    fn test_gate_closes_on_quiet_sidechain() {
        let mut gate = SidechainGate::new(-20.0, 0.001, 0.001, 0.001).with_range(-60.0);
        gate.set_sample_rate(44100.0);

        let mut output = [0.0f32];

        // First open the gate
        for _ in 0..500 {
            gate.tick(&[0.5, 0.9], &mut output);
        }
        assert!(gate.is_open());

        // Now feed quiet sidechain
        for _ in 0..2000 {
            gate.tick(&[0.5, 0.01], &mut output);
        }

        assert!(!gate.is_open(), "Gate should be closed");
        assert!(output[0] < 0.1, "Audio should be attenuated: {}", output[0]);
    }

    #[test]
    fn test_stereo_compressor() {
        let mut comp = StereoSidechainCompressor::new(-20.0, 4.0, 0.0001, 0.1);
        comp.set_sample_rate(44100.0);

        let mut output = [0.0f32, 0.0f32];

        for _ in 0..1000 {
            comp.tick(&[0.5, 0.5, 0.9, 0.9], &mut output);
        }

        // Both channels should be equally compressed
        assert!((output[0] - output[1]).abs() < 0.001);
        assert!(output[0] < 0.5);
    }

    #[test]
    fn test_amplitude_db_conversion() {
        assert!((amplitude_to_db(1.0) - 0.0).abs() < 0.001);
        assert!((amplitude_to_db(0.5) - (-6.02)).abs() < 0.1);
        assert!((db_to_amplitude(0.0) - 1.0).abs() < 0.001);
        assert!((db_to_amplitude(-6.0) - 0.501).abs() < 0.01);
    }
}
