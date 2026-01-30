//! Simple polyphonic synthesizer
//!
//! A basic polyphonic synth with:
//! - Multiple oscillator types (sine, saw, square, triangle)
//! - ADSR envelope per voice
//! - Polyphonic voice management
//! - MIDI input via MidiRegistry

use tutti_core::{AudioUnit, BufferMut, BufferRef, Setting, SignalFrame};

/// Oscillator waveform type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waveform {
    Sine,
    Saw,
    Square,
    Triangle,
}

/// ADSR envelope parameters
#[derive(Debug, Clone, Copy)]
pub struct Envelope {
    pub attack: f32,  // seconds
    pub decay: f32,   // seconds
    pub sustain: f32, // 0.0 - 1.0
    pub release: f32, // seconds
}

impl Default for Envelope {
    fn default() -> Self {
        Self {
            attack: 0.01,
            decay: 0.1,
            sustain: 0.7,
            release: 0.2,
        }
    }
}

/// Voice state
#[derive(Clone)]
enum VoiceState {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Single voice in the polyphonic synth
#[derive(Clone)]
struct Voice {
    velocity: f32,
    phase: f32,
    frequency: f32,
    state: VoiceState,
    envelope_value: f32,
    time_in_state: f32, // samples
}

impl Voice {
    fn new(note: u8, velocity: u8, _sample_rate: f32) -> Self {
        let frequency = midi_note_to_hz(note);
        Self {
            velocity: velocity as f32 / 127.0,
            phase: 0.0,
            frequency,
            state: VoiceState::Attack,
            envelope_value: 0.0,
            time_in_state: 0.0,
        }
    }

    fn is_idle(&self) -> bool {
        matches!(self.state, VoiceState::Idle)
    }

    fn process(&mut self, waveform: Waveform, envelope: &Envelope, sample_rate: f32) -> f32 {
        // Generate oscillator
        let osc_value = match waveform {
            Waveform::Sine => (self.phase * std::f32::consts::TAU).sin(),
            Waveform::Saw => 2.0 * (self.phase - self.phase.floor()) - 1.0,
            Waveform::Square => {
                if self.phase % 1.0 < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Waveform::Triangle => {
                let t = self.phase % 1.0;
                if t < 0.5 {
                    4.0 * t - 1.0
                } else {
                    -4.0 * t + 3.0
                }
            }
        };

        // Advance phase
        self.phase += self.frequency / sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }

        // Process envelope
        let attack_samples = envelope.attack * sample_rate;
        let decay_samples = envelope.decay * sample_rate;
        let release_samples = envelope.release * sample_rate;

        match self.state {
            VoiceState::Attack => {
                if self.time_in_state < attack_samples {
                    self.envelope_value = self.time_in_state / attack_samples;
                } else {
                    self.state = VoiceState::Decay;
                    self.time_in_state = 0.0;
                }
            }
            VoiceState::Decay => {
                if self.time_in_state < decay_samples {
                    let t = self.time_in_state / decay_samples;
                    self.envelope_value = 1.0 - (1.0 - envelope.sustain) * t;
                } else {
                    self.state = VoiceState::Sustain;
                    self.envelope_value = envelope.sustain;
                }
            }
            VoiceState::Sustain => {
                self.envelope_value = envelope.sustain;
            }
            VoiceState::Release => {
                if self.time_in_state < release_samples {
                    let t = self.time_in_state / release_samples;
                    self.envelope_value = envelope.sustain * (1.0 - t);
                } else {
                    self.state = VoiceState::Idle;
                    self.envelope_value = 0.0;
                }
            }
            VoiceState::Idle => {
                self.envelope_value = 0.0;
            }
        }

        self.time_in_state += 1.0;

        osc_value * self.envelope_value * self.velocity
    }

    fn release(&mut self) {
        if !matches!(self.state, VoiceState::Idle | VoiceState::Release) {
            self.state = VoiceState::Release;
            self.time_in_state = 0.0;
        }
    }
}

/// Simple polyphonic synthesizer
#[derive(Clone)]
pub struct PolySynth {
    voices: Vec<Voice>,
    max_voices: usize,
    waveform: Waveform,
    envelope: Envelope,
    sample_rate: f32,

    // MIDI note tracking (fixed-size array indexed by note 0-127, avoids HashMap allocations)
    active_notes: [Option<usize>; 128],

    // MIDI registry for polling events
    #[cfg(feature = "midi")]
    midi_registry: Option<tutti_core::MidiRegistry>,

    // Pre-allocated scratch buffer for RT-safe MIDI polling
    #[cfg(feature = "midi")]
    midi_buffer: Vec<tutti_core::MidiEvent>,
}

impl PolySynth {
    /// Create a new polyphonic synthesizer without MIDI support
    pub fn new(sample_rate: f32, max_voices: usize) -> Self {
        Self {
            voices: Vec::with_capacity(max_voices),
            max_voices,
            waveform: Waveform::Saw,
            envelope: Envelope::default(),
            sample_rate,
            active_notes: [None; 128],

            #[cfg(feature = "midi")]
            midi_registry: None,
            #[cfg(feature = "midi")]
            midi_buffer: Vec::new(),
        }
    }

    /// Create a new polyphonic synthesizer with MIDI support
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate
    /// * `max_voices` - Maximum number of simultaneous voices
    /// * `midi_registry` - MIDI registry for receiving events
    #[cfg(feature = "midi")]
    pub fn midi(
        sample_rate: f32,
        max_voices: usize,
        midi_registry: tutti_core::MidiRegistry,
    ) -> Self {
        Self {
            voices: Vec::with_capacity(max_voices),
            max_voices,
            waveform: Waveform::Saw,
            envelope: Envelope::default(),
            sample_rate,
            active_notes: [None; 128],
            midi_registry: Some(midi_registry),
            midi_buffer: {
                let placeholder = tutti_core::MidiEvent::note_on_builder(0, 0).build();
                vec![placeholder; 256]
            },
        }
    }

    /// Set the oscillator waveform
    pub fn set_waveform(&mut self, waveform: Waveform) {
        self.waveform = waveform;
    }

    /// Set the ADSR envelope
    pub fn set_envelope(&mut self, envelope: Envelope) {
        self.envelope = envelope;
    }

    /// Trigger a note on
    pub fn note_on(&mut self, note: u8, velocity: u8) {
        // Find idle voice or steal oldest
        let voice_idx = if let Some(idx) = self.voices.iter().position(|v| v.is_idle()) {
            idx
        } else if self.voices.len() < self.max_voices {
            self.voices
                .push(Voice::new(note, velocity, self.sample_rate));
            self.voices.len() - 1
        } else {
            // Voice stealing: take first voice
            0
        };

        // Initialize voice
        if voice_idx < self.voices.len() {
            self.voices[voice_idx] = Voice::new(note, velocity, self.sample_rate);
        }

        self.active_notes[note as usize] = Some(voice_idx);
    }

    /// Trigger a note off
    pub fn note_off(&mut self, note: u8) {
        if let Some(voice_idx) = self.active_notes[note as usize] {
            if voice_idx < self.voices.len() {
                self.voices[voice_idx].release();
            }
            self.active_notes[note as usize] = None;
        }
    }

    /// Poll for MIDI events from the registry and process them (RT-safe).
    ///
    /// Uses `poll_into()` with a pre-allocated buffer — zero heap allocations.
    #[cfg(feature = "midi")]
    fn poll_midi_events(&mut self) {
        use tutti_midi::RawMidiEvent;

        // Poll events from the registry using our get_id()
        if let Some(ref registry) = self.midi_registry {
            let unit_id = self.get_id();
            let count = registry.poll_into(unit_id, &mut self.midi_buffer);

            for i in 0..count {
                let raw: RawMidiEvent = self.midi_buffer[i].into();
                let status = raw.data[0];
                let data1 = raw.data[1];
                let data2 = raw.data[2];

                // Note On (0x90)
                if (0x90..0xA0).contains(&status) {
                    let note = data1;
                    let velocity = data2;
                    if velocity > 0 {
                        self.note_on(note, velocity);
                    } else {
                        self.note_off(note);
                    }
                }
                // Note Off (0x80)
                else if (0x80..0x90).contains(&status) {
                    let note = data1;
                    self.note_off(note);
                }
            }
        }
    }
}

impl AudioUnit for PolySynth {
    fn reset(&mut self) {
        self.voices.clear();
        self.active_notes.fill(None);
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate as f32;
    }

    fn tick(&mut self, _input: &[f32], output: &mut [f32]) {
        #[cfg(feature = "midi")]
        self.poll_midi_events();

        let mut sample = 0.0;
        for voice in &mut self.voices {
            if !voice.is_idle() {
                sample += voice.process(self.waveform, &self.envelope, self.sample_rate);
            }
        }

        // Simple mixing (divide by max voices to prevent clipping)
        sample /= self.max_voices as f32;

        // Stereo output (duplicate mono)
        output[0] = sample;
        output[1] = sample;
    }

    fn process(&mut self, size: usize, _input: &BufferRef, output: &mut BufferMut) {
        #[cfg(feature = "midi")]
        self.poll_midi_events();

        for i in 0..size {
            let mut sample = 0.0;
            for voice in &mut self.voices {
                if !voice.is_idle() {
                    sample += voice.process(self.waveform, &self.envelope, self.sample_rate);
                }
            }

            // Simple mixing
            sample /= self.max_voices as f32;

            // Stereo output
            output.set_f32(0, i, sample);
            output.set_f32(1, i, sample);
        }
    }

    fn inputs(&self) -> usize {
        0
    }

    fn outputs(&self) -> usize {
        2
    }

    fn route(&mut self, _input: &SignalFrame, _frequency: f64) -> SignalFrame {
        SignalFrame::new(self.outputs())
    }

    fn set(&mut self, setting: Setting) {
        // Handle MIDI registry setting (custom extension)
        // FunDSP's Setting is extensible, so we can add custom variants
        // For now, we'll ignore settings since we use set_midi_context() instead
        let _ = setting;
    }

    fn get_id(&self) -> u64 {
        0x504F4C5953594E // "POLYSYN" in hex
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn footprint(&self) -> usize {
        std::mem::size_of::<Self>() + self.voices.capacity() * std::mem::size_of::<Voice>()
    }

    fn allocate(&mut self) {
        self.voices.reserve(self.max_voices);
    }
}

/// Convert MIDI note number to frequency in Hz
fn midi_note_to_hz(note: u8) -> f32 {
    440.0 * 2.0f32.powf((note as f32 - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_midi_note_conversion() {
        assert!((midi_note_to_hz(69) - 440.0).abs() < 0.01); // A4
        assert!((midi_note_to_hz(60) - 261.63).abs() < 0.01); // C4
    }

    #[test]
    fn test_polysynth_creation() {
        let synth = PolySynth::new(44100.0, 8);
        assert_eq!(synth.outputs(), 2);
        assert_eq!(synth.inputs(), 0);
    }
}
