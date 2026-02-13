//! Internal voice implementation for PolySynth.

use super::SvfMode;
use super::{EnvelopeConfig, FilterModConfig, FilterType, OscillatorType, SynthConfig};
use crate::UnisonEngine;
use tutti_core::dsp::{
    adsr_live, bandpass_q, dc, highpass_q, lowpass_q, moog, notch_q, pass, pink, poly_pulse, saw,
    sine, triangle, var,
};
use tutti_core::{AudioUnit, Shared};

extern crate alloc;
use alloc::vec::Vec;

#[derive(Clone)]
struct SubVoice {
    pitch: Shared,
    dsp: Box<dyn AudioUnit>,
}

#[derive(Clone)]
pub(crate) struct SynthVoice {
    pub note: u8,
    pub channel: u8,
    pub velocity: f32,
    pub gate: Shared,
    filter_cutoff: Shared,
    base_filter_cutoff: f32,
    filter_resonance: Shared,
    base_filter_resonance: f32,
    mod_wheel_value: f32,
    velocity_mod_value: f32,
    cc_cutoff_value: f32,
    cc_resonance_value: f32,
    filter_mod: FilterModConfig,
    lfo_phase: f32,
    pub envelope_level: f32,
    pub active: bool,
    sub_voices: Vec<SubVoice>,
    config: SynthConfig,
    sample_rate: f64,
}

impl SynthVoice {
    pub fn from_config(config: &SynthConfig, unison_count: usize) -> Self {
        let gate = tutti_core::shared(0.0);
        let base_filter_cutoff = match &config.filter {
            FilterType::Moog { cutoff, .. } => *cutoff,
            FilterType::Svf { cutoff, .. } => *cutoff,
            FilterType::None => 20000.0,
        };
        let filter_cutoff = tutti_core::shared(base_filter_cutoff);

        let base_filter_resonance = match &config.filter {
            FilterType::Moog { resonance, .. } => *resonance,
            FilterType::Svf { q, .. } => *q,
            FilterType::None => 0.0,
        };
        let filter_resonance = tutti_core::shared(base_filter_resonance);

        let count = unison_count.max(1);
        let mut sub_voices = Vec::with_capacity(count);
        for _ in 0..count {
            let pitch = tutti_core::shared(440.0);
            let mut dsp =
                build_sub_voice_dsp(config, &pitch, &gate, &filter_cutoff, &filter_resonance);
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
            channel: 0,
            velocity: 0.0,
            gate,
            filter_cutoff,
            base_filter_cutoff,
            filter_resonance,
            base_filter_resonance,
            mod_wheel_value: 0.0,
            velocity_mod_value: 1.0,
            cc_cutoff_value: 0.5,
            cc_resonance_value: 0.0,
            filter_mod: config.filter_mod,
            lfo_phase: 0.0,
            envelope_level: 0.0,
            active: false,
            sub_voices,
            config: config.clone(),
            sample_rate: config.sample_rate,
        }
    }

    pub fn note_on(
        &mut self,
        note: u8,
        channel: u8,
        velocity: f32,
        base_freq: f32,
        unison: Option<&mut UnisonEngine>,
    ) {
        self.note = note;
        self.channel = channel;
        self.velocity = velocity;
        self.gate.set(1.0);
        self.active = true;
        self.lfo_phase = 0.0;

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

    pub fn note_off(&mut self) {
        self.gate.set(0.0);
    }

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

    pub fn reset(&mut self) {
        self.note = 0;
        self.channel = 0;
        self.velocity = 0.0;
        self.envelope_level = 0.0;
        self.active = false;
        self.gate.set(0.0);
        self.mod_wheel_value = 0.0;
        self.velocity_mod_value = 1.0;
        self.cc_cutoff_value = 0.5;
        self.cc_resonance_value = 0.0;
        self.lfo_phase = 0.0;
        self.filter_cutoff.set(self.base_filter_cutoff);
        self.filter_resonance.set(self.base_filter_resonance);
        for sub in &mut self.sub_voices {
            sub.dsp.reset();
        }
    }

    #[cfg(test)]
    pub fn sub_voice_count(&self) -> usize {
        self.sub_voices.len()
    }

    pub fn tick_stereo(&mut self, unison: Option<&UnisonEngine>) -> (f32, f32) {
        self.update_modulated_filter();

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

            let left_gain = ((1.0 - pan_pos) * 0.5).sqrt() * amplitude;
            let right_gain = ((1.0 + pan_pos) * 0.5).sqrt() * amplitude;
            let mono_sample = out_buf[0];
            left += mono_sample * left_gain;
            right += mono_sample * right_gain;
        }

        (left * self.velocity, right * self.velocity)
    }

    fn update_modulated_filter(&mut self) {
        let fm = &self.filter_mod;
        let has_filter_mod =
            fm.mod_wheel_depth > 0.0 || fm.velocity_depth > 0.0 || fm.lfo_depth > 0.0;
        let has_cc_cutoff = self.cc_cutoff_value != 0.5;
        let has_cc_resonance = self.cc_resonance_value != 0.0;

        if !has_filter_mod && !has_cc_cutoff && !has_cc_resonance {
            return;
        }

        // Cutoff modulation
        if has_filter_mod || has_cc_cutoff {
            let mut cutoff = self.base_filter_cutoff;

            // Mod wheel: at depth 1.0, fully up doubles cutoff
            if fm.mod_wheel_depth > 0.0 {
                cutoff *= 1.0 + self.mod_wheel_value * fm.mod_wheel_depth;
            }

            // Velocity: at depth 1.0, vel 0 = 0.5x cutoff, vel 1 = 1x cutoff
            if fm.velocity_depth > 0.0 {
                let vel_mult = 1.0 - fm.velocity_depth * 0.5
                    + self.velocity_mod_value * fm.velocity_depth * 0.5;
                cutoff *= vel_mult;
            }

            // LFO: at depth 1.0, sweeps cutoff ±50%
            if fm.lfo_depth > 0.0 && fm.lfo_rate > 0.0 {
                let phase_inc = fm.lfo_rate / self.sample_rate as f32;
                self.lfo_phase = (self.lfo_phase + phase_inc) % 1.0;

                let lfo_val = (self.lfo_phase * core::f32::consts::TAU).sin();
                cutoff *= 1.0 + lfo_val * fm.lfo_depth * 0.5;
            }

            // CC 74: 0.0 = 0.25x base, 0.5 = 1.0x (neutral), 1.0 = 4x base (±2 octaves)
            if has_cc_cutoff {
                let factor = (4.0_f32).powf(self.cc_cutoff_value - 0.5);
                cutoff *= factor;
            }

            self.filter_cutoff.set(cutoff);
        }

        // CC 71 resonance: scale 0.0-1.0 into base..0.95 range
        if has_cc_resonance {
            let base_res = self.base_filter_resonance;
            let max_res = 0.95;
            let res = base_res + self.cc_resonance_value * (max_res - base_res);
            self.filter_resonance.set(res);
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
        for sub in &mut self.sub_voices {
            sub.dsp.set_sample_rate(sample_rate);
        }
    }

    pub fn set_mod_wheel(&mut self, value: f32) {
        self.mod_wheel_value = value;
    }

    pub fn set_velocity_mod(&mut self, value: f32) {
        self.velocity_mod_value = value;
    }

    pub fn set_cc_cutoff(&mut self, value: f32) {
        self.cc_cutoff_value = value;
    }

    pub fn set_filter_resonance(&mut self, value: f32) {
        self.cc_resonance_value = value;
    }

    /// New sub-voices start silent and join on next note-on.
    pub fn resize_unison(&mut self, new_count: usize) {
        let new_count = new_count.max(1);
        let current_count = self.sub_voices.len();

        if new_count == current_count {
            return;
        }

        if new_count > current_count {
            for _ in current_count..new_count {
                let pitch = tutti_core::shared(440.0);
                let mut dsp = build_sub_voice_dsp(
                    &self.config,
                    &pitch,
                    &self.gate,
                    &self.filter_cutoff,
                    &self.filter_resonance,
                );
                dsp.set_sample_rate(self.sample_rate);

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

    pub fn footprint(&self) -> usize {
        self.sub_voices.iter().map(|s| s.dsp.footprint()).sum()
    }

    pub fn allocate(&mut self) {
        for sub in &mut self.sub_voices {
            sub.dsp.allocate();
        }
    }
}

fn build_sub_voice_dsp(
    config: &SynthConfig,
    pitch: &Shared,
    gate: &Shared,
    filter_cutoff: &Shared,
    filter_resonance: &Shared,
) -> Box<dyn AudioUnit> {
    let env = &config.envelope;

    let make_env = |g: &Shared, e: &EnvelopeConfig| {
        var(g) >> adsr_live(e.attack, e.decay, e.sustain, e.release)
    };

    match (&config.oscillator, &config.filter) {
        (OscillatorType::Sine, FilterType::None) => {
            Box::new((var(pitch) >> sine::<f32>()) * make_env(gate, env))
        }
        (OscillatorType::Saw, FilterType::None) => {
            Box::new((var(pitch) >> saw()) * make_env(gate, env))
        }
        (OscillatorType::Square { pulse_width }, FilterType::None) => {
            Box::new(((var(pitch) | dc(*pulse_width)) >> poly_pulse::<f32>()) * make_env(gate, env))
        }
        (OscillatorType::Triangle, FilterType::None) => {
            Box::new((var(pitch) >> triangle()) * make_env(gate, env))
        }
        (OscillatorType::Noise, FilterType::None) => Box::new(pink::<f32>() * make_env(gate, env)),

        // Moog filter: 3-input moog (signal + cutoff + Q) — resonance is a runtime Shared
        (OscillatorType::Sine, FilterType::Moog { .. }) => Box::new(
            ((var(pitch) >> sine::<f32>()) | var(filter_cutoff) | var(filter_resonance))
                >> moog::<f32>()
                >> (make_env(gate, env) * pass()),
        ),
        (OscillatorType::Saw, FilterType::Moog { .. }) => Box::new(
            ((var(pitch) >> saw()) | var(filter_cutoff) | var(filter_resonance))
                >> moog::<f32>()
                >> (make_env(gate, env) * pass()),
        ),
        (OscillatorType::Square { pulse_width }, FilterType::Moog { .. }) => Box::new(
            (((var(pitch) | dc(*pulse_width)) >> poly_pulse::<f32>())
                | var(filter_cutoff)
                | var(filter_resonance))
                >> moog::<f32>()
                >> (make_env(gate, env) * pass()),
        ),
        (OscillatorType::Triangle, FilterType::Moog { .. }) => Box::new(
            ((var(pitch) >> triangle()) | var(filter_cutoff) | var(filter_resonance))
                >> moog::<f32>()
                >> (make_env(gate, env) * pass()),
        ),
        (OscillatorType::Noise, FilterType::Moog { .. }) => Box::new(
            (pink::<f32>() | var(filter_cutoff) | var(filter_resonance))
                >> moog::<f32>()
                >> (make_env(gate, env) * pass()),
        ),

        // SVF filters: resonance remains fixed (no 3-input variant available)
        (
            OscillatorType::Sine,
            FilterType::Svf {
                q,
                mode: SvfMode::Lowpass,
                ..
            },
        ) => Box::new(
            ((var(pitch) >> sine::<f32>()) | var(filter_cutoff))
                >> lowpass_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (
            OscillatorType::Saw,
            FilterType::Svf {
                q,
                mode: SvfMode::Lowpass,
                ..
            },
        ) => Box::new(
            ((var(pitch) >> saw()) | var(filter_cutoff))
                >> lowpass_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (
            OscillatorType::Square { pulse_width },
            FilterType::Svf {
                q,
                mode: SvfMode::Lowpass,
                ..
            },
        ) => Box::new(
            (((var(pitch) | dc(*pulse_width)) >> poly_pulse::<f32>()) | var(filter_cutoff))
                >> lowpass_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (
            OscillatorType::Triangle,
            FilterType::Svf {
                q,
                mode: SvfMode::Lowpass,
                ..
            },
        ) => Box::new(
            ((var(pitch) >> triangle()) | var(filter_cutoff))
                >> lowpass_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (
            OscillatorType::Noise,
            FilterType::Svf {
                q,
                mode: SvfMode::Lowpass,
                ..
            },
        ) => Box::new(
            (pink::<f32>() | var(filter_cutoff))
                >> lowpass_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),

        (
            OscillatorType::Sine,
            FilterType::Svf {
                q,
                mode: SvfMode::Highpass,
                ..
            },
        ) => Box::new(
            ((var(pitch) >> sine::<f32>()) | var(filter_cutoff))
                >> highpass_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (
            OscillatorType::Saw,
            FilterType::Svf {
                q,
                mode: SvfMode::Highpass,
                ..
            },
        ) => Box::new(
            ((var(pitch) >> saw()) | var(filter_cutoff))
                >> highpass_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (
            OscillatorType::Square { pulse_width },
            FilterType::Svf {
                q,
                mode: SvfMode::Highpass,
                ..
            },
        ) => Box::new(
            (((var(pitch) | dc(*pulse_width)) >> poly_pulse::<f32>()) | var(filter_cutoff))
                >> highpass_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (
            OscillatorType::Triangle,
            FilterType::Svf {
                q,
                mode: SvfMode::Highpass,
                ..
            },
        ) => Box::new(
            ((var(pitch) >> triangle()) | var(filter_cutoff))
                >> highpass_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (
            OscillatorType::Noise,
            FilterType::Svf {
                q,
                mode: SvfMode::Highpass,
                ..
            },
        ) => Box::new(
            (pink::<f32>() | var(filter_cutoff))
                >> highpass_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),

        (
            OscillatorType::Sine,
            FilterType::Svf {
                q,
                mode: SvfMode::Bandpass,
                ..
            },
        ) => Box::new(
            ((var(pitch) >> sine::<f32>()) | var(filter_cutoff))
                >> bandpass_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (
            OscillatorType::Saw,
            FilterType::Svf {
                q,
                mode: SvfMode::Bandpass,
                ..
            },
        ) => Box::new(
            ((var(pitch) >> saw()) | var(filter_cutoff))
                >> bandpass_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (
            OscillatorType::Square { pulse_width },
            FilterType::Svf {
                q,
                mode: SvfMode::Bandpass,
                ..
            },
        ) => Box::new(
            (((var(pitch) | dc(*pulse_width)) >> poly_pulse::<f32>()) | var(filter_cutoff))
                >> bandpass_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (
            OscillatorType::Triangle,
            FilterType::Svf {
                q,
                mode: SvfMode::Bandpass,
                ..
            },
        ) => Box::new(
            ((var(pitch) >> triangle()) | var(filter_cutoff))
                >> bandpass_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (
            OscillatorType::Noise,
            FilterType::Svf {
                q,
                mode: SvfMode::Bandpass,
                ..
            },
        ) => Box::new(
            (pink::<f32>() | var(filter_cutoff))
                >> bandpass_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),

        (
            OscillatorType::Sine,
            FilterType::Svf {
                q,
                mode: SvfMode::Notch,
                ..
            },
        ) => Box::new(
            ((var(pitch) >> sine::<f32>()) | var(filter_cutoff))
                >> notch_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (
            OscillatorType::Saw,
            FilterType::Svf {
                q,
                mode: SvfMode::Notch,
                ..
            },
        ) => Box::new(
            ((var(pitch) >> saw()) | var(filter_cutoff))
                >> notch_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (
            OscillatorType::Square { pulse_width },
            FilterType::Svf {
                q,
                mode: SvfMode::Notch,
                ..
            },
        ) => Box::new(
            (((var(pitch) | dc(*pulse_width)) >> poly_pulse::<f32>()) | var(filter_cutoff))
                >> notch_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (
            OscillatorType::Triangle,
            FilterType::Svf {
                q,
                mode: SvfMode::Notch,
                ..
            },
        ) => Box::new(
            ((var(pitch) >> triangle()) | var(filter_cutoff))
                >> notch_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
        (
            OscillatorType::Noise,
            FilterType::Svf {
                q,
                mode: SvfMode::Notch,
                ..
            },
        ) => Box::new(
            (pink::<f32>() | var(filter_cutoff))
                >> notch_q::<f32>(*q)
                >> (make_env(gate, env) * pass()),
        ),
    }
}
