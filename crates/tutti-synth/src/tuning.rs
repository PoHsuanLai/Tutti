//! Microtuning support for synthesizers.
//!
//! Alternative tuning systems: 12-TET, just intonation, Pythagorean, meantone,
//! or custom scales from cents/ratios.
//!
//! Pre-computes a 128-note frequency table for RT-safe lookup.

/// Reference pitch for A4.
pub const A4_FREQ: f32 = 440.0;

/// MIDI note number for A4.
pub const A4_NOTE: u8 = 69;

/// A scale degree in cents from the root.
#[derive(Debug, Clone, Copy)]
pub struct ScaleDegree {
    /// Cents from the scale root (0 = unison, 1200 = octave)
    pub cents: f32,
}

impl ScaleDegree {
    /// Create a scale degree from cents.
    pub fn from_cents(cents: f32) -> Self {
        Self { cents }
    }

    /// Create a scale degree from a frequency ratio.
    pub fn from_ratio(ratio: f32) -> Self {
        Self {
            cents: 1200.0 * ratio.log2(),
        }
    }
}

/// Tuning configuration and frequency lookup table.
///
/// Pre-computes frequencies for all 128 MIDI notes for RT-safe lookup.
#[derive(Debug, Clone)]
pub struct Tuning {
    /// Name of the tuning
    name: String,
    /// Scale degrees (one per scale step)
    degrees: Vec<ScaleDegree>,
    /// Pre-computed frequency table (128 MIDI notes)
    freq_table: [f32; 128],
    /// Reference frequency for A4
    reference_freq: f32,
    /// Reference note (default A4 = 69)
    reference_note: u8,
}

impl Tuning {
    /// Create 12-tone equal temperament (12-TET).
    pub fn equal_temperament() -> Self {
        Self::equal_temperament_with_reference(A4_FREQ, A4_NOTE)
    }

    /// Create 12-TET with custom reference pitch.
    pub fn equal_temperament_with_reference(reference_freq: f32, reference_note: u8) -> Self {
        let degrees: Vec<ScaleDegree> = (0..12)
            .map(|i| ScaleDegree::from_cents(i as f32 * 100.0))
            .collect();

        let mut tuning = Self {
            name: "12-TET".to_string(),
            degrees,
            freq_table: [0.0; 128],
            reference_freq,
            reference_note,
        };
        tuning.recompute_table();
        tuning
    }

    /// Create just intonation tuning (5-limit).
    ///
    /// Based on small integer frequency ratios for pure intervals.
    pub fn just_intonation() -> Self {
        // Just intonation ratios (5-limit)
        let ratios = [
            1.0,         // Unison
            16.0 / 15.0, // Minor second
            9.0 / 8.0,   // Major second
            6.0 / 5.0,   // Minor third
            5.0 / 4.0,   // Major third
            4.0 / 3.0,   // Perfect fourth
            45.0 / 32.0, // Augmented fourth / tritone
            3.0 / 2.0,   // Perfect fifth
            8.0 / 5.0,   // Minor sixth
            5.0 / 3.0,   // Major sixth
            9.0 / 5.0,   // Minor seventh
            15.0 / 8.0,  // Major seventh
        ];

        let degrees: Vec<ScaleDegree> =
            ratios.iter().map(|r| ScaleDegree::from_ratio(*r)).collect();

        let mut tuning = Self {
            name: "Just Intonation".to_string(),
            degrees,
            freq_table: [0.0; 128],
            reference_freq: A4_FREQ,
            reference_note: A4_NOTE,
        };
        tuning.recompute_table();
        tuning
    }

    /// Create Pythagorean tuning.
    ///
    /// Based on perfect fifths (3:2 ratio).
    pub fn pythagorean() -> Self {
        // Pythagorean ratios (circle of fifths)
        let ratios = [
            1.0,           // C
            256.0 / 243.0, // C#/Db
            9.0 / 8.0,     // D
            32.0 / 27.0,   // D#/Eb
            81.0 / 64.0,   // E
            4.0 / 3.0,     // F
            729.0 / 512.0, // F#/Gb
            3.0 / 2.0,     // G
            128.0 / 81.0,  // G#/Ab
            27.0 / 16.0,   // A
            16.0 / 9.0,    // A#/Bb
            243.0 / 128.0, // B
        ];

        let degrees: Vec<ScaleDegree> =
            ratios.iter().map(|r| ScaleDegree::from_ratio(*r)).collect();

        let mut tuning = Self {
            name: "Pythagorean".to_string(),
            degrees,
            freq_table: [0.0; 128],
            reference_freq: A4_FREQ,
            reference_note: A4_NOTE,
        };
        tuning.recompute_table();
        tuning
    }

    /// Create meantone temperament (quarter-comma).
    pub fn meantone() -> Self {
        // Quarter-comma meantone ratios
        let fifth = 5.0_f32.powf(0.25); // Tempered fifth
        let mut cents = Vec::with_capacity(12);

        // Build scale from tempered fifths
        for i in 0..12 {
            let c = match i {
                0 => 0.0,
                1 => 76.0,    // Db
                2 => 193.0,   // D
                3 => 310.0,   // Eb
                4 => 386.0,   // E (pure major third)
                5 => 503.0,   // F
                6 => 579.0,   // F#
                7 => 697.0,   // G
                8 => 773.0,   // Ab
                9 => 890.0,   // A
                10 => 1007.0, // Bb
                11 => 1083.0, // B
                _ => 0.0,
            };
            cents.push(ScaleDegree::from_cents(c));
        }

        let mut tuning = Self {
            name: "Meantone".to_string(),
            degrees: cents,
            freq_table: [0.0; 128],
            reference_freq: A4_FREQ,
            reference_note: A4_NOTE,
        };
        let _ = fifth; // Silence unused warning
        tuning.recompute_table();
        tuning
    }

    /// Create a tuning from cents values for each scale degree.
    ///
    /// The scale repeats every octave (1200 cents).
    pub fn from_cents(cents: &[f32]) -> Self {
        let degrees: Vec<ScaleDegree> = cents.iter().map(|c| ScaleDegree::from_cents(*c)).collect();

        let mut tuning = Self {
            name: "Custom".to_string(),
            degrees,
            freq_table: [0.0; 128],
            reference_freq: A4_FREQ,
            reference_note: A4_NOTE,
        };
        tuning.recompute_table();
        tuning
    }

    /// Create a tuning from frequency ratios.
    pub fn from_ratios(ratios: &[f32]) -> Self {
        let degrees: Vec<ScaleDegree> =
            ratios.iter().map(|r| ScaleDegree::from_ratio(*r)).collect();

        let mut tuning = Self {
            name: "Custom".to_string(),
            degrees,
            freq_table: [0.0; 128],
            reference_freq: A4_FREQ,
            reference_note: A4_NOTE,
        };
        tuning.recompute_table();
        tuning
    }

    /// Recompute the frequency lookup table.
    fn recompute_table(&mut self) {
        let scale_size = self.degrees.len();
        if scale_size == 0 {
            // Fallback to 12-TET
            for note in 0..128 {
                let semitones = note as f32 - self.reference_note as f32;
                self.freq_table[note] = self.reference_freq * 2.0_f32.powf(semitones / 12.0);
            }
            return;
        }

        // Reference note's position in the scale
        let ref_scale_pos = (self.reference_note as usize) % scale_size;
        let ref_octave = (self.reference_note as i32) / (scale_size as i32);

        // Compute frequency for each MIDI note
        for note in 0..128 {
            let note_scale_pos = (note as usize) % scale_size;
            let note_octave = (note as i32) / (scale_size as i32);

            // Get cents offset within scale
            let note_cents = self.degrees[note_scale_pos].cents;
            let ref_cents = self.degrees[ref_scale_pos].cents;

            // Total cents from reference
            let octave_diff = note_octave - ref_octave;
            let cents_diff = note_cents - ref_cents + (octave_diff as f32 * 1200.0);

            // Convert cents to frequency
            self.freq_table[note] = self.reference_freq * 2.0_f32.powf(cents_diff / 1200.0);
        }
    }

    /// Get frequency for a MIDI note.
    ///
    /// RT-safe: simple array lookup.
    #[inline]
    pub fn note_to_freq(&self, note: u8) -> f32 {
        self.freq_table[note as usize]
    }

    /// Get frequency for a fractional MIDI note (for pitch bend, portamento).
    ///
    /// Linearly interpolates between adjacent notes.
    #[inline]
    pub fn fractional_note_to_freq(&self, note: f32) -> f32 {
        if note <= 0.0 {
            return self.freq_table[0];
        }
        if note >= 127.0 {
            return self.freq_table[127];
        }

        let low = note.floor() as usize;
        let high = (low + 1).min(127);
        let frac = note - low as f32;

        // Interpolate in log space for musical pitch
        let low_freq = self.freq_table[low];
        let high_freq = self.freq_table[high];

        (low_freq.ln() + (high_freq.ln() - low_freq.ln()) * frac).exp()
    }

    /// Get the tuning name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Set the tuning name.
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    /// Get the scale degrees.
    pub fn degrees(&self) -> &[ScaleDegree] {
        &self.degrees
    }

    /// Get the number of scale degrees (notes per octave).
    pub fn scale_size(&self) -> usize {
        self.degrees.len()
    }

    /// Set reference pitch.
    pub fn set_reference(&mut self, freq: f32, note: u8) {
        self.reference_freq = freq;
        self.reference_note = note;
        self.recompute_table();
    }

    /// Get reference frequency.
    pub fn reference_freq(&self) -> f32 {
        self.reference_freq
    }

    /// Get reference note.
    pub fn reference_note(&self) -> u8 {
        self.reference_note
    }
}

impl Default for Tuning {
    fn default() -> Self {
        Self::equal_temperament()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_equal_temperament() {
        let tuning = Tuning::equal_temperament();

        // A4 should be 440 Hz
        assert!((tuning.note_to_freq(69) - 440.0).abs() < 0.01);

        // A5 should be 880 Hz (octave above)
        assert!((tuning.note_to_freq(81) - 880.0).abs() < 0.01);

        // A3 should be 220 Hz (octave below)
        assert!((tuning.note_to_freq(57) - 220.0).abs() < 0.01);

        // Middle C (C4) should be ~261.6 Hz
        assert!((tuning.note_to_freq(60) - 261.63).abs() < 0.1);
    }

    #[test]
    fn test_custom_reference() {
        let tuning = Tuning::equal_temperament_with_reference(432.0, 69);

        // A4 should be 432 Hz
        assert!((tuning.note_to_freq(69) - 432.0).abs() < 0.01);
    }

    #[test]
    fn test_just_intonation() {
        let tuning = Tuning::just_intonation();

        // Reference should still be A4 = 440 Hz
        let a4 = tuning.note_to_freq(69);
        assert!((a4 - 440.0).abs() < 0.01);

        // Perfect fifth above A4 (E5) should be 3/2 ratio
        // In just intonation from C, perfect fifth is 3/2
        // E5 is note 76, A4 is note 69
        // This is 7 semitones, so it's a perfect fifth from A to E
        let e5 = tuning.note_to_freq(76);
        let ratio = e5 / a4;
        // Should be close to 3/2 = 1.5
        assert!((ratio - 1.5).abs() < 0.02);
    }

    #[test]
    fn test_fractional_note() {
        let tuning = Tuning::equal_temperament();

        let a4 = tuning.note_to_freq(69);
        let a4_50 = tuning.fractional_note_to_freq(69.5);
        let bb4 = tuning.note_to_freq(70);

        // Fractional note should be between
        assert!(a4_50 > a4);
        assert!(a4_50 < bb4);

        // Should be roughly in the middle (geometrically)
        let expected = (a4 * bb4).sqrt();
        assert!((a4_50 - expected).abs() < 0.1);
    }

    #[test]
    fn test_from_cents() {
        // Create quarter-tone scale (24 notes per octave)
        let cents: Vec<f32> = (0..24).map(|i| i as f32 * 50.0).collect();
        let tuning = Tuning::from_cents(&cents);

        assert_eq!(tuning.scale_size(), 24);

        // Note 0 and note 24 should be an octave apart
        let freq_0 = tuning.note_to_freq(0);
        let freq_24 = tuning.note_to_freq(24);
        let ratio = freq_24 / freq_0;
        assert!((ratio - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_from_ratios() {
        // Simple Pythagorean pentatonic
        let ratios = [1.0, 9.0 / 8.0, 81.0 / 64.0, 3.0 / 2.0, 27.0 / 16.0];
        let tuning = Tuning::from_ratios(&ratios);

        assert_eq!(tuning.scale_size(), 5);
    }

    #[test]
    fn test_boundary_notes() {
        let tuning = Tuning::equal_temperament();

        // Should not panic at boundaries
        let _low = tuning.note_to_freq(0);
        let _high = tuning.note_to_freq(127);

        // Fractional boundaries
        let _low_frac = tuning.fractional_note_to_freq(-1.0);
        let _high_frac = tuning.fractional_note_to_freq(128.0);
    }
}
