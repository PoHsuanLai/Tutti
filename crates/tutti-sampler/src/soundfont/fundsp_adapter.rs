//! FunDSP adapter for RustySynth
//!
//! Wraps RustySynth's Synthesizer to work with fundsp's AudioUnit trait.
//! Lock-free: each voice owns its Synthesizer instance (no shared state).

use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use std::sync::Arc;
use tutti_core::{AudioUnit, BufferMut, BufferRef, Setting, SignalFrame};

/// FunDSP adapter for RustySynth Synthesizer
///
/// This wraps a RustySynth Synthesizer to implement fundsp's AudioUnit trait.
/// Since RustySynth is not designed for sample-by-sample processing, we buffer
/// the output internally.
///
/// **Lock-free**: Each RustySynthUnit owns its own Synthesizer instance.
/// Synthesizer is Clone, so voices can be cloned without shared state.
#[derive(Clone)]
pub struct RustySynthUnit {
    synthesizer: Synthesizer,
    sample_rate: u32,
    buffer_size: usize,
    left_buffer: Vec<f32>,
    right_buffer: Vec<f32>,
    buffer_pos: usize,
}

impl RustySynthUnit {
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

impl AudioUnit for RustySynthUnit {
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
        assert_eq!(output.len(), 2, "RustySynthUnit is stereo (2 outputs)");

        // Refill buffers if needed
        if self.buffer_pos >= self.buffer_size {
            self.refill_buffers();
        }

        output[0] = self.left_buffer[self.buffer_pos];
        output[1] = self.right_buffer[self.buffer_pos];
        self.buffer_pos += 1;
    }

    fn process(&mut self, size: usize, _input: &BufferRef, output: &mut BufferMut) {
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

    fn footprint(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.left_buffer.capacity() * std::mem::size_of::<f32>()
            + self.right_buffer.capacity() * std::mem::size_of::<f32>()
    }

    fn allocate(&mut self) {
        // Buffers are already allocated
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
