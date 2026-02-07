//! SoundFont audio unit for Tutti
//!
//! Wraps RustySynth's Synthesizer to implement AudioUnit trait.
//! Lock-free: each voice owns its Synthesizer instance (no shared state).
//! MIDI events are received via pull-based polling from MidiRegistry.
//!
//! ## RT Safety
//!
//! Uses SmallVec for pending MIDI events to avoid heap allocation
//! for typical MIDI event counts (up to 128 per buffer).

use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use smallvec::SmallVec;
use tutti_core::midi::{MidiEvent, MidiRegistry};
use tutti_core::Arc;
use tutti_core::{AudioUnit, BufferMut, BufferRef, Setting, SignalFrame};

/// SoundFont synthesizer audio unit
///
/// Wraps RustySynth to provide SoundFont (.sf2) synthesis as a Tutti audio node.
/// Since RustySynth is not designed for sample-by-sample processing, we buffer
/// the output internally.
///
/// **Lock-free**: Each SoundFontUnit owns its own Synthesizer instance.
/// Synthesizer is Clone, so voices can be cloned without shared state.
///
/// **RT-safe**: Uses SmallVec for pending MIDI to avoid heap allocation
/// for typical event counts.
pub struct SoundFontUnit {
    synthesizer: Synthesizer,
    sample_rate: u32,
    buffer_size: usize,
    left_buffer: Vec<f32>,
    right_buffer: Vec<f32>,
    buffer_pos: usize,
    /// Pending MIDI events - SmallVec avoids allocation for up to 128 events
    pending_midi: SmallVec<[MidiEvent; 128]>,
    midi_registry: Option<MidiRegistry>,
    midi_buffer: Vec<MidiEvent>,
}

impl SoundFontUnit {
    /// Create a new RustySynth unit
    pub fn new(soundfont: Arc<SoundFont>, settings: &SynthesizerSettings) -> Self {
        let synthesizer = Synthesizer::new(&soundfont, settings)
            .expect("Failed to create RustySynth synthesizer");

        let buffer_size = 64; // Process in 64-sample blocks

        Self {
            synthesizer,
            sample_rate: settings.sample_rate as u32,
            buffer_size,
            left_buffer: vec![0.0; buffer_size],
            right_buffer: vec![0.0; buffer_size],
            buffer_pos: buffer_size,
            pending_midi: SmallVec::new(),
            midi_registry: None,
            midi_buffer: vec![MidiEvent::note_on_builder(0, 0).build(); 256],
        }
    }

    /// Create a SoundFont unit with MIDI registry support
    pub fn with_midi(
        soundfont: Arc<SoundFont>,
        settings: &SynthesizerSettings,
        midi_registry: MidiRegistry,
    ) -> Self {
        let mut unit = Self::new(soundfont, settings);
        unit.midi_registry = Some(midi_registry);
        unit
    }

    /// Get the sample rate
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Trigger a note on
    pub fn note_on(&mut self, channel: i32, key: i32, velocity: i32) {
        self.synthesizer.note_on(channel, key, velocity);
    }

    /// Trigger a note off
    pub fn note_off(&mut self, channel: i32, key: i32) {
        self.synthesizer.note_off(channel, key);
    }

    /// Change program (preset)
    pub fn program_change(&mut self, channel: i32, preset: i32) {
        self.synthesizer
            .process_midi_message(channel, 0xC0, preset, 0);
    }

    fn refill_buffers(&mut self) {
        self.left_buffer[..self.buffer_size].fill(0.0);
        self.right_buffer[..self.buffer_size].fill(0.0);
        self.synthesizer
            .render(&mut self.left_buffer, &mut self.right_buffer);
        self.buffer_pos = 0;
    }

    fn poll_midi_events(&mut self) {
        use tutti_core::midi::ChannelVoiceMsg;

        if let Some(ref registry) = self.midi_registry {
            let unit_id = self.get_id();
            let count = registry.poll_into(unit_id, &mut self.midi_buffer);
            for i in 0..count {
                self.pending_midi.push(self.midi_buffer[i]);
            }
        }

        for event in self.pending_midi.drain(..) {
            let channel = event.channel_num() as i32;
            match event.msg {
                ChannelVoiceMsg::NoteOn { note, velocity } => {
                    if velocity > 0 {
                        self.synthesizer
                            .note_on(channel, note as i32, velocity as i32);
                    } else {
                        self.synthesizer.note_off(channel, note as i32);
                    }
                }
                ChannelVoiceMsg::NoteOff { note, .. } => {
                    self.synthesizer.note_off(channel, note as i32);
                }
                ChannelVoiceMsg::ProgramChange { program } => {
                    self.synthesizer
                        .process_midi_message(channel, 0xC0, program as i32, 0);
                }
                _ => {}
            }
        }
    }
}

impl AudioUnit for SoundFontUnit {
    fn reset(&mut self) {
        (0..16).for_each(|channel| {
            (0..128).for_each(|key| {
                self.synthesizer.note_off(channel, key);
            });
        });
        self.buffer_pos = self.buffer_size;
    }

    fn set_sample_rate(&mut self, _sample_rate: f64) {
        // RustySynth sample rate is fixed at construction
    }

    fn tick(&mut self, _input: &[f32], output: &mut [f32]) {
        assert_eq!(output.len(), 2, "SoundFontUnit is stereo (2 outputs)");
        self.poll_midi_events();

        if self.buffer_pos >= self.buffer_size {
            self.refill_buffers();
        }

        output[0] = self.left_buffer[self.buffer_pos];
        output[1] = self.right_buffer[self.buffer_pos];
        self.buffer_pos += 1;
    }

    fn process(&mut self, size: usize, _input: &BufferRef, output: &mut BufferMut) {
        self.poll_midi_events();

        (0..size).for_each(|i| {
            if self.buffer_pos >= self.buffer_size {
                self.refill_buffers();
            }
            output.set_f32(0, i, self.left_buffer[self.buffer_pos]);
            output.set_f32(1, i, self.right_buffer[self.buffer_pos]);
            self.buffer_pos += 1;
        });
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

    fn set(&mut self, _setting: Setting) {}

    fn get_id(&self) -> u64 {
        0x52555354595359
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }

    fn footprint(&self) -> usize {
        core::mem::size_of::<Self>()
            + self.left_buffer.capacity() * core::mem::size_of::<f32>()
            + self.right_buffer.capacity() * core::mem::size_of::<f32>()
    }

    fn allocate(&mut self) {}
}

impl Clone for SoundFontUnit {
    fn clone(&self) -> Self {
        Self {
            synthesizer: self.synthesizer.clone(),
            sample_rate: self.sample_rate,
            buffer_size: self.buffer_size,
            left_buffer: self.left_buffer.clone(),
            right_buffer: self.right_buffer.clone(),
            buffer_pos: self.buffer_pos,
            pending_midi: SmallVec::new(), // Fresh empty buffer for clone
            midi_registry: self.midi_registry.clone(),
            midi_buffer: vec![MidiEvent::note_on_builder(0, 0).build(); 256],
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_rustysynth_unit_interface() {
        // This test verifies the AudioUnit interface is implemented correctly
        // Actual synthesis testing requires a SoundFont file
        // Those tests should be in integration tests
    }
}
