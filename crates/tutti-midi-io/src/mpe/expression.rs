//! Lock-free per-note expression state for MPE.

use tutti_core::{AtomicFlag, AtomicFloat};

/// Lock-free per-note expression state. All methods are safe to call from the audio thread.
pub struct PerNoteExpression {
    pitch_bend: [AtomicFloat; 128],
    pressure: [AtomicFloat; 128],
    slide: [AtomicFloat; 128],
    active: [AtomicFlag; 128],
    global_pitch_bend: AtomicFloat,
    global_pressure: AtomicFloat,
}

impl Default for PerNoteExpression {
    fn default() -> Self {
        Self::new()
    }
}

impl PerNoteExpression {
    pub fn new() -> Self {
        Self {
            pitch_bend: std::array::from_fn(|_| AtomicFloat::new(0.0)),
            pressure: std::array::from_fn(|_| AtomicFloat::new(0.0)),
            slide: std::array::from_fn(|_| AtomicFloat::new(0.5)), // CC74 default is center
            active: std::array::from_fn(|_| AtomicFlag::new(false)),
            global_pitch_bend: AtomicFloat::new(0.0),
            global_pressure: AtomicFloat::new(0.0),
        }
    }

    /// Resets per-note pitch bend, pressure, and slide to defaults.
    #[inline]
    pub fn note_on(&self, note: u8) {
        if note < 128 {
            self.pitch_bend[note as usize].set(0.0);
            self.pressure[note as usize].set(0.0);
            self.slide[note as usize].set(0.5);
            self.active[note as usize].set(true);
        }
    }

    #[inline]
    pub fn note_off(&self, note: u8) {
        if note < 128 {
            self.active[note as usize].set(false);
        }
    }

    /// `value`: -1.0 to 1.0
    #[inline]
    pub fn set_pitch_bend(&self, note: u8, value: f32) {
        if note < 128 {
            self.pitch_bend[note as usize].set(value.clamp(-1.0, 1.0));
        }
    }

    /// `value`: 0.0 to 1.0
    #[inline]
    pub fn set_pressure(&self, note: u8, value: f32) {
        if note < 128 {
            self.pressure[note as usize].set(value.clamp(0.0, 1.0));
        }
    }

    /// CC74 slide. `value`: 0.0 to 1.0
    #[inline]
    pub fn set_slide(&self, note: u8, value: f32) {
        if note < 128 {
            self.slide[note as usize].set(value.clamp(0.0, 1.0));
        }
    }

    /// `value`: -1.0 to 1.0, added to per-note bend.
    #[inline]
    pub fn set_global_pitch_bend(&self, value: f32) {
        self.global_pitch_bend.set(value.clamp(-1.0, 1.0));
    }

    /// `value`: 0.0 to 1.0, combined with per-note via max().
    #[inline]
    pub fn set_global_pressure(&self, value: f32) {
        self.global_pressure.set(value.clamp(0.0, 1.0));
    }

    /// Combined per-note + global pitch bend, clamped to -1.0..1.0.
    #[inline]
    pub fn get_pitch_bend(&self, note: u8) -> f32 {
        if note < 128 {
            let per_note = self.pitch_bend[note as usize].get();
            let global = self.global_pitch_bend.get();
            (per_note + global).clamp(-1.0, 1.0)
        } else {
            0.0
        }
    }

    #[inline]
    pub fn get_pitch_bend_per_note(&self, note: u8) -> f32 {
        if note < 128 {
            self.pitch_bend[note as usize].get()
        } else {
            0.0
        }
    }

    #[inline]
    pub fn get_pitch_bend_global(&self) -> f32 {
        self.global_pitch_bend.get()
    }

    /// Returns max(per-note, global) pressure.
    #[inline]
    pub fn get_pressure(&self, note: u8) -> f32 {
        if note < 128 {
            let per_note = self.pressure[note as usize].get();
            let global = self.global_pressure.get();
            per_note.max(global)
        } else {
            0.0
        }
    }

    #[inline]
    pub fn get_pressure_per_note(&self, note: u8) -> f32 {
        if note < 128 {
            self.pressure[note as usize].get()
        } else {
            0.0
        }
    }

    /// Returns 0.5 (CC74 center) for inactive/out-of-range notes.
    #[inline]
    pub fn get_slide(&self, note: u8) -> f32 {
        if note < 128 {
            self.slide[note as usize].get()
        } else {
            0.5
        }
    }

    #[inline]
    pub fn is_active(&self, note: u8) -> bool {
        if note < 128 {
            self.active[note as usize].get()
        } else {
            false
        }
    }

    pub fn reset(&self) {
        for i in 0..128 {
            self.pitch_bend[i].set(0.0);
            self.pressure[i].set(0.0);
            self.slide[i].set(0.5);
            self.active[i].set(false);
        }
        self.global_pitch_bend.set(0.0);
        self.global_pressure.set(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_per_note_expression() {
        let expr = PerNoteExpression::new();

        expr.note_on(60);
        assert!(expr.is_active(60));

        expr.set_pitch_bend(60, 0.5);
        assert!((expr.get_pitch_bend(60) - 0.5).abs() < 0.001);

        expr.set_pressure(60, 0.75);
        assert!((expr.get_pressure(60) - 0.75).abs() < 0.001);

        expr.set_slide(60, 0.3);
        assert!((expr.get_slide(60) - 0.3).abs() < 0.001);

        expr.note_off(60);
        assert!(!expr.is_active(60));
    }

    #[test]
    fn test_global_expression() {
        let expr = PerNoteExpression::new();

        expr.note_on(60);
        expr.set_pitch_bend(60, 0.2);
        expr.set_global_pitch_bend(0.3);

        // Combined pitch bend should be 0.5
        assert!((expr.get_pitch_bend(60) - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_expression_clamping() {
        let expr = PerNoteExpression::new();

        expr.set_pitch_bend(60, 2.0);
        assert!((expr.get_pitch_bend(60) - 1.0).abs() < 0.001);

        expr.set_pitch_bend(60, -2.0);
        assert!((expr.get_pitch_bend(60) - (-1.0)).abs() < 0.001);

        expr.set_pressure(60, 1.5);
        assert!((expr.get_pressure(60) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_per_note_vs_global_pitch_bend_isolation() {
        let expr = PerNoteExpression::new();

        expr.set_pitch_bend(60, 0.3);
        expr.set_global_pitch_bend(0.5);

        // get_pitch_bend_per_note should return only per-note part
        assert!((expr.get_pitch_bend_per_note(60) - 0.3).abs() < 0.001);
        // get_pitch_bend_global should return only global part
        assert!((expr.get_pitch_bend_global() - 0.5).abs() < 0.001);
        // get_pitch_bend should return combined (0.8)
        assert!((expr.get_pitch_bend(60) - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_combined_pitch_bend_clamps() {
        let expr = PerNoteExpression::new();

        // Per-note 0.8 + global 0.5 = 1.3, should clamp to 1.0
        expr.set_pitch_bend(60, 0.8);
        expr.set_global_pitch_bend(0.5);
        assert!((expr.get_pitch_bend(60) - 1.0).abs() < 0.001);

        // Per-note -0.8 + global -0.5 = -1.3, should clamp to -1.0
        expr.set_pitch_bend(60, -0.8);
        expr.set_global_pitch_bend(-0.5);
        assert!((expr.get_pitch_bend(60) - (-1.0)).abs() < 0.001);
    }

    #[test]
    fn test_pressure_uses_max_of_per_note_and_global() {
        let expr = PerNoteExpression::new();

        expr.set_pressure(60, 0.3);
        expr.set_global_pressure(0.7);

        // get_pressure returns max(per_note, global) = 0.7
        assert!((expr.get_pressure(60) - 0.7).abs() < 0.001);

        // get_pressure_per_note returns just per_note = 0.3
        assert!((expr.get_pressure_per_note(60) - 0.3).abs() < 0.001);

        // When per_note > global
        expr.set_pressure(60, 0.9);
        assert!((expr.get_pressure(60) - 0.9).abs() < 0.001);
    }

    #[test]
    fn test_note_on_resets_expression() {
        let expr = PerNoteExpression::new();

        // Set some expression values
        expr.set_pitch_bend(60, 0.5);
        expr.set_pressure(60, 0.75);
        expr.set_slide(60, 0.8);

        // note_on should reset all per-note values
        expr.note_on(60);
        assert!((expr.get_pitch_bend_per_note(60)).abs() < 0.001, "Pitch bend should reset to 0");
        assert!((expr.get_pressure_per_note(60)).abs() < 0.001, "Pressure should reset to 0");
        assert!((expr.get_slide(60) - 0.5).abs() < 0.001, "Slide should reset to 0.5 (center)");
        assert!(expr.is_active(60));
    }

    #[test]
    fn test_slide_default_is_center() {
        let expr = PerNoteExpression::new();

        // Default slide is 0.5 (CC74 center)
        assert!((expr.get_slide(60) - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_reset_clears_everything() {
        let expr = PerNoteExpression::new();

        // Set up various state
        expr.note_on(60);
        expr.note_on(72);
        expr.set_pitch_bend(60, 0.5);
        expr.set_pressure(72, 0.8);
        expr.set_slide(60, 0.9);
        expr.set_global_pitch_bend(0.3);
        expr.set_global_pressure(0.6);

        expr.reset();

        // All per-note state should be cleared
        assert!(!expr.is_active(60));
        assert!(!expr.is_active(72));
        assert!((expr.get_pitch_bend_per_note(60)).abs() < 0.001);
        assert!((expr.get_pressure_per_note(72)).abs() < 0.001);
        assert!((expr.get_slide(60) - 0.5).abs() < 0.001); // Reset to default
        // Global should be cleared
        assert!((expr.get_pitch_bend_global()).abs() < 0.001);
        assert!((expr.get_pressure(60)).abs() < 0.001); // max(0, 0) = 0
    }

    #[test]
    fn test_out_of_range_note_returns_defaults() {
        let expr = PerNoteExpression::new();

        // note >= 128 should return safe defaults
        assert!(!expr.is_active(128));
        assert!(!expr.is_active(255));
        assert!((expr.get_pitch_bend(128)).abs() < 0.001);
        assert!((expr.get_pitch_bend_per_note(200)).abs() < 0.001);
        assert!((expr.get_pressure(128)).abs() < 0.001);
        assert!((expr.get_pressure_per_note(128)).abs() < 0.001);
        assert!((expr.get_slide(128) - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_out_of_range_note_set_does_not_panic() {
        let expr = PerNoteExpression::new();

        // These should silently do nothing (no panic)
        expr.note_on(128);
        expr.note_off(255);
        expr.set_pitch_bend(128, 0.5);
        expr.set_pressure(200, 0.5);
        expr.set_slide(255, 0.5);
    }

    #[test]
    fn test_multiple_notes_independent() {
        let expr = PerNoteExpression::new();

        expr.note_on(60);
        expr.note_on(72);

        expr.set_pitch_bend(60, 0.5);
        expr.set_pitch_bend(72, -0.3);
        expr.set_pressure(60, 0.8);

        // Values should be independent
        assert!((expr.get_pitch_bend_per_note(60) - 0.5).abs() < 0.001);
        assert!((expr.get_pitch_bend_per_note(72) - (-0.3)).abs() < 0.001);
        assert!((expr.get_pressure_per_note(60) - 0.8).abs() < 0.001);
        assert!((expr.get_pressure_per_note(72)).abs() < 0.001);

        // note_off only affects that note
        expr.note_off(60);
        assert!(!expr.is_active(60));
        assert!(expr.is_active(72));
    }

    #[test]
    fn test_slide_clamping() {
        let expr = PerNoteExpression::new();

        expr.set_slide(60, 1.5);
        assert!((expr.get_slide(60) - 1.0).abs() < 0.001);

        expr.set_slide(60, -0.5);
        assert!((expr.get_slide(60)).abs() < 0.001);
    }

    #[test]
    fn test_global_pressure_clamping() {
        let expr = PerNoteExpression::new();

        expr.set_global_pressure(1.5);
        assert!((expr.get_pressure(60) - 1.0).abs() < 0.001);

        expr.set_global_pressure(-0.5);
        assert!((expr.get_pressure(60)).abs() < 0.001);
    }
}
