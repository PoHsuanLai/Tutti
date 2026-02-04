//! Polyphonic synthesizer AudioUnit implementation.

use super::voice::SynthVoice;
use super::SynthConfig;
use crate::{
    AllocationResult, ModSourceValues, ModulationMatrix, Portamento, UnisonEngine, VoiceAllocator,
    VoiceAllocatorConfig,
};
use tutti_core::midi::{ChannelVoiceMsg, MidiAudioUnit, MidiEvent};
use tutti_core::{AudioUnit, BufferMut, BufferRef, Shared, SignalFrame};

extern crate alloc;
use alloc::vec::Vec;

/// Polyphonic synthesizer built from tutti-synth building blocks.
///
/// Combines VoiceAllocator, Portamento, UnisonEngine, and ModulationMatrix
/// with FunDSP-based voice DSP chains.
#[derive(Clone)]
pub struct PolySynth {
    config: SynthConfig,
    allocator: VoiceAllocator,
    voices: Vec<SynthVoice>,
    portamento: Option<Portamento>,
    /// Unison engine for detuned voice stacking
    unison: Option<UnisonEngine>,
    modulation: Option<ModulationMatrix>,
    mod_sources: ModSourceValues,

    /// Master volume (0.0 - 1.0)
    master_volume: Shared,

    /// Pending MIDI events
    pending_midi: Vec<MidiEvent>,

    /// Output mix buffer (stereo)
    mix_buffer: [f32; 2],
}

impl PolySynth {
    /// Create a new PolySynth from configuration.
    pub(crate) fn from_config(config: SynthConfig) -> crate::Result<Self> {
        // Validate configuration
        if config.max_voices == 0 {
            return Err(crate::Error::InvalidConfig(
                "max_voices must be at least 1".into(),
            ));
        }

        // Create voice allocator
        let allocator_config = VoiceAllocatorConfig {
            max_voices: config.max_voices,
            mode: config.voice_mode,
            strategy: config.allocation_strategy,
            ..Default::default()
        };
        let allocator = VoiceAllocator::new(allocator_config);

        // Create unison engine
        let unison = config.unison.as_ref().map(|u| UnisonEngine::new(u.clone()));

        // Determine unison count for voice construction
        let unison_count = config.unison.as_ref().map(|u| u.voice_count as usize).unwrap_or(1);

        // Create voices with sub-voices for unison
        let mut voices = Vec::with_capacity(config.max_voices);
        for _ in 0..config.max_voices {
            let mut voice = SynthVoice::from_config(&config, unison_count);
            voice.set_sample_rate(config.sample_rate);
            voices.push(voice);
        }

        // Create portamento
        let portamento = config
            .portamento
            .as_ref()
            .map(|p| Portamento::new(p.clone(), config.sample_rate as f32));

        // Create modulation matrix
        let modulation = config
            .modulation
            .as_ref()
            .map(|m| ModulationMatrix::new(m.clone()));

        let master_volume = tutti_core::shared(1.0);

        Ok(Self {
            config,
            allocator,
            voices,
            portamento,
            unison,
            modulation,
            mod_sources: ModSourceValues::default(),
            master_volume,
            pending_midi: Vec::with_capacity(64),
            mix_buffer: [0.0; 2],
        })
    }

    /// Set master volume (0.0 - 1.0).
    pub fn set_volume(&mut self, volume: f32) {
        self.master_volume.set(volume.clamp(0.0, 1.0));
    }

    /// Get master volume.
    pub fn volume(&self) -> f32 {
        self.master_volume.value()
    }

    /// Get number of active voices.
    pub fn active_voice_count(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    /// Process a single MIDI event.
    fn process_midi_event(&mut self, event: &MidiEvent) {
        match event.msg {
            ChannelVoiceMsg::NoteOn { note, velocity } => {
                if velocity == 0 {
                    // Note on with velocity 0 = note off
                    self.handle_note_off(note, event.channel_num());
                } else {
                    self.handle_note_on(note, velocity, event.channel_num());
                }
            }
            ChannelVoiceMsg::NoteOff { note, .. } => {
                self.handle_note_off(note, event.channel_num());
            }
            ChannelVoiceMsg::ControlChange { control } => {
                self.handle_control_change(control, event.channel_num());
            }
            ChannelVoiceMsg::PitchBend { bend } => {
                // Update mod sources for pitch bend
                self.mod_sources.pitch_bend = (bend as f32 - 8192.0) / 8192.0;
            }
            ChannelVoiceMsg::ChannelPressure { pressure } => {
                self.mod_sources.aftertouch = pressure as f32 / 127.0;
            }
            _ => {}
        }
    }

    fn handle_note_on(&mut self, note: u8, velocity: u8, channel: u8) {
        let vel_norm = velocity as f32 / 127.0;
        let result = self.allocator.allocate(note, channel, vel_norm);

        // Extract slot_index from allocation result
        let slot_index = match result {
            AllocationResult::Allocated { slot_index, .. } => Some(slot_index),
            AllocationResult::Stolen { slot_index, .. } => Some(slot_index),
            AllocationResult::LegatoRetrigger { slot_index, .. } => Some(slot_index),
            AllocationResult::Unavailable => None,
        };

        // Check if this is a legato retrigger (don't retrigger envelope)
        let is_legato = matches!(result, AllocationResult::LegatoRetrigger { .. });

        if let Some(slot_index) = slot_index {
            let freq = self.config.tuning.note_to_freq(note);

            // Handle portamento
            let target_freq = if let Some(ref mut porta) = self.portamento {
                porta.set_target(freq, is_legato);
                porta.current()
            } else {
                freq
            };

            // Trigger voice with unison
            let voice = &mut self.voices[slot_index];
            if is_legato {
                // Legato: just update pitch, don't retrigger
                voice.set_pitch(target_freq, self.unison.as_ref());
                voice.note = note;
            } else {
                voice.note_on(note, vel_norm, target_freq, self.unison.as_mut());
            }

            // Update velocity in mod sources
            self.mod_sources.velocity = vel_norm;
        }
    }

    fn handle_note_off(&mut self, note: u8, channel: u8) {
        self.allocator.release(note, channel);

        // Find and release the voice playing this note
        for voice in &mut self.voices {
            if voice.active && voice.note == note {
                voice.note_off();
                break;
            }
        }
    }

    fn handle_control_change(&mut self, control: tutti_core::midi::ControlChange, channel: u8) {
        use tutti_core::midi::ControlChange;

        match control {
            ControlChange::CC { control: cc, value } => {
                // Update CC in mod sources
                self.mod_sources.cc[cc as usize] = value as f32 / 127.0;

                // Handle special CCs
                match cc {
                    1 => {
                        // Mod wheel
                        self.mod_sources.mod_wheel = value as f32 / 127.0;
                    }
                    64 => {
                        // Sustain pedal
                        self.allocator.sustain_pedal(channel, value >= 64);
                    }
                    66 => {
                        // Sostenuto pedal
                        self.allocator.sostenuto_pedal(channel, value >= 64);
                    }
                    123 => {
                        // All notes off
                        for voice in &mut self.voices {
                            voice.note_off();
                        }
                        self.allocator.all_notes_off(channel);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Mark a voice as finished after envelope completes.
    fn mark_voice_finished(&mut self, note: u8) {
        // Find the voice_id for this note from the allocator's slots
        if let Some(slot_index) = (0..self.voices.len()).find(|&i| self.voices[i].note == note) {
            let slots = self.allocator.slots();
            if slot_index < slots.len() {
                let voice_id = slots[slot_index].voice_id;
                self.allocator.voice_finished(voice_id);
            }
        }
    }
}

impl AudioUnit for PolySynth {
    fn reset(&mut self) {
        for voice in &mut self.voices {
            voice.reset();
        }
        self.allocator.reset();
        if let Some(ref mut porta) = self.portamento {
            // Reset portamento to A4
            porta.reset(440.0);
        }
        self.pending_midi.clear();
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        for voice in &mut self.voices {
            voice.set_sample_rate(sample_rate);
        }
        if let Some(ref mut porta) = self.portamento {
            porta.set_sample_rate(sample_rate as f32);
        }
    }

    fn tick(&mut self, _input: &[f32], output: &mut [f32]) {
        // Process pending MIDI
        let events: Vec<_> = self.pending_midi.drain(..).collect();
        for event in events {
            self.process_midi_event(&event);
        }

        // Update portamento
        if let Some(ref mut porta) = self.portamento {
            let current_freq = porta.tick();
            // Update all active voices with the portamento frequency
            let unison_ref = self.unison.as_ref();
            for voice in &mut self.voices {
                if voice.active {
                    voice.set_pitch(current_freq, unison_ref);
                }
            }
        }

        // Apply modulation matrix
        if let Some(ref mut modulation) = self.modulation {
            let destinations = modulation.compute(&self.mod_sources);
            // Apply filter cutoff modulation to all voices
            let cutoff_mod = destinations.filter_cutoff;
            for voice in &mut self.voices {
                if voice.active {
                    // Base cutoff from config, modulated
                    let base_cutoff = match self.config.filter {
                        super::FilterType::Moog { cutoff, .. } => cutoff,
                        super::FilterType::Svf { cutoff, .. } => cutoff,
                        super::FilterType::Biquad { cutoff, .. } => cutoff,
                        super::FilterType::None => 20000.0,
                    };
                    voice.set_filter_cutoff(base_cutoff * (1.0 + cutoff_mod));
                }
            }
        }

        // Mix all voices with unison panning
        self.mix_buffer = [0.0, 0.0];
        let mut finished_notes = Vec::new();

        for voice in &mut self.voices {
            if voice.active {
                // Process all sub-voices and get stereo output with unison panning
                let (left, right) = voice.tick_stereo(self.unison.as_ref());

                // Check if voice has finished (envelope level near zero and gate off)
                if voice.gate.value() == 0.0 {
                    // Simple heuristic: if output is very quiet, voice is done
                    let level = left.abs().max(right.abs());
                    voice.envelope_level = level;
                    if level < 0.0001 {
                        voice.active = false;
                        finished_notes.push(voice.note);
                    }
                }

                self.mix_buffer[0] += left;
                self.mix_buffer[1] += right;
            }
        }

        // Mark finished voices
        for note in finished_notes {
            self.mark_voice_finished(note);
        }

        // Apply master volume
        let volume = self.master_volume.value();
        output[0] = self.mix_buffer[0] * volume;
        if output.len() > 1 {
            output[1] = self.mix_buffer[1] * volume;
        }
    }

    fn process(&mut self, size: usize, _input: &BufferRef, output: &mut BufferMut) {
        let mut sample_output = [0.0f32; 2];

        for i in 0..size {
            self.tick(&[], &mut sample_output);
            output.set_f32(0, i, sample_output[0]);
            if output.channels() > 1 {
                output.set_f32(1, i, sample_output[1]);
            }
        }
    }

    fn inputs(&self) -> usize {
        0 // Generator
    }

    fn outputs(&self) -> usize {
        2 // Stereo
    }

    fn route(&mut self, _input: &SignalFrame, _frequency: f64) -> SignalFrame {
        SignalFrame::new(self.outputs())
    }

    fn set(&mut self, _setting: tutti_core::Setting) {
        // Could be extended to handle settings
    }

    fn get_id(&self) -> u64 {
        // Unique ID for PolySynth
        0x504F4C5953594E54 // "POLYSYNT" in hex
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }

    fn footprint(&self) -> usize {
        core::mem::size_of::<Self>()
            + self.voices.iter().map(|v| v.footprint()).sum::<usize>()
            + self.pending_midi.capacity() * core::mem::size_of::<MidiEvent>()
    }

    fn allocate(&mut self) {
        for voice in &mut self.voices {
            voice.allocate();
        }
    }
}

impl MidiAudioUnit for PolySynth {
    fn queue_midi(&mut self, events: &[MidiEvent]) {
        self.pending_midi.extend_from_slice(events);
    }

    fn clear_midi(&mut self) {
        self.pending_midi.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{OscillatorType, SynthBuilder};
    use crate::UnisonConfig;

    #[test]
    fn test_polysynth_creation() {
        let synth = SynthBuilder::new(44100.0)
            .poly(4)
            .oscillator(OscillatorType::Saw)
            .envelope(0.01, 0.1, 0.7, 0.2)
            .build();

        assert!(synth.is_ok());
        let synth = synth.unwrap();
        assert_eq!(synth.voices.len(), 4);
    }

    #[test]
    fn test_polysynth_midi() {
        let mut synth = SynthBuilder::new(44100.0)
            .poly(4)
            .oscillator(OscillatorType::Sine)
            .build()
            .unwrap();

        // Queue a note on
        let note_on = MidiEvent::note_on_builder(60, 100).build();
        synth.queue_midi(&[note_on]);

        // Process one sample to trigger the note
        let mut output = [0.0f32; 2];
        synth.tick(&[], &mut output);

        assert_eq!(synth.active_voice_count(), 1);
    }

    #[test]
    fn test_polysynth_volume() {
        let mut synth = SynthBuilder::new(44100.0)
            .poly(2)
            .build()
            .unwrap();

        synth.set_volume(0.5);
        assert!((synth.volume() - 0.5).abs() < 0.001);

        synth.set_volume(1.5); // Should clamp
        assert!((synth.volume() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_unison_creates_subvoices() {
        // Create synth with 3-voice unison
        let synth = SynthBuilder::new(44100.0)
            .poly(4)
            .oscillator(OscillatorType::Saw)
            .unison(UnisonConfig {
                voice_count: 3,
                detune_cents: 15.0,
                stereo_spread: 0.5,
                phase_randomize: false,
            })
            .build()
            .unwrap();

        // Each voice should have 3 sub-voices
        assert_eq!(synth.voices[0].sub_voice_count(), 3);
        assert_eq!(synth.voices[1].sub_voice_count(), 3);

        // Unison engine should be present
        assert!(synth.unison.is_some());
    }

    #[test]
    fn test_unison_stereo_output() {
        let mut synth = SynthBuilder::new(44100.0)
            .poly(2)
            .oscillator(OscillatorType::Saw)
            .envelope(0.001, 0.1, 0.8, 0.1) // Fast attack to get output quickly
            .unison(UnisonConfig {
                voice_count: 3,
                detune_cents: 15.0,
                stereo_spread: 1.0, // Full stereo spread
                phase_randomize: false,
            })
            .build()
            .unwrap();

        // Verify unison is set up
        assert!(synth.unison.is_some());
        assert_eq!(synth.voices[0].sub_voice_count(), 3);

        // Trigger a note
        let note_on = MidiEvent::note_on_builder(60, 100).build();
        synth.queue_midi(&[note_on]);

        // Process samples and accumulate max output
        // Note: FunDSP EnvelopeIn samples at 2ms intervals (~88 samples at 44100Hz)
        // so we need several hundred samples to see envelope output
        let mut output = [0.0f32; 2];
        let mut max_left = 0.0f32;
        let mut max_right = 0.0f32;

        for _ in 0..2000 {
            synth.tick(&[], &mut output);
            max_left = max_left.max(output[0].abs());
            max_right = max_right.max(output[1].abs());
        }

        // With full stereo spread, we should get output on both channels
        // (the left/right sub-voices should be panned to opposite sides)
        assert!(max_left > 0.0, "Expected non-zero left channel output, got {}", max_left);
        assert!(max_right > 0.0, "Expected non-zero right channel output, got {}", max_right);
    }

    #[test]
    fn test_no_unison_single_subvoice() {
        // Create synth without unison
        let synth = SynthBuilder::new(44100.0)
            .poly(4)
            .oscillator(OscillatorType::Sine)
            .build()
            .unwrap();

        // Each voice should have 1 sub-voice (no unison)
        assert_eq!(synth.voices[0].sub_voice_count(), 1);

        // Unison engine should be None
        assert!(synth.unison.is_none());
    }

    #[test]
    fn test_basic_dsp_chain() {
        use tutti_core::dsp::{adsr_live, saw, var};
        use tutti_core::AudioUnit;

        // Create a minimal DSP chain similar to what we use
        let pitch = tutti_core::shared(440.0);
        let gate = tutti_core::shared(0.0); // Gate OFF initially

        // Simple: just saw oscillator - box it to use AudioUnit interface
        let mut osc: Box<dyn AudioUnit> = Box::new(var(&pitch) >> saw());
        osc.set_sample_rate(44100.0);

        let mut out = [0.0f32; 1];
        osc.tick(&[], &mut out);
        assert!(out[0] != 0.0, "Oscillator should produce output");

        // Test envelope with osc - using correct FunDSP pattern from live_adsr example
        let mut chain: Box<dyn AudioUnit> = Box::new(
            var(&pitch) >> saw() * (var(&gate) >> adsr_live(0.001, 0.1, 0.8, 0.1))
        );
        chain.set_sample_rate(44100.0);

        // Process one sample with gate=0 to initialize envelope
        let mut out2 = [0.0f32; 1];
        chain.tick(&[], &mut out2);

        // Trigger gate
        gate.set(1.0);

        // EnvelopeIn samples at 2ms intervals (about 88 samples at 44100Hz)
        // So we need to process more samples to see the envelope respond
        let mut max_out = 0.0f32;
        for _ in 0..500 {
            chain.tick(&[], &mut out2);
            max_out = max_out.max(out2[0].abs());
        }
        assert!(max_out > 0.0001, "Chain should produce output after triggering gate, max={}", max_out);
    }
}
