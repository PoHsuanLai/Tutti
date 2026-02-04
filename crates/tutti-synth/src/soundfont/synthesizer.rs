//! SoundFont synthesizer wrapper

use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use std::sync::Arc;

/// SoundFont synthesizer wrapper.
pub struct SoundFontSynth {
    synthesizer: Synthesizer,
    sample_rate: u32,
}

impl SoundFontSynth {
    /// Create a new SoundFont synthesizer.
    pub fn new(soundfont: Arc<SoundFont>, settings: &SynthesizerSettings) -> Self {
        let synthesizer =
            Synthesizer::new(&soundfont, settings).expect("Failed to create synthesizer");
        Self {
            synthesizer,
            sample_rate: settings.sample_rate as u32,
        }
    }

    /// Note on.
    pub fn note_on(&mut self, channel: i32, key: i32, velocity: i32) {
        self.synthesizer.note_on(channel, key, velocity);
    }

    /// Note off
    ///
    /// # Arguments
    /// * `channel` - MIDI channel (0-15)
    /// * `key` - MIDI note number (0-127)
    pub fn note_off(&mut self, channel: i32, key: i32) {
        self.synthesizer.note_off(channel, key);
    }

    /// Change MIDI program (preset).
    pub fn program_change(&mut self, channel: i32, preset: i32) {
        self.synthesizer
            .process_midi_message(channel, 0xC0, preset, 0);
    }

    /// Render audio samples (buffers must have the same length).
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        assert_eq!(
            left.len(),
            right.len(),
            "Left and right buffers must have the same length"
        );
        self.synthesizer.render(left, right);
    }

    /// Get the sample rate
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Reset the synthesizer
    pub fn reset(&mut self) {
        // RustySynth doesn't have a direct reset method
        // Note offs for all notes on all channels
        (0..16).for_each(|channel| {
            (0..128).for_each(|key| {
                self.note_off(channel, key);
            });
        });
    }
}

#[cfg(test)]
mod tests {
    // Note: These tests would require actual SoundFont files
    // They should be added in integration tests with test fixtures

    #[test]
    fn test_buffer_length_assertion() {
        // This test verifies the assertion in render()
        // We can't create a synthesizer without a SoundFont,
        // so this is just a placeholder for when we have test fixtures
    }
}
