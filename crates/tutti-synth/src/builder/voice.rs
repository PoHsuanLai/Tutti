//! Internal voice implementation for PolySynth.

use super::{EnvelopeConfig, FilterType, OscillatorType, SynthConfig};
use crate::UnisonEngine;
use tutti_core::dsp::{adsr_live, moog_q, pass, pink, saw, sine, square, triangle, var};
use tutti_core::{AudioUnit, Shared};

extern crate alloc;
use alloc::vec::Vec;

/// A single sub-voice within a SynthVoice (for unison).
#[derive(Clone)]
struct SubVoice {
    /// Pitch control (frequency in Hz)
    pitch: Shared,
    /// The DSP chain (mono output)
    dsp: Box<dyn AudioUnit>,
}

/// Internal synth voice with FunDSP DSP chain.
/// Contains multiple sub-voices for unison support.
#[derive(Clone)]
pub(crate) struct SynthVoice {
    /// MIDI note number (0-127)
    pub note: u8,
    /// Velocity (0.0-1.0)
    pub velocity: f32,
    /// Gate control (0.0 = off, 1.0 = on) - shared across all sub-voices
    pub gate: Shared,
    /// Filter cutoff control (Hz) - shared across all sub-voices
    pub filter_cutoff: Shared,
    /// Current envelope level (for voice stealing)
    pub envelope_level: f32,
    /// Whether this voice is active
    pub active: bool,
    /// Sub-voices for unison (1 if no unison)
    sub_voices: Vec<SubVoice>,
    /// Stored config for rebuilding sub-voices
    config: SynthConfig,
    /// Current sample rate
    sample_rate: f64,
}

impl SynthVoice {
    /// Create a new voice from synth configuration.
    /// `unison_count` determines how many sub-voices to create (1 if no unison).
    pub fn from_config(config: &SynthConfig, unison_count: usize) -> Self {
        let gate = tutti_core::shared(0.0);
        let filter_cutoff = match &config.filter {
            FilterType::Moog { cutoff, .. } => tutti_core::shared(*cutoff),
            FilterType::Svf { cutoff, .. } => tutti_core::shared(*cutoff),
            FilterType::Biquad { cutoff, .. } => tutti_core::shared(*cutoff),
            FilterType::None => tutti_core::shared(20000.0),
        };

        let count = unison_count.max(1);
        let mut sub_voices = Vec::with_capacity(count);
        for _ in 0..count {
            let pitch = tutti_core::shared(440.0);
            let mut dsp = build_sub_voice_dsp(config, &pitch, &gate, &filter_cutoff);
            dsp.set_sample_rate(config.sample_rate);

            // IMPORTANT: Initialize the DSP chain by ticking with gate=0.
            // FunDSP's adsr_live uses EnvelopeIn which samples at 2ms intervals.
            // We need to tick enough times to ensure the envelope sees control=0
            // before it can respond to gate transitions (control>0 triggers attack).
            // At 44100Hz, 2ms is ~88 samples, so we tick 100 times to be safe.
            let num_outputs = dsp.outputs();
            let mut init_buf = [0.0f32; 2];
            for _ in 0..100 {
                dsp.tick(&[], &mut init_buf[..num_outputs]);
            }

            sub_voices.push(SubVoice { pitch, dsp });
        }

        Self {
            note: 0,
            velocity: 0.0,
            gate,
            filter_cutoff,
            envelope_level: 0.0,
            active: false,
            sub_voices,
            config: config.clone(),
            sample_rate: config.sample_rate,
        }
    }

    /// Trigger a note on with optional unison.
    pub fn note_on(
        &mut self,
        note: u8,
        velocity: f32,
        base_freq: f32,
        unison: Option<&mut UnisonEngine>,
    ) {
        self.note = note;
        self.velocity = velocity;
        self.gate.set(1.0);
        self.active = true;

        if let Some(unison) = unison {
            unison.randomize_phases();
            for (i, sub) in self.sub_voices.iter_mut().enumerate() {
                let params = unison.voice_params(i);
                sub.pitch.set(base_freq * params.freq_ratio);
            }
        } else {
            for sub in &mut self.sub_voices {
                sub.pitch.set(base_freq);
            }
        }
    }

    /// Trigger a note off.
    pub fn note_off(&mut self) {
        self.gate.set(0.0);
    }

    /// Set the pitch (frequency in Hz) for all sub-voices.
    /// When unison is active, applies freq_ratio from unison params.
    pub fn set_pitch(&mut self, base_freq: f32, unison: Option<&UnisonEngine>) {
        if let Some(unison) = unison {
            for (i, sub) in self.sub_voices.iter_mut().enumerate() {
                let params = unison.voice_params(i);
                sub.pitch.set(base_freq * params.freq_ratio);
            }
        } else {
            for sub in &mut self.sub_voices {
                sub.pitch.set(base_freq);
            }
        }
    }

    /// Set filter cutoff.
    pub fn set_filter_cutoff(&mut self, cutoff: f32) {
        self.filter_cutoff.set(cutoff);
    }

    /// Reset the voice.
    pub fn reset(&mut self) {
        self.note = 0;
        self.velocity = 0.0;
        self.envelope_level = 0.0;
        self.active = false;
        self.gate.set(0.0);
        for sub in &mut self.sub_voices {
            sub.dsp.reset();
        }
    }

    /// Get the number of sub-voices.
    #[cfg(test)]
    pub fn sub_voice_count(&self) -> usize {
        self.sub_voices.len()
    }

    /// Process all sub-voices and mix to stereo output with unison panning.
    /// Returns (left, right) stereo samples.
    pub fn tick_stereo(&mut self, unison: Option<&UnisonEngine>) -> (f32, f32) {
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        let mut out_buf = [0.0f32; 2];

        for (i, sub) in self.sub_voices.iter_mut().enumerate() {
            let num_outputs = sub.dsp.outputs();
            out_buf[0] = 0.0;
            out_buf[1] = 0.0;
            sub.dsp.tick(&[], &mut out_buf[..num_outputs]);

            let (pan_pos, amplitude) = if let Some(u) = unison {
                let p = u.voice_params(i);
                (p.pan, p.amplitude)
            } else {
                (0.0, 1.0)
            };

            // Equal-power panning
            let left_gain = ((1.0 - pan_pos) * 0.5).sqrt() * amplitude;
            let right_gain = ((1.0 + pan_pos) * 0.5).sqrt() * amplitude;
            let mono_sample = out_buf[0];
            left += mono_sample * left_gain;
            right += mono_sample * right_gain;
        }

        (left, right)
    }

    /// Set sample rate for all sub-voices.
    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
        for sub in &mut self.sub_voices {
            sub.dsp.set_sample_rate(sample_rate);
        }
    }

    /// Resize sub-voices to match new unison count.
    ///
    /// Rebuilds DSP chains as needed. If the voice is currently active,
    /// new sub-voices start silent (gate is shared, so they'll join on next note-on).
    pub fn resize_unison(&mut self, new_count: usize) {
        let new_count = new_count.max(1);
        let current_count = self.sub_voices.len();

        if new_count == current_count {
            return;
        }

        if new_count > current_count {
            for _ in current_count..new_count {
                let pitch = tutti_core::shared(440.0);
                let mut dsp =
                    build_sub_voice_dsp(&self.config, &pitch, &self.gate, &self.filter_cutoff);
                dsp.set_sample_rate(self.sample_rate);

                // Initialize DSP chain (see from_config for explanation)
                let num_outputs = dsp.outputs();
                let mut init_buf = [0.0f32; 2];
                for _ in 0..100 {
                    dsp.tick(&[], &mut init_buf[..num_outputs]);
                }

                self.sub_voices.push(SubVoice { pitch, dsp });
            }
        } else {
            self.sub_voices.truncate(new_count);
        }
    }

    /// Get footprint of all sub-voices.
    pub fn footprint(&self) -> usize {
        self.sub_voices.iter().map(|s| s.dsp.footprint()).sum()
    }

    /// Allocate all sub-voices.
    pub fn allocate(&mut self) {
        for sub in &mut self.sub_voices {
            sub.dsp.allocate();
        }
    }
}

/// Build a sub-voice DSP chain from configuration.
/// Outputs mono (panning is applied during mixing in PolySynth).
fn build_sub_voice_dsp(
    config: &SynthConfig,
    pitch: &Shared,
    gate: &Shared,
    filter_cutoff: &Shared,
) -> Box<dyn AudioUnit> {
    let env = &config.envelope;

    // Build envelope expression
    let make_env = |g: &Shared, e: &EnvelopeConfig| {
        var(g) >> adsr_live(e.attack, e.decay, e.sustain, e.release)
    };

    // Note: sine() needs <f32>, but saw/square/triangle/pass don't
    // moog_q and pink need <f32>

    // All DSP chains output mono (panning applied during mixing)
    match (&config.oscillator, &config.filter) {
        // No filter - simple osc * envelope (mono)
        (OscillatorType::Sine, FilterType::None) => {
            Box::new((var(pitch) >> sine::<f32>()) * make_env(gate, env))
        }
        (OscillatorType::Saw, FilterType::None) => {
            Box::new((var(pitch) >> saw()) * make_env(gate, env))
        }
        (OscillatorType::Square { .. }, FilterType::None) => {
            Box::new((var(pitch) >> square()) * make_env(gate, env))
        }
        (OscillatorType::Triangle, FilterType::None) => {
            Box::new((var(pitch) >> triangle()) * make_env(gate, env))
        }
        (OscillatorType::Noise, FilterType::None) => Box::new(pink::<f32>() * make_env(gate, env)),

        // Moog filter with modulated cutoff (mono)
        (OscillatorType::Sine, FilterType::Moog { resonance, .. }) => Box::new(
            ((var(pitch) >> sine::<f32>()) | var(filter_cutoff))
                >> moog_q::<f32>(*resonance)
                >> (make_env(gate, env) * pass()),
        ),
        (OscillatorType::Saw, FilterType::Moog { resonance, .. }) => Box::new(
            ((var(pitch) >> saw()) | var(filter_cutoff))
                >> moog_q::<f32>(*resonance)
                >> (make_env(gate, env) * pass()),
        ),
        (OscillatorType::Square { .. }, FilterType::Moog { resonance, .. }) => Box::new(
            ((var(pitch) >> square()) | var(filter_cutoff))
                >> moog_q::<f32>(*resonance)
                >> (make_env(gate, env) * pass()),
        ),
        (OscillatorType::Triangle, FilterType::Moog { resonance, .. }) => Box::new(
            ((var(pitch) >> triangle()) | var(filter_cutoff))
                >> moog_q::<f32>(*resonance)
                >> (make_env(gate, env) * pass()),
        ),
        (OscillatorType::Noise, FilterType::Moog { resonance, .. }) => Box::new(
            (pink::<f32>() | var(filter_cutoff))
                >> moog_q::<f32>(*resonance)
                >> (make_env(gate, env) * pass()),
        ),

        // SVF/Biquad - use Moog filter as approximation (mono)
        (OscillatorType::Sine, FilterType::Svf { q, .. })
        | (OscillatorType::Sine, FilterType::Biquad { q, .. }) => Box::new(
            ((var(pitch) >> sine::<f32>()) | var(filter_cutoff))
                >> moog_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (OscillatorType::Saw, FilterType::Svf { q, .. })
        | (OscillatorType::Saw, FilterType::Biquad { q, .. }) => Box::new(
            ((var(pitch) >> saw()) | var(filter_cutoff))
                >> moog_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (OscillatorType::Square { .. }, FilterType::Svf { q, .. })
        | (OscillatorType::Square { .. }, FilterType::Biquad { q, .. }) => Box::new(
            ((var(pitch) >> square()) | var(filter_cutoff))
                >> moog_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (OscillatorType::Triangle, FilterType::Svf { q, .. })
        | (OscillatorType::Triangle, FilterType::Biquad { q, .. }) => Box::new(
            ((var(pitch) >> triangle()) | var(filter_cutoff))
                >> moog_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (OscillatorType::Noise, FilterType::Svf { q, .. })
        | (OscillatorType::Noise, FilterType::Biquad { q, .. }) => Box::new(
            (pink::<f32>() | var(filter_cutoff))
                >> moog_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
    }
}
