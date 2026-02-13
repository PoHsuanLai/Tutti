//! SoundFont audio unit wrapping RustySynth.

use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use smallvec::SmallVec;
use tutti_core::midi::{MidiEvent, MidiRegistry, MidiSource};
use tutti_core::Arc;
use tutti_core::{AudioUnit, BufferMut, BufferRef, Setting, SignalFrame};

/// Buffers RustySynth output internally since it doesn't support sample-by-sample processing.
/// Each instance owns its own Synthesizer (lock-free, Clone-safe).
pub struct SoundFontUnit {
    synthesizer: Synthesizer,
    sample_rate: u32,
    buffer_size: usize,
    left_buffer: Vec<f32>,
    right_buffer: Vec<f32>,
    buffer_pos: usize,
    pending_midi: SmallVec<[MidiEvent; 128]>,
    midi_source: Option<Box<dyn MidiSource>>,
    midi_buffer: Vec<MidiEvent>,
}

impl SoundFontUnit {
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
            midi_source: None,
            midi_buffer: vec![MidiEvent::note_on_builder(0, 0).build(); 256],
        }
    }

    pub fn with_midi(
        soundfont: Arc<SoundFont>,
        settings: &SynthesizerSettings,
        midi_registry: MidiRegistry,
    ) -> Self {
        let mut unit = Self::new(soundfont, settings);
        unit.midi_source = Some(Box::new(midi_registry));
        unit
    }

    /// Set the MIDI source (live registry or export snapshot reader).
    pub fn set_midi_source(&mut self, source: Box<dyn MidiSource>) {
        self.midi_source = Some(source);
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn note_on(&mut self, channel: i32, key: i32, velocity: i32) {
        self.synthesizer.note_on(channel, key, velocity);
    }

    pub fn note_off(&mut self, channel: i32, key: i32) {
        self.synthesizer.note_off(channel, key);
    }

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

        if let Some(ref source) = self.midi_source {
            let unit_id = self.get_id();
            let count = source.poll_into(unit_id, &mut self.midi_buffer);
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
                ChannelVoiceMsg::PitchBend { bend } => {
                    let lsb = (bend & 0x7F) as i32;
                    let msb = ((bend >> 7) & 0x7F) as i32;
                    self.synthesizer
                        .process_midi_message(channel, 0xE0, lsb, msb);
                }
                ChannelVoiceMsg::ControlChange { control: tutti_core::midi::ControlChange::CC { control: cc, value } } => {
                    self.synthesizer.process_midi_message(
                        channel,
                        0xB0,
                        cc as i32,
                        value as i32,
                    );
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
            pending_midi: SmallVec::new(),
            // Cloned units need explicit MIDI source setup
            midi_source: None,
            midi_buffer: vec![MidiEvent::note_on_builder(0, 0).build(); 256],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Get path to test SoundFont (if available)
    fn test_soundfont_path() -> Option<PathBuf> {
        // CARGO_MANIFEST_DIR is crates/tutti-synth, go up to crates/tutti
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent() // crates/
            .unwrap()
            .parent() // tutti/
            .unwrap()
            .join("assets/soundfonts/TimGM6mb.sf2");
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }

    /// Load test SoundFont if available
    fn load_test_soundfont() -> Option<Arc<SoundFont>> {
        test_soundfont_path().and_then(|path| {
            let mut file = std::fs::File::open(&path).ok()?;
            SoundFont::new(&mut file).ok().map(Arc::new)
        })
    }

    /// Calculate RMS of stereo samples
    fn rms(samples: &[(f32, f32)]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum_sq: f32 = samples.iter().map(|(l, r)| l * l + r * r).sum();
        (sum_sq / (samples.len() * 2) as f32).sqrt()
    }

    /// Render N samples from a SoundFontUnit
    fn render_samples(unit: &mut SoundFontUnit, count: usize) -> Vec<(f32, f32)> {
        let mut samples = Vec::with_capacity(count);
        for _ in 0..count {
            let mut output = [0.0f32; 2];
            unit.tick(&[], &mut output);
            samples.push((output[0], output[1]));
        }
        samples
    }

    #[test]
    fn test_note_on_produces_audio() {
        let sf = match load_test_soundfont() {
            Some(sf) => sf,
            None => {
                eprintln!("Skipping: test soundfont not found");
                return;
            }
        };

        let settings = SynthesizerSettings::new(44100);
        let mut unit = SoundFontUnit::new(sf, &settings);

        // Play middle C
        unit.note_on(0, 60, 100);

        let samples = render_samples(&mut unit, 2000);
        let level = rms(&samples);

        assert!(level > 0.001, "Note should produce audio, RMS={}", level);
    }

    #[test]
    fn test_velocity_affects_volume() {
        let sf = match load_test_soundfont() {
            Some(sf) => sf,
            None => {
                eprintln!("Skipping: test soundfont not found");
                return;
            }
        };

        // Soft note
        let settings = SynthesizerSettings::new(44100);
        let mut unit_soft = SoundFontUnit::new(Arc::clone(&sf), &settings);
        unit_soft.note_on(0, 60, 30);
        let samples_soft = render_samples(&mut unit_soft, 2000);
        let rms_soft = rms(&samples_soft);

        // Loud note
        let mut unit_loud = SoundFontUnit::new(sf, &settings);
        unit_loud.note_on(0, 60, 127);
        let samples_loud = render_samples(&mut unit_loud, 2000);
        let rms_loud = rms(&samples_loud);

        assert!(
            rms_loud > rms_soft,
            "Loud note (vel=127, RMS={}) should be louder than soft (vel=30, RMS={})",
            rms_loud,
            rms_soft
        );
    }

    #[test]
    fn test_note_off_stops_sound() {
        let sf = match load_test_soundfont() {
            Some(sf) => sf,
            None => {
                eprintln!("Skipping: test soundfont not found");
                return;
            }
        };

        let settings = SynthesizerSettings::new(44100);
        let mut unit = SoundFontUnit::new(sf, &settings);

        // Play note
        unit.note_on(0, 60, 100);
        let samples_playing = render_samples(&mut unit, 500);
        let rms_playing = rms(&samples_playing);

        // Release note
        unit.note_off(0, 60);

        // Wait for release to complete (longer for piano sounds)
        let _ = render_samples(&mut unit, 20000);

        // Now should be much quieter
        let samples_after = render_samples(&mut unit, 1000);
        let rms_after = rms(&samples_after);

        assert!(
            rms_after < rms_playing * 0.1,
            "After note off and decay, RMS={} should be much less than playing RMS={}",
            rms_after,
            rms_playing
        );
    }

    #[test]
    fn test_polyphony_multiple_notes() {
        let sf = match load_test_soundfont() {
            Some(sf) => sf,
            None => {
                eprintln!("Skipping: test soundfont not found");
                return;
            }
        };

        let settings = SynthesizerSettings::new(44100);

        // Single note
        let mut unit_single = SoundFontUnit::new(Arc::clone(&sf), &settings);
        unit_single.note_on(0, 60, 80);
        let samples_single = render_samples(&mut unit_single, 2000);
        let rms_single = rms(&samples_single);

        // Chord (3 notes)
        let mut unit_chord = SoundFontUnit::new(sf, &settings);
        unit_chord.note_on(0, 60, 80); // C
        unit_chord.note_on(0, 64, 80); // E
        unit_chord.note_on(0, 67, 80); // G
        let samples_chord = render_samples(&mut unit_chord, 2000);
        let rms_chord = rms(&samples_chord);

        assert!(
            rms_chord > rms_single,
            "Chord RMS={} should be louder than single note RMS={}",
            rms_chord,
            rms_single
        );
    }

    #[test]
    fn test_reset_silences_all_notes() {
        let sf = match load_test_soundfont() {
            Some(sf) => sf,
            None => {
                eprintln!("Skipping: test soundfont not found");
                return;
            }
        };

        let settings = SynthesizerSettings::new(44100);
        let mut unit = SoundFontUnit::new(sf, &settings);

        // Play several notes
        unit.note_on(0, 60, 100);
        unit.note_on(0, 64, 100);
        unit.note_on(0, 67, 100);

        // Confirm audio is playing
        let samples_playing = render_samples(&mut unit, 500);
        let rms_playing = rms(&samples_playing);
        assert!(rms_playing > 0.001);

        // Reset
        unit.reset();

        // Wait for any release to complete
        let _ = render_samples(&mut unit, 20000);

        // Should be silent
        let samples_after = render_samples(&mut unit, 1000);
        let rms_after = rms(&samples_after);

        assert!(
            rms_after < 0.001,
            "After reset and decay, should be silent, RMS={}",
            rms_after
        );
    }

    #[test]
    fn test_clone_creates_independent_instance() {
        let sf = match load_test_soundfont() {
            Some(sf) => sf,
            None => {
                eprintln!("Skipping: test soundfont not found");
                return;
            }
        };

        let settings = SynthesizerSettings::new(44100);
        let mut unit = SoundFontUnit::new(sf, &settings);

        // Play note on original
        unit.note_on(0, 60, 100);
        let _ = render_samples(&mut unit, 100);

        // Clone
        let mut clone = unit.clone();

        // Clone should NOT have the note playing (fresh state)
        // Note: the synthesizer itself is cloned with state, but pending_midi is fresh
        // Actually RustySynth clones the synthesizer state, so both will have the note

        // But we can verify they're independent by playing different notes
        clone.note_on(0, 72, 100); // Different note on clone

        let samples_original = render_samples(&mut unit, 1000);
        let samples_clone = render_samples(&mut clone, 1000);

        // Both should produce audio (unit has C4, clone has C4+C5)
        let rms_original = rms(&samples_original);
        let rms_clone = rms(&samples_clone);

        assert!(rms_original > 0.001);
        assert!(rms_clone > 0.001);
        // Clone has extra note, should be louder
        assert!(rms_clone > rms_original);
    }
}
