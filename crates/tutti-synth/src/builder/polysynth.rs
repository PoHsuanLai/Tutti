//! Polyphonic synthesizer implementing [`AudioUnit`].

use super::voice::SynthVoice;
use super::SynthConfig;
use crate::{
    AllocationResult, ModSourceValues, ModulationMatrix, Portamento, UnisonEngine, VoiceAllocator,
    VoiceAllocatorConfig,
};
use smallvec::SmallVec;
use tutti_core::midi::{ChannelVoiceMsg, MidiEvent, MidiRegistry, MidiSnapshot};
use tutti_core::{Arc, AudioUnit, BufferMut, BufferRef, ExportTimeline, Shared, SignalFrame};

extern crate alloc;
use alloc::vec::Vec;

/// Stack capacity for finished notes per tick (covers typical polyphony)
const FINISHED_NOTES_CAPACITY: usize = 16;

/// Polyphonic synthesizer combining tutti-synth building blocks with FunDSP.
///
/// Created via [`SynthBuilder`](super::SynthBuilder). Implements [`AudioUnit`]
/// for audio processing. MIDI events are received via pull-based polling from
/// [`MidiRegistry`] during `tick()`/`process()`.
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

    /// Unique ID for this synth instance (used for MIDI registry lookup)
    id: u64,

    /// MIDI registry for pull-based event delivery
    midi_registry: Option<MidiRegistry>,

    /// Pre-allocated buffer for RT-safe MIDI polling
    midi_buffer: Vec<MidiEvent>,

    /// Export mode: MIDI snapshot for non-destructive event delivery
    midi_snapshot: Option<MidiSnapshot>,

    /// Export mode: Timeline for beat-based MIDI polling
    export_timeline: Option<Arc<ExportTimeline>>,

    /// Export mode: Last polled beat position
    last_poll_beat: f64,

    /// Output mix buffer (stereo)
    mix_buffer: [f32; 2],

    /// Pre-allocated buffer for finished notes (RT-safe)
    finished_notes: SmallVec<[u8; FINISHED_NOTES_CAPACITY]>,
}

impl PolySynth {
    /// Create a new PolySynth from configuration.
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
            ..Default::default()
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

        let modulation = config
            .modulation
            .as_ref()
            .map(|m| ModulationMatrix::new(m.clone()));

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
            modulation,
            mod_sources: ModSourceValues::default(),
            master_volume,
            id,
            midi_registry: None,
            midi_buffer: vec![MidiEvent::note_on(0, 0, 0, 0); 256],
            midi_snapshot: None,
            export_timeline: None,
            last_poll_beat: 0.0,
            mix_buffer: [0.0; 2],
            finished_notes: SmallVec::new(),
        })
    }

    /// Set the MIDI registry for pull-based event delivery.
    ///
    /// When set, the synth will poll for MIDI events during `tick()`/`process()`.
    pub fn with_midi_registry(mut self, registry: MidiRegistry) -> Self {
        self.midi_registry = Some(registry);
        self
    }

    /// Configure the synth for export mode with snapshot and timeline.
    ///
    /// When configured for export:
    /// - MIDI events are read from the snapshot (non-destructive)
    /// - Events are triggered based on beat position from the export timeline
    /// - The live MIDI registry is disabled
    ///
    /// # Example
    /// ```ignore
    /// let mut snapshot = MidiSnapshot::new();
    /// snapshot.add_event(synth.get_id(), 0.0, note_on(60, 100));
    /// snapshot.add_event(synth.get_id(), 1.0, note_off(60));
    ///
    /// synth.configure_for_export(snapshot, export_context.timeline.clone());
    /// ```
    pub fn configure_for_export(&mut self, snapshot: MidiSnapshot, timeline: Arc<ExportTimeline>) {
        self.midi_snapshot = Some(snapshot);
        self.export_timeline = Some(timeline);
        self.midi_registry = None; // Disable live registry in export mode
        self.last_poll_beat = 0.0;
    }

    /// Check if the synth is in export mode.
    pub fn is_export_mode(&self) -> bool {
        self.export_timeline.is_some()
    }

    /// Poll MIDI events from the registry or snapshot.
    ///
    /// Called at the start of `tick()` and `process()`.
    fn poll_midi_events(&mut self) {
        // Export mode: poll from snapshot based on timeline beat
        if let Some(timeline) = &self.export_timeline {
            let current_beat = timeline.current_beat();
            let beat_increment = timeline.beats_per_sample();

            if let Some(ref mut snapshot) = self.midi_snapshot {
                let events = snapshot.poll_range(self.id, self.last_poll_beat, current_beat + beat_increment);
                for event in events {
                    self.process_midi_event(&event);
                }
            }

            self.last_poll_beat = current_beat;
            return;
        }

        // Live mode: poll from MIDI registry
        let registry = match &self.midi_registry {
            Some(r) => r.clone(),
            None => return,
        };

        let count = registry.poll_into(self.id, &mut self.midi_buffer);
        for i in 0..count {
            let event = self.midi_buffer[i];
            self.process_midi_event(&event);
        }
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

    /// Get the current unison configuration, if unison is enabled.
    pub fn unison_config(&self) -> Option<&crate::UnisonConfig> {
        self.unison.as_ref().map(|u| u.config())
    }

    /// Get the number of unison voices (1 if unison is disabled).
    pub fn unison_voice_count(&self) -> usize {
        self.unison.as_ref().map_or(1, |u| u.voice_count())
    }

    /// Set the unison detune amount in cents.
    ///
    /// Takes effect immediately for all active voices on next tick.
    pub fn set_unison_detune(&mut self, cents: f32) {
        if let Some(ref mut unison) = self.unison {
            unison.set_detune(cents);
        }
    }

    /// Set the unison stereo spread (0.0 = mono, 1.0 = full stereo).
    ///
    /// Takes effect immediately for all active voices on next tick.
    pub fn set_unison_stereo_spread(&mut self, spread: f32) {
        if let Some(ref mut unison) = self.unison {
            unison.set_stereo_spread(spread);
        }
    }

    /// Set the number of unison voices.
    ///
    /// This rebuilds sub-voice DSP chains for all polyphonic voices.
    /// New sub-voices start silent and will join on the next note-on.
    pub fn set_unison_voice_count(&mut self, count: u8) {
        if let Some(ref mut unison) = self.unison {
            unison.set_voice_count(count);
            let new_count = unison.voice_count();
            for voice in &mut self.voices {
                voice.resize_unison(new_count);
            }
        }
    }

    /// Replace the entire unison configuration.
    ///
    /// This rebuilds sub-voice DSP chains if the voice count changed.
    pub fn set_unison_config(&mut self, config: crate::UnisonConfig) {
        if let Some(ref mut unison) = self.unison {
            unison.set_config(config);
            let new_count = unison.voice_count();
            for voice in &mut self.voices {
                voice.resize_unison(new_count);
            }
        }
    }

    /// Seed the unison RNG for reproducible phase randomization.
    pub fn seed_unison_rng(&mut self, seed: u32) {
        if let Some(ref mut unison) = self.unison {
            unison.seed_rng(seed);
        }
    }

    /// Get the unison voice parameters as a slice (for inspection/visualization).
    pub fn unison_params(&self) -> Option<&[crate::UnisonVoiceParams]> {
        self.unison.as_ref().map(|u| u.all_params())
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

        let slot_index = match result {
            AllocationResult::Allocated { slot_index, .. } => Some(slot_index),
            AllocationResult::Stolen { slot_index, .. } => Some(slot_index),
            AllocationResult::LegatoRetrigger { slot_index, .. } => Some(slot_index),
            AllocationResult::Unavailable => None,
        };
        let is_legato = matches!(result, AllocationResult::LegatoRetrigger { .. });

        if let Some(slot_index) = slot_index {
            let freq = self.config.tuning.note_to_freq(note);
            let target_freq = if let Some(ref mut porta) = self.portamento {
                porta.set_target(freq, is_legato);
                porta.current()
            } else {
                freq
            };

            let voice = &mut self.voices[slot_index];
            if is_legato {
                voice.set_pitch(target_freq, self.unison.as_ref());
                voice.note = note;
            } else {
                voice.note_on(note, vel_norm, target_freq, self.unison.as_mut());
            }

            self.mod_sources.velocity = vel_norm;
        }
    }

    fn handle_note_off(&mut self, note: u8, channel: u8) {
        self.allocator.release(note, channel);

        for voice in &mut self.voices {
            if voice.active && voice.note == note {
                voice.note_off();
                break;
            }
        }
    }

    fn handle_control_change(&mut self, control: tutti_core::midi::ControlChange, channel: u8) {
        use tutti_core::midi::ControlChange;

        if let ControlChange::CC { control: cc, value } = control {
            self.mod_sources.cc[cc as usize] = value as f32 / 127.0;

            match cc {
                1 => self.mod_sources.mod_wheel = value as f32 / 127.0,
                64 => self.allocator.sustain_pedal(channel, value >= 64),
                66 => self.allocator.sostenuto_pedal(channel, value >= 64),
                123 => {
                    for voice in &mut self.voices {
                        voice.note_off();
                    }
                    self.allocator.all_notes_off(channel);
                }
                _ => {}
            }
        }
    }

    fn mark_voice_finished(&mut self, note: u8) {
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
            let current_freq = porta.tick();
            let unison_ref = self.unison.as_ref();
            for voice in &mut self.voices {
                if voice.active {
                    voice.set_pitch(current_freq, unison_ref);
                }
            }
        }

        if let Some(ref mut modulation) = self.modulation {
            let destinations = modulation.compute(&self.mod_sources);
            let cutoff_mod = destinations.filter_cutoff;
            for voice in &mut self.voices {
                if voice.active {
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

        self.mix_buffer = [0.0, 0.0];
        self.finished_notes.clear();

        for voice in &mut self.voices {
            if voice.active {
                let (left, right) = voice.tick_stereo(self.unison.as_ref());

                // Check if voice envelope has finished
                if voice.gate.value() == 0.0 {
                    let level = left.abs().max(right.abs());
                    voice.envelope_level = level;
                    if level < 0.0001 {
                        voice.active = false;
                        self.finished_notes.push(voice.note);
                    }
                }

                self.mix_buffer[0] += left;
                self.mix_buffer[1] += right;
            }
        }

        for note in core::mem::take(&mut self.finished_notes) {
            self.mark_voice_finished(note);
        }

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
}
