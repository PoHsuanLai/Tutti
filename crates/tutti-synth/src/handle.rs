//! Fluent handle for creating MIDI-responsive synthesizers.

use crate::{EnvelopeConfig, FilterType, OscillatorType, PolySynth, SvfMode, SynthBuilder};
use tutti_core::midi::MidiRegistry;
use tutti_core::AudioUnit; // For get_id()

/// Fluent builder for creating MIDI-responsive polyphonic synthesizers.
///
/// Wraps `SynthBuilder` and provides a cleaner API that automatically
/// wires up the `MidiRegistry` so the synth responds to MIDI events.
///
/// # Example
/// ```ignore
/// let synth = SynthHandle::new(44100.0, midi_registry)
///     .saw()
///     .poly(8)
///     .filter_moog(2000.0, 0.7)
///     .adsr(0.01, 0.2, 0.6, 0.3)
///     .build()?;
/// ```
pub struct SynthHandle {
    builder: SynthBuilder,
    midi_registry: MidiRegistry,
}

impl SynthHandle {
    /// Create a new synth handle.
    pub fn new(sample_rate: f64, midi_registry: MidiRegistry) -> Self {
        Self {
            builder: SynthBuilder::new(sample_rate),
            midi_registry,
        }
    }

    /// Use sine wave oscillator.
    pub fn sine(mut self) -> Self {
        self.builder = self.builder.oscillator(OscillatorType::Sine);
        self
    }

    /// Use sawtooth wave oscillator (default).
    pub fn saw(mut self) -> Self {
        self.builder = self.builder.oscillator(OscillatorType::Saw);
        self
    }

    /// Use square wave oscillator with pulse width (0.0 - 1.0).
    pub fn square(mut self, pulse_width: f32) -> Self {
        self.builder = self
            .builder
            .oscillator(OscillatorType::Square { pulse_width });
        self
    }

    /// Use triangle wave oscillator.
    pub fn triangle(mut self) -> Self {
        self.builder = self.builder.oscillator(OscillatorType::Triangle);
        self
    }

    /// Use noise oscillator.
    pub fn noise(mut self) -> Self {
        self.builder = self.builder.oscillator(OscillatorType::Noise);
        self
    }

    /// Set polyphonic mode with the specified number of voices.
    pub fn poly(mut self, voices: usize) -> Self {
        self.builder = self.builder.poly(voices);
        self
    }

    /// Set monophonic mode (single voice, retrigger on each note).
    pub fn mono(mut self) -> Self {
        self.builder = self.builder.mono();
        self
    }

    /// Set legato mode (single voice, glide between overlapping notes).
    pub fn legato(mut self) -> Self {
        self.builder = self.builder.legato();
        self
    }

    /// Use Moog ladder lowpass filter.
    pub fn filter_moog(mut self, cutoff: f32, resonance: f32) -> Self {
        self.builder = self.builder.filter(FilterType::Moog { cutoff, resonance });
        self
    }

    /// Use SVF lowpass filter.
    pub fn filter_lowpass(mut self, cutoff: f32, q: f32) -> Self {
        self.builder = self.builder.filter(FilterType::Svf {
            cutoff,
            q,
            mode: SvfMode::Lowpass,
        });
        self
    }

    /// Use SVF highpass filter.
    pub fn filter_highpass(mut self, cutoff: f32, q: f32) -> Self {
        self.builder = self.builder.filter(FilterType::Svf {
            cutoff,
            q,
            mode: SvfMode::Highpass,
        });
        self
    }

    /// Use SVF bandpass filter.
    pub fn filter_bandpass(mut self, cutoff: f32, q: f32) -> Self {
        self.builder = self.builder.filter(FilterType::Svf {
            cutoff,
            q,
            mode: SvfMode::Bandpass,
        });
        self
    }

    /// Bypass filter (no filtering).
    pub fn no_filter(mut self) -> Self {
        self.builder = self.builder.filter(FilterType::None);
        self
    }

    /// Set ADSR envelope parameters.
    pub fn adsr(mut self, attack: f32, decay: f32, sustain: f32, release: f32) -> Self {
        self.builder = self.builder.envelope(attack, decay, sustain, release);
        self
    }

    /// Use organ envelope preset (fast attack, full sustain).
    pub fn envelope_organ(mut self) -> Self {
        self.builder = self.builder.envelope_config(EnvelopeConfig::organ());
        self
    }

    /// Use pluck envelope preset (fast attack and decay).
    pub fn envelope_pluck(mut self) -> Self {
        self.builder = self.builder.envelope_config(EnvelopeConfig::pluck());
        self
    }

    /// Use pad envelope preset (slow attack and release).
    pub fn envelope_pad(mut self) -> Self {
        self.builder = self.builder.envelope_config(EnvelopeConfig::pad());
        self
    }

    /// Build the synth with MIDI wired up.
    ///
    /// Returns the `PolySynth` ready to be added to an audio graph.
    /// The synth's ID is automatically registered with the MidiRegistry.
    pub fn build(self) -> crate::Result<PolySynth> {
        let synth = self.builder.build()?;

        // Register with MIDI registry
        let synth_id = synth.get_id();
        self.midi_registry.register_unit(synth_id);

        // Wire up registry
        let synth = synth.with_midi_registry(self.midi_registry);

        Ok(synth)
    }
}
