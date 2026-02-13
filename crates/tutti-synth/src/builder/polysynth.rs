//! Polyphonic synthesizer implementing [`AudioUnit`].

use super::voice::SynthVoice;
use super::SynthConfig;
use crate::{AllocationResult, Portamento, UnisonEngine, VoiceAllocator, VoiceAllocatorConfig};
use smallvec::SmallVec;
use tutti_core::midi::{ChannelVoiceMsg, MidiEvent, MidiRegistry, MidiSource};
use tutti_core::{AudioUnit, BufferMut, BufferRef, Shared, SignalFrame};

extern crate alloc;
use alloc::vec::Vec;

const FINISHED_NOTES_CAPACITY: usize = 16;

/// Polyphonic synthesizer combining tutti-synth building blocks with FunDSP.
///
/// Created via [`SynthBuilder`](super::SynthBuilder). MIDI events are received
/// via pull-based polling from a [`MidiSource`] during `tick()`/`process()`.
///
/// The synth works with any MIDI source:
/// - **Live**: `MidiRegistry` for real-time MIDI routing
/// - **Export**: `MidiSnapshotReader` for offline rendering
pub struct PolySynth {
    config: SynthConfig,
    allocator: VoiceAllocator,
    voices: Vec<SynthVoice>,
    portamento: Option<Portamento>,
    unison: Option<UnisonEngine>,
    pitch_bend: f32,
    master_volume: Shared,
    id: u64,
    midi_source: Option<Box<dyn MidiSource>>,
    midi_buffer: Vec<MidiEvent>,
    mix_buffer: [f32; 2],
    finished_indices: SmallVec<[usize; FINISHED_NOTES_CAPACITY]>,
}

impl PolySynth {
    pub(crate) fn from_config(config: SynthConfig) -> crate::Result<Self> {
        if config.max_voices == 0 {
            return Err(crate::Error::InvalidConfig(
                "max_voices must be at least 1".into(),
            ));
        }

        let allocator_config = VoiceAllocatorConfig {
            max_voices: config.max_voices,
            mode: config.voice_mode,
            strategy: config.allocation_strategy,
        };
        let allocator = VoiceAllocator::new(allocator_config);

        let unison = config.unison.as_ref().map(|u| UnisonEngine::new(u.clone()));
        let unison_count = config
            .unison
            .as_ref()
            .map(|u| u.voice_count as usize)
            .unwrap_or(1);

        let mut voices = Vec::with_capacity(config.max_voices);
        for _ in 0..config.max_voices {
            let mut voice = SynthVoice::from_config(&config, unison_count);
            voice.set_sample_rate(config.sample_rate);
            voices.push(voice);
        }

        let portamento = config
            .portamento
            .as_ref()
            .map(|p| Portamento::new(p.clone(), config.sample_rate as f32));

        let master_volume = tutti_core::shared(1.0);

        use tutti_core::{AtomicU64, Ordering};
        static ID_COUNTER: AtomicU64 = AtomicU64::new(1);
        let id = ID_COUNTER.fetch_add(1, Ordering::Relaxed);

        Ok(Self {
            config,
            allocator,
            voices,
            portamento,
            unison,
            pitch_bend: 0.0,
            master_volume,
            id,
            midi_source: None,
            midi_buffer: vec![MidiEvent::note_on(0, 0, 0, 0); 256],
            mix_buffer: [0.0; 2],
            finished_indices: SmallVec::new(),
        })
    }

    /// Convenience: set a live `MidiRegistry` as the MIDI source.
    pub fn with_midi_registry(mut self, registry: MidiRegistry) -> Self {
        self.midi_source = Some(Box::new(registry));
        self
    }

    /// Set the MIDI source (live registry or export snapshot reader).
    pub fn set_midi_source(&mut self, source: Box<dyn MidiSource>) {
        self.midi_source = Some(source);
    }

    fn poll_midi_events(&mut self) {
        let source = match &self.midi_source {
            Some(s) => s,
            None => return,
        };
        let count = source.poll_into(self.id, &mut self.midi_buffer);
        for i in 0..count {
            let event = self.midi_buffer[i];
            self.process_midi_event(&event);
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.master_volume.set(volume.clamp(0.0, 1.0));
    }

    pub fn volume(&self) -> f32 {
        self.master_volume.value()
    }

    pub fn active_voice_count(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    pub fn unison_config(&self) -> Option<&crate::UnisonConfig> {
        self.unison.as_ref().map(|u| u.config())
    }

    pub fn unison_voice_count(&self) -> usize {
        self.unison.as_ref().map_or(1, |u| u.voice_count())
    }

    pub fn set_unison_detune(&mut self, cents: f32) {
        if let Some(ref mut unison) = self.unison {
            unison.set_detune(cents);
        }
    }

    pub fn set_unison_stereo_spread(&mut self, spread: f32) {
        if let Some(ref mut unison) = self.unison {
            unison.set_stereo_spread(spread);
        }
    }

    /// Rebuilds sub-voice DSP chains for all polyphonic voices.
    /// New sub-voices start silent and join on the next note-on.
    pub fn set_unison_voice_count(&mut self, count: u8) {
        if let Some(ref mut unison) = self.unison {
            unison.set_voice_count(count);
            let new_count = unison.voice_count();
            for voice in &mut self.voices {
                voice.resize_unison(new_count);
            }
        }
    }

    /// Rebuilds sub-voice DSP chains if the voice count changed.
    pub fn set_unison_config(&mut self, config: crate::UnisonConfig) {
        if let Some(ref mut unison) = self.unison {
            unison.set_config(config);
            let new_count = unison.voice_count();
            for voice in &mut self.voices {
                voice.resize_unison(new_count);
            }
        }
    }

    pub fn seed_unison_rng(&mut self, seed: u32) {
        if let Some(ref mut unison) = self.unison {
            unison.seed_rng(seed);
        }
    }

    pub fn unison_params(&self) -> Option<&[crate::UnisonVoiceParams]> {
        self.unison.as_ref().map(|u| u.all_params())
    }

    fn process_midi_event(&mut self, event: &MidiEvent) {
        match event.msg {
            ChannelVoiceMsg::NoteOn { note, velocity } => {
                if velocity == 0 {
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
                self.pitch_bend = (bend as f32 - 8192.0) / 8192.0;
                self.apply_pitch_bend();
            }
            ChannelVoiceMsg::ChannelPressure { .. } => {}
            _ => {}
        }
    }

    fn handle_note_on(&mut self, note: u8, velocity: u8, channel: u8) {
        let vel_norm = velocity as f32 / 127.0;
        let result = self.allocator.allocate(note, channel, vel_norm);

        let slot_index = match result {
            AllocationResult::Allocated { slot_index } => Some(slot_index),
            AllocationResult::Stolen { slot_index } => Some(slot_index),
            AllocationResult::LegatoRetrigger { slot_index } => Some(slot_index),
            AllocationResult::Unavailable => None,
        };
        let is_legato = matches!(result, AllocationResult::LegatoRetrigger { .. });

        if let Some(slot_index) = slot_index {
            let base_freq = self.config.tuning.fractional_note_to_freq(note as f32);
            let bend_semitones = self.pitch_bend * self.config.pitch_bend_range;
            let bend_multiplier = 2.0_f32.powf(bend_semitones / 12.0);

            let target_freq = if let Some(ref mut porta) = self.portamento {
                porta.set_target(base_freq, is_legato);
                porta.current() * bend_multiplier
            } else {
                base_freq * bend_multiplier
            };

            let voice = &mut self.voices[slot_index];
            voice.set_velocity_mod(vel_norm);
            if is_legato {
                voice.set_pitch(target_freq, self.unison.as_ref());
                voice.note = note;
                voice.channel = channel;
            } else {
                voice.note_on(note, channel, vel_norm, target_freq, self.unison.as_mut());
            }
        }
    }

    fn handle_note_off(&mut self, note: u8, channel: u8) {
        self.allocator.release(note, channel);

        // If the slot is still Active, the note is being held by sustain/sostenuto pedal.
        let slot_still_active = self.allocator.slots().iter().any(|s| {
            s.note == note && s.channel == channel && s.state == crate::voice::VoiceState::Active
        });

        if !slot_still_active {
            if let Some(voice) = self
                .voices
                .iter_mut()
                .find(|v| v.active && v.note == note && v.channel == channel)
            {
                voice.note_off();
            }
        }
    }

    fn handle_control_change(&mut self, control: tutti_core::midi::ControlChange, channel: u8) {
        use tutti_core::midi::ControlChange;

        if let ControlChange::CC { control: cc, value } = control {
            match cc {
                1 => {
                    let norm = value as f32 / 127.0;
                    self.voices.iter_mut().for_each(|v| v.set_mod_wheel(norm));
                }
                71 => {
                    let norm = value as f32 / 127.0;
                    self.voices
                        .iter_mut()
                        .for_each(|v| v.set_filter_resonance(norm));
                }
                74 => {
                    let norm = value as f32 / 127.0;
                    self.voices.iter_mut().for_each(|v| v.set_cc_cutoff(norm));
                }
                64 => {
                    self.allocator.sustain_pedal(channel, value >= 64);
                    if value < 64 {
                        self.sync_voice_gates();
                    }
                }
                66 => {
                    self.allocator.sostenuto_pedal(channel, value >= 64);
                    if value < 64 {
                        self.sync_voice_gates();
                    }
                }
                120 => {
                    self.voices
                        .iter_mut()
                        .filter(|v| v.channel == channel)
                        .for_each(|v| v.reset());
                    self.allocator.all_sound_off(channel);
                }
                123 => {
                    self.voices
                        .iter_mut()
                        .filter(|v| v.active && v.channel == channel)
                        .for_each(|v| v.note_off());
                    self.allocator.all_notes_off(channel);
                }
                _ => {}
            }
        }
    }

    /// If portamento is actively gliding, bend is layered in the tick() loop instead.
    fn apply_pitch_bend(&mut self) {
        if let Some(ref porta) = self.portamento {
            if porta.is_gliding() {
                return;
            }
        }

        let bend_semitones = self.pitch_bend * self.config.pitch_bend_range;
        let tuning = &self.config.tuning;
        let unison = self.unison.as_ref();
        self.voices
            .iter_mut()
            .filter(|v| v.active)
            .for_each(|voice| {
                let bent_freq = tuning.fractional_note_to_freq(voice.note as f32 + bend_semitones);
                voice.set_pitch(bent_freq, unison);
            });
    }

    /// Close the gate on any voice whose allocator slot is Releasing but
    /// whose gate is still open (happens after sustain/sostenuto pedal off).
    fn sync_voice_gates(&mut self) {
        let slots = self.allocator.slots();
        for (i, voice) in self.voices.iter_mut().enumerate() {
            if voice.active
                && voice.gate.value() > 0.0
                && i < slots.len()
                && slots[i].state == crate::voice::VoiceState::Releasing
            {
                voice.note_off();
            }
        }
    }

    fn mark_voice_finished(&mut self, slot_index: usize) {
        let slots = self.allocator.slots();
        if slot_index < slots.len() {
            let voice_id = slots[slot_index].voice_id;
            self.allocator.voice_finished(voice_id);
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
            porta.reset(440.0);
        }
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
        self.poll_midi_events();

        if let Some(ref mut porta) = self.portamento {
            if porta.is_gliding() {
                let porta_freq = porta.tick();
                let bend_semitones = self.pitch_bend * self.config.pitch_bend_range;
                let bend_multiplier = 2.0_f32.powf(bend_semitones / 12.0);
                let freq = porta_freq * bend_multiplier;
                let unison_ref = self.unison.as_ref();
                for voice in &mut self.voices {
                    if voice.active {
                        voice.set_pitch(freq, unison_ref);
                    }
                }
            }
        }

        self.mix_buffer = [0.0, 0.0];
        self.finished_indices.clear();

        for (i, voice) in self.voices.iter_mut().enumerate() {
            if voice.active {
                let (left, right) = voice.tick_stereo(self.unison.as_ref());

                let level = left.abs().max(right.abs());
                voice.envelope_level = level;
                self.allocator.update_envelope_level(i, level);

                if voice.gate.value() == 0.0 && level < 0.0001 {
                    voice.active = false;
                    self.finished_indices.push(i);
                }

                self.mix_buffer[0] += left;
                self.mix_buffer[1] += right;
            }
        }

        for slot_index in core::mem::take(&mut self.finished_indices) {
            self.mark_voice_finished(slot_index);
        }

        self.allocator.advance_time(1);

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
        0
    }

    fn outputs(&self) -> usize {
        2
    }

    fn route(&mut self, _input: &SignalFrame, _frequency: f64) -> SignalFrame {
        SignalFrame::new(self.outputs())
    }

    fn set(&mut self, _setting: tutti_core::Setting) {}

    fn get_id(&self) -> u64 {
        self.id
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
            + self.midi_buffer.capacity() * core::mem::size_of::<MidiEvent>()
    }

    fn allocate(&mut self) {
        for voice in &mut self.voices {
            voice.allocate();
        }
    }
}

impl Clone for PolySynth {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            allocator: self.allocator.clone(),
            voices: self.voices.clone(),
            portamento: self.portamento.clone(),
            unison: self.unison.clone(),
            pitch_bend: self.pitch_bend,
            master_volume: self.master_volume.clone(),
            id: self.id,
            // Cloned synths need explicit MIDI source setup
            midi_source: None,
            midi_buffer: vec![MidiEvent::note_on(0, 0, 0, 0); 256],
            mix_buffer: [0.0; 2],
            finished_indices: SmallVec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{OscillatorType, SynthBuilder};
    use crate::UnisonConfig;

    /// Helper to create a synth with registry and queue MIDI events.
    fn queue_midi_via_registry(
        synth: &mut PolySynth,
        registry: &MidiRegistry,
        events: &[MidiEvent],
    ) {
        registry.queue(synth.get_id(), events);
    }

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
        let registry = MidiRegistry::new();
        let mut synth = SynthBuilder::new(44100.0)
            .poly(4)
            .oscillator(OscillatorType::Sine)
            .build()
            .unwrap()
            .with_midi_registry(registry.clone());

        // Queue a note on via registry
        let note_on = MidiEvent::note_on_builder(60, 100).build();
        queue_midi_via_registry(&mut synth, &registry, &[note_on]);

        // Process one sample to trigger the note
        let mut output = [0.0f32; 2];
        synth.tick(&[], &mut output);

        assert_eq!(synth.active_voice_count(), 1);
    }

    #[test]
    fn test_polysynth_volume() {
        let mut synth = SynthBuilder::new(44100.0).poly(2).build().unwrap();

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
        let registry = MidiRegistry::new();
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
            .unwrap()
            .with_midi_registry(registry.clone());

        // Verify unison is set up
        assert!(synth.unison.is_some());
        assert_eq!(synth.voices[0].sub_voice_count(), 3);

        // Trigger a note via registry
        let note_on = MidiEvent::note_on_builder(60, 100).build();
        queue_midi_via_registry(&mut synth, &registry, &[note_on]);

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
        assert!(
            max_left > 0.0,
            "Expected non-zero left channel output, got {}",
            max_left
        );
        assert!(
            max_right > 0.0,
            "Expected non-zero right channel output, got {}",
            max_right
        );
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

        let pitch = tutti_core::shared(440.0);
        let gate = tutti_core::shared(0.0);

        let mut osc: Box<dyn AudioUnit> = Box::new(var(&pitch) >> saw());
        osc.set_sample_rate(44100.0);

        let mut out = [0.0f32; 1];
        osc.tick(&[], &mut out);
        assert!(out[0] != 0.0, "Oscillator should produce output");

        let mut chain: Box<dyn AudioUnit> =
            Box::new(var(&pitch) >> saw() * (var(&gate) >> adsr_live(0.001, 0.1, 0.8, 0.1)));
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
        assert!(
            max_out > 0.0001,
            "Chain should produce output after triggering gate, max={}",
            max_out
        );
    }

    #[test]
    fn test_dynamic_unison_resize() {
        let registry = MidiRegistry::new();
        // Create synth with 2-voice unison
        let mut synth = SynthBuilder::new(44100.0)
            .poly(2)
            .oscillator(OscillatorType::Saw)
            .unison(UnisonConfig {
                voice_count: 2,
                detune_cents: 10.0,
                stereo_spread: 0.5,
                phase_randomize: false,
            })
            .build()
            .unwrap()
            .with_midi_registry(registry.clone());

        // Initial state: 2 sub-voices per polyphonic voice
        assert_eq!(synth.voices[0].sub_voice_count(), 2);
        assert_eq!(synth.voices[1].sub_voice_count(), 2);
        assert_eq!(synth.unison_voice_count(), 2);

        // Increase to 5 unison voices
        synth.set_unison_voice_count(5);
        assert_eq!(synth.voices[0].sub_voice_count(), 5);
        assert_eq!(synth.voices[1].sub_voice_count(), 5);
        assert_eq!(synth.unison_voice_count(), 5);

        // Decrease to 3 unison voices
        synth.set_unison_voice_count(3);
        assert_eq!(synth.voices[0].sub_voice_count(), 3);
        assert_eq!(synth.voices[1].sub_voice_count(), 3);
        assert_eq!(synth.unison_voice_count(), 3);

        // Verify it still produces sound
        let note_on = MidiEvent::note_on_builder(60, 100).build();
        queue_midi_via_registry(&mut synth, &registry, &[note_on]);

        let mut output = [0.0f32; 2];
        let mut max_out = 0.0f32;
        for _ in 0..2000 {
            synth.tick(&[], &mut output);
            max_out = max_out.max(output[0].abs().max(output[1].abs()));
        }

        assert!(
            max_out > 0.0,
            "Synth should produce output after resize, got max={}",
            max_out
        );
    }

    #[test]
    fn test_note_off_respects_channel() {
        let registry = MidiRegistry::new();
        let mut synth = SynthBuilder::new(44100.0)
            .poly(4)
            .oscillator(OscillatorType::Sine)
            .envelope(0.001, 0.1, 1.0, 0.1)
            .build()
            .unwrap()
            .with_midi_registry(registry.clone());

        // Play same note (C4=60) on channel 0 and channel 1
        // note_on(frame_offset, channel, note, velocity)
        let note_on_ch0 = MidiEvent::note_on(0, 0, 60, 100);
        let note_on_ch1 = MidiEvent::note_on(0, 1, 60, 100);
        queue_midi_via_registry(&mut synth, &registry, &[note_on_ch0, note_on_ch1]);

        // Process to trigger both notes
        let mut output = [0.0f32; 2];
        synth.tick(&[], &mut output);
        assert_eq!(synth.active_voice_count(), 2, "Should have 2 active voices");

        // Note off on channel 0 only
        // note_off(frame_offset, channel, note, velocity)
        let note_off_ch0 = MidiEvent::note_off(0, 0, 60, 0);
        queue_midi_via_registry(&mut synth, &registry, &[note_off_ch0]);
        synth.tick(&[], &mut output);

        // Channel 1's voice should still be active (gate=1.0)
        // Channel 0's voice should be releasing (gate=0.0) but still active
        // until envelope finishes
        let mut ch1_still_gated = false;
        for voice in &synth.voices {
            if voice.active && voice.channel == 1 && voice.note == 60 {
                assert!(
                    voice.gate.value() > 0.0,
                    "Channel 1 voice should still have gate open"
                );
                ch1_still_gated = true;
            }
        }
        assert!(
            ch1_still_gated,
            "Channel 1 voice should still be active and gated"
        );
    }

    // =========================================================================
    // CC Handling Tests
    // =========================================================================

    #[test]
    fn test_cc64_sustain_pedal_holds_notes() {
        let registry = MidiRegistry::new();
        let mut synth = SynthBuilder::new(44100.0)
            .poly(4)
            .oscillator(OscillatorType::Sine)
            .envelope(0.001, 0.0, 1.0, 0.05)
            .build()
            .unwrap()
            .with_midi_registry(registry.clone());

        // Play note
        let note_on = MidiEvent::note_on_builder(60, 100).build();
        queue_midi_via_registry(&mut synth, &registry, &[note_on]);
        let mut output = [0.0f32; 2];
        synth.tick(&[], &mut output);
        assert_eq!(synth.active_voice_count(), 1);

        // Press sustain pedal (CC64 >= 64 = on)
        let sustain_on = MidiEvent::cc_builder(64, 127).build();
        queue_midi_via_registry(&mut synth, &registry, &[sustain_on]);
        synth.tick(&[], &mut output);

        // Release note — voice should stay active (sustained)
        let note_off = MidiEvent::note_off_builder(60).build();
        queue_midi_via_registry(&mut synth, &registry, &[note_off]);
        synth.tick(&[], &mut output);

        // Voice is still active due to sustain pedal
        assert!(
            synth.voices[0].gate.value() > 0.0 || synth.voices[0].active,
            "Voice should still be held by sustain pedal"
        );

        // Release sustain pedal (CC64 < 64 = off)
        let sustain_off = MidiEvent::cc_builder(64, 0).build();
        queue_midi_via_registry(&mut synth, &registry, &[sustain_off]);
        synth.tick(&[], &mut output);

        // Voice should now be releasing (gate off)
        let voice = &synth.voices[0];
        assert!(
            voice.gate.value() == 0.0,
            "Voice should release after sustain pedal off"
        );
    }

    #[test]
    fn test_cc66_sostenuto_pedal_holds_notes() {
        let registry = MidiRegistry::new();
        let mut synth = SynthBuilder::new(44100.0)
            .poly(4)
            .oscillator(OscillatorType::Sine)
            .envelope(0.001, 0.0, 1.0, 0.05)
            .build()
            .unwrap()
            .with_midi_registry(registry.clone());

        // Play note, then press sostenuto
        let note_on = MidiEvent::note_on_builder(60, 100).build();
        queue_midi_via_registry(&mut synth, &registry, &[note_on]);
        let mut output = [0.0f32; 2];
        synth.tick(&[], &mut output);

        let sostenuto_on = MidiEvent::cc_builder(66, 127).build();
        queue_midi_via_registry(&mut synth, &registry, &[sostenuto_on]);
        synth.tick(&[], &mut output);

        // Release note — should be held by sostenuto
        let note_off = MidiEvent::note_off_builder(60).build();
        queue_midi_via_registry(&mut synth, &registry, &[note_off]);
        synth.tick(&[], &mut output);

        assert!(synth.voices[0].active, "Voice should be held by sostenuto");

        // Play a NEW note AFTER sostenuto — this should NOT be held
        let note_on2 = MidiEvent::note_on_builder(64, 100).build();
        queue_midi_via_registry(&mut synth, &registry, &[note_on2]);
        synth.tick(&[], &mut output);
        let note_off2 = MidiEvent::note_off_builder(64).build();
        queue_midi_via_registry(&mut synth, &registry, &[note_off2]);
        synth.tick(&[], &mut output);

        // Second note's voice should be releasing (gate off)
        let voice2 = &synth.voices[1];
        assert_eq!(
            voice2.gate.value(),
            0.0,
            "New note after sostenuto should release normally"
        );

        // Release sostenuto — original note should now release
        let sostenuto_off = MidiEvent::cc_builder(66, 0).build();
        queue_midi_via_registry(&mut synth, &registry, &[sostenuto_off]);
        synth.tick(&[], &mut output);

        assert_eq!(
            synth.voices[0].gate.value(),
            0.0,
            "Original note should release after sostenuto off"
        );
    }

    #[test]
    fn test_cc120_all_sound_off() {
        let registry = MidiRegistry::new();
        let mut synth = SynthBuilder::new(44100.0)
            .poly(4)
            .oscillator(OscillatorType::Sine)
            .envelope(0.001, 0.0, 1.0, 5.0) // Very long release
            .build()
            .unwrap()
            .with_midi_registry(registry.clone());

        // Play multiple notes
        let events: Vec<MidiEvent> = (60..64)
            .map(|n| MidiEvent::note_on_builder(n, 100).build())
            .collect();
        queue_midi_via_registry(&mut synth, &registry, &events);
        let mut output = [0.0f32; 2];
        synth.tick(&[], &mut output);
        assert_eq!(synth.active_voice_count(), 4);

        // CC120 = All Sound Off (immediate silence)
        let all_sound_off = MidiEvent::cc_builder(120, 0).build();
        queue_midi_via_registry(&mut synth, &registry, &[all_sound_off]);
        synth.tick(&[], &mut output);

        // All voices should be immediately reset (not just releasing)
        assert_eq!(
            synth.active_voice_count(),
            0,
            "All Sound Off should immediately silence all voices"
        );
    }

    #[test]
    fn test_cc123_all_notes_off() {
        let registry = MidiRegistry::new();
        let mut synth = SynthBuilder::new(44100.0)
            .poly(4)
            .oscillator(OscillatorType::Sine)
            .envelope(0.001, 0.0, 1.0, 5.0) // Long release
            .build()
            .unwrap()
            .with_midi_registry(registry.clone());

        // Play multiple notes
        let events: Vec<MidiEvent> = (60..64)
            .map(|n| MidiEvent::note_on_builder(n, 100).build())
            .collect();
        queue_midi_via_registry(&mut synth, &registry, &events);
        let mut output = [0.0f32; 2];
        synth.tick(&[], &mut output);
        assert_eq!(synth.active_voice_count(), 4);

        // CC123 = All Notes Off (release with envelope tail)
        let all_notes_off = MidiEvent::cc_builder(123, 0).build();
        queue_midi_via_registry(&mut synth, &registry, &[all_notes_off]);
        synth.tick(&[], &mut output);

        // Voices should still be active (releasing with long tail)
        // but gates should be off
        for voice in &synth.voices {
            if voice.active {
                assert_eq!(
                    voice.gate.value(),
                    0.0,
                    "All Notes Off should release (gate off), not silence"
                );
            }
        }
    }

    #[test]
    fn test_cc123_respects_channel() {
        let registry = MidiRegistry::new();
        let mut synth = SynthBuilder::new(44100.0)
            .poly(4)
            .oscillator(OscillatorType::Sine)
            .envelope(0.001, 0.0, 1.0, 5.0)
            .build()
            .unwrap()
            .with_midi_registry(registry.clone());

        // Play notes on channel 0 and channel 1
        let note_ch0 = MidiEvent::note_on(0, 0, 60, 100);
        let note_ch1 = MidiEvent::note_on(0, 1, 64, 100);
        queue_midi_via_registry(&mut synth, &registry, &[note_ch0, note_ch1]);
        let mut output = [0.0f32; 2];
        synth.tick(&[], &mut output);
        assert_eq!(synth.active_voice_count(), 2);

        // All Notes Off on channel 0 only
        let all_notes_off = MidiEvent::control_change(0, 0, 123, 0);
        queue_midi_via_registry(&mut synth, &registry, &[all_notes_off]);
        synth.tick(&[], &mut output);

        // Channel 1 voice should still have gate open
        let ch1_voice = synth.voices.iter().find(|v| v.active && v.channel == 1);
        assert!(
            ch1_voice.is_some(),
            "Channel 1 voice should still be active"
        );
        assert!(
            ch1_voice.unwrap().gate.value() > 0.0,
            "Channel 1 voice gate should still be open"
        );
    }

    #[test]
    fn test_velocity_zero_note_on_is_note_off() {
        let registry = MidiRegistry::new();
        let mut synth = SynthBuilder::new(44100.0)
            .poly(4)
            .oscillator(OscillatorType::Sine)
            .envelope(0.001, 0.0, 1.0, 0.05)
            .build()
            .unwrap()
            .with_midi_registry(registry.clone());

        // Play note
        let note_on = MidiEvent::note_on_builder(60, 100).build();
        queue_midi_via_registry(&mut synth, &registry, &[note_on]);
        let mut output = [0.0f32; 2];
        synth.tick(&[], &mut output);
        assert_eq!(synth.active_voice_count(), 1);
        assert!(synth.voices[0].gate.value() > 0.0);

        // Note on with velocity 0 = note off (MIDI standard)
        let vel0_off = MidiEvent::note_on(0, 0, 60, 0);
        queue_midi_via_registry(&mut synth, &registry, &[vel0_off]);
        synth.tick(&[], &mut output);

        assert_eq!(
            synth.voices[0].gate.value(),
            0.0,
            "Note-on with velocity 0 should act as note-off"
        );
    }

    // =========================================================================
    // Voice Stealing & Legato Integration Tests
    // =========================================================================

    #[test]
    fn test_voice_stealing_in_polysynth() {
        let registry = MidiRegistry::new();
        let mut synth = SynthBuilder::new(44100.0)
            .poly(2)
            .oscillator(OscillatorType::Sine)
            .envelope(0.001, 0.0, 1.0, 5.0) // Long release so voices stay active
            .build()
            .unwrap()
            .with_midi_registry(registry.clone());

        // Fill all 2 voices
        let note1 = MidiEvent::note_on_builder(60, 100).build();
        let note2 = MidiEvent::note_on_builder(64, 100).build();
        queue_midi_via_registry(&mut synth, &registry, &[note1, note2]);
        let mut output = [0.0f32; 2];
        synth.tick(&[], &mut output);
        assert_eq!(synth.active_voice_count(), 2);

        // Third note should steal a voice
        let note3 = MidiEvent::note_on_builder(67, 100).build();
        queue_midi_via_registry(&mut synth, &registry, &[note3]);
        synth.tick(&[], &mut output);

        // Should still have voices active, with the stolen one now playing note 67
        let has_note67 = synth.voices.iter().any(|v| v.active && v.note == 67);
        assert!(has_note67, "Stolen voice should now play note 67");
    }

    #[test]
    fn test_legato_mode_no_retrigger() {
        use crate::VoiceMode;

        let registry = MidiRegistry::new();
        let mut synth = SynthBuilder::new(44100.0)
            .legato()
            .oscillator(OscillatorType::Sine)
            .envelope(0.001, 0.0, 1.0, 0.1)
            .build()
            .unwrap()
            .with_midi_registry(registry.clone());

        assert_eq!(synth.config.voice_mode, VoiceMode::Legato);

        // First note triggers normally
        let note1 = MidiEvent::note_on_builder(60, 100).build();
        queue_midi_via_registry(&mut synth, &registry, &[note1]);
        let mut output = [0.0f32; 2];
        synth.tick(&[], &mut output);
        assert_eq!(synth.active_voice_count(), 1);
        assert_eq!(synth.voices[0].note, 60);

        // Second note should legato (update pitch, no retrigger)
        let note2 = MidiEvent::note_on_builder(64, 100).build();
        queue_midi_via_registry(&mut synth, &registry, &[note2]);
        synth.tick(&[], &mut output);

        assert_eq!(
            synth.active_voice_count(),
            1,
            "Legato should use same voice"
        );
        assert_eq!(synth.voices[0].note, 64, "Voice should have new note");
        assert!(
            synth.voices[0].gate.value() > 0.0,
            "Gate should stay open (no retrigger)"
        );
    }

    #[test]
    fn test_mono_mode_retrigger() {
        use crate::VoiceMode;

        let registry = MidiRegistry::new();
        let mut synth = SynthBuilder::new(44100.0)
            .mono()
            .oscillator(OscillatorType::Sine)
            .envelope(0.001, 0.0, 1.0, 0.1)
            .build()
            .unwrap()
            .with_midi_registry(registry.clone());

        assert_eq!(synth.config.voice_mode, VoiceMode::Mono);

        // First note
        let note1 = MidiEvent::note_on_builder(60, 100).build();
        queue_midi_via_registry(&mut synth, &registry, &[note1]);
        let mut output = [0.0f32; 2];
        synth.tick(&[], &mut output);
        assert_eq!(synth.voices[0].note, 60);

        // Second note should retrigger (new allocation, not legato)
        let note2 = MidiEvent::note_on_builder(64, 100).build();
        queue_midi_via_registry(&mut synth, &registry, &[note2]);
        synth.tick(&[], &mut output);
        assert_eq!(synth.voices[0].note, 64);
    }

    // =========================================================================
    // Portamento + Pitch Bend Interaction Tests
    // =========================================================================

    #[test]
    fn test_portamento_with_pitch_bend() {
        use crate::{PortamentoConfig, PortamentoCurve, PortamentoMode};

        let registry = MidiRegistry::new();
        let mut synth = SynthBuilder::new(44100.0)
            .poly(2)
            .oscillator(OscillatorType::Sine)
            .envelope(0.001, 0.0, 1.0, 0.1)
            .portamento(PortamentoConfig {
                mode: PortamentoMode::Always,
                curve: PortamentoCurve::Linear,
                time: 0.05, // 50ms glide
                constant_time: true,
            })
            .build()
            .unwrap()
            .with_midi_registry(registry.clone());

        // Play first note to initialize portamento
        let note1 = MidiEvent::note_on_builder(60, 100).build();
        queue_midi_via_registry(&mut synth, &registry, &[note1]);
        let mut output = [0.0f32; 2];
        for _ in 0..4410 {
            synth.tick(&[], &mut output);
        }

        // Play second note — triggers portamento glide
        let note2 = MidiEvent::note_on_builder(72, 100).build();
        queue_midi_via_registry(&mut synth, &registry, &[note2]);
        synth.tick(&[], &mut output);

        // Apply pitch bend while portamento is gliding
        let bend_up = MidiEvent::bend_builder(16383).build();
        queue_midi_via_registry(&mut synth, &registry, &[bend_up]);

        // Process samples during glide — should not crash or produce silence
        let mut max_out = 0.0f32;
        for _ in 0..2205 {
            synth.tick(&[], &mut output);
            max_out = max_out.max(output[0].abs().max(output[1].abs()));
        }

        assert!(
            max_out > 0.0,
            "Should produce audio during portamento+bend, got max={}",
            max_out
        );
    }

    // =========================================================================
    // Error / Edge Case Tests
    // =========================================================================

    #[test]
    fn test_zero_voices_returns_error() {
        let result = SynthBuilder::new(44100.0).poly(0).build();
        assert!(result.is_err(), "max_voices=0 should return error");
    }

    #[test]
    fn test_polysynth_reset() {
        let registry = MidiRegistry::new();
        let mut synth = SynthBuilder::new(44100.0)
            .poly(4)
            .oscillator(OscillatorType::Sine)
            .envelope(0.001, 0.0, 1.0, 5.0)
            .build()
            .unwrap()
            .with_midi_registry(registry.clone());

        // Play notes
        let events: Vec<MidiEvent> = (60..64)
            .map(|n| MidiEvent::note_on_builder(n, 100).build())
            .collect();
        queue_midi_via_registry(&mut synth, &registry, &events);
        let mut output = [0.0f32; 2];
        synth.tick(&[], &mut output);
        assert_eq!(synth.active_voice_count(), 4);

        // Reset
        synth.reset();
        assert_eq!(
            synth.active_voice_count(),
            0,
            "Reset should clear all voices"
        );
    }

    #[test]
    fn test_set_sample_rate() {
        let mut synth = SynthBuilder::new(44100.0)
            .poly(2)
            .oscillator(OscillatorType::Sine)
            .build()
            .unwrap();

        // Should not panic
        synth.set_sample_rate(48000.0);
    }
}
