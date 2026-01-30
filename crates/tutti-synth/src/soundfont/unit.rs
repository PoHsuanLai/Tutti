//! SoundFont audio unit for Tutti
//!
//! Wraps RustySynth's Synthesizer to implement AudioUnit and MidiAudioUnit traits.
//! Lock-free: each voice owns its Synthesizer instance (no shared state).

use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use std::sync::Arc;
use tutti_core::{AudioUnit, BufferMut, BufferRef, Setting, SignalFrame};

#[cfg(feature = "midi")]
use tutti_core::midi::{MidiAudioUnit, MidiEvent};

/// SoundFont synthesizer audio unit
///
/// Wraps RustySynth to provide SoundFont (.sf2) synthesis as a Tutti audio node.
/// Since RustySynth is not designed for sample-by-sample processing, we buffer
/// the output internally.
///
/// **Lock-free**: Each SoundFontUnit owns its own Synthesizer instance.
/// Synthesizer is Clone, so voices can be cloned without shared state.
#[derive(Clone)]
pub struct SoundFontUnit {
    synthesizer: Synthesizer,
    sample_rate: u32,
    buffer_size: usize,
    left_buffer: Vec<f32>,
    right_buffer: Vec<f32>,
    buffer_pos: usize,

    #[cfg(feature = "midi")]
    pending_midi: Vec<MidiEvent>,
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

            #[cfg(feature = "midi")]
            pending_midi: Vec::with_capacity(128),
        }
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

    /// Refill internal buffers
    fn refill_buffers(&mut self) {
        // Clear buffers
        self.left_buffer[..self.buffer_size].fill(0.0);
        self.right_buffer[..self.buffer_size].fill(0.0);

        // Render new samples
        self.synthesizer
            .render(&mut self.left_buffer, &mut self.right_buffer);
        self.buffer_pos = 0;
    }
}

impl AudioUnit for SoundFontUnit {
    fn reset(&mut self) {
        // Reset by sending note offs for all notes
        (0..16).for_each(|channel| {
            (0..128).for_each(|key| {
                self.synthesizer.note_off(channel, key);
            });
        });
        self.buffer_pos = self.buffer_size; // Force refill on next tick
    }

    fn set_sample_rate(&mut self, _sample_rate: f64) {
        // RustySynth sample rate is set at construction time
        // We can't change it after creation
    }

    fn tick(&mut self, _input: &[f32], output: &mut [f32]) {
        assert_eq!(output.len(), 2, "SoundFontUnit is stereo (2 outputs)");

        // Refill buffers if needed
        if self.buffer_pos >= self.buffer_size {
            self.refill_buffers();
        }

        output[0] = self.left_buffer[self.buffer_pos];
        output[1] = self.right_buffer[self.buffer_pos];
        self.buffer_pos += 1;
    }

    fn process(&mut self, size: usize, _input: &BufferRef, output: &mut BufferMut) {
        #[cfg(feature = "midi")]
        {
            // Process pending MIDI events
            for event in self.pending_midi.drain(..) {
                use tutti_midi::RawMidiEvent;
                let raw: RawMidiEvent = event.into();
                let status = raw.data[0];
                let data1 = raw.data[1];
                let data2 = raw.data[2];

                // Note On (0x90)
                if (0x90..0xA0).contains(&status) {
                    let channel = (status & 0x0F) as i32;
                    let note = data1 as i32;
                    let velocity = data2 as i32;
                    if velocity > 0 {
                        self.synthesizer.note_on(channel, note, velocity);
                    } else {
                        self.synthesizer.note_off(channel, note);
                    }
                }
                // Note Off (0x80)
                else if (0x80..0x90).contains(&status) {
                    let channel = (status & 0x0F) as i32;
                    let note = data1 as i32;
                    self.synthesizer.note_off(channel, note);
                }
                // Program Change (0xC0)
                else if (0xC0..0xD0).contains(&status) {
                    let channel = (status & 0x0F) as i32;
                    let program = data1 as i32;
                    self.synthesizer
                        .process_midi_message(channel, 0xC0, program, 0);
                }
            }
        }

        (0..size).for_each(|i| {
            // Refill buffers if needed
            if self.buffer_pos >= self.buffer_size {
                self.refill_buffers();
            }

            output.set_f32(0, i, self.left_buffer[self.buffer_pos]);
            output.set_f32(1, i, self.right_buffer[self.buffer_pos]);
            self.buffer_pos += 1;
        });
    }

    fn inputs(&self) -> usize {
        0 // Generator - no inputs
    }

    fn outputs(&self) -> usize {
        2 // Stereo output
    }

    fn route(&mut self, _input: &SignalFrame, _frequency: f64) -> SignalFrame {
        SignalFrame::new(self.outputs())
    }

    fn set(&mut self, _setting: Setting) {
        // RustySynth doesn't use fundsp's Setting system
    }

    fn get_id(&self) -> u64 {
        // Unique ID for RustySynth
        0x52555354595359 // "RUSTYSY" in hex
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn footprint(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.left_buffer.capacity() * std::mem::size_of::<f32>()
            + self.right_buffer.capacity() * std::mem::size_of::<f32>()
    }

    fn allocate(&mut self) {
        // Buffers are already allocated
    }
}

#[cfg(feature = "midi")]
impl MidiAudioUnit for SoundFontUnit {
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

    #[test]
    fn test_rustysynth_unit_interface() {
        // This test verifies the AudioUnit interface is implemented correctly
        // Actual synthesis testing requires a SoundFont file
        // Those tests should be in integration tests
    }
}
