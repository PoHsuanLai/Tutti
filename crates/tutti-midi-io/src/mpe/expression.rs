//! Lock-free per-note expression state for MPE.

use tutti_core::{AtomicFlag, AtomicFloat};

/// Lock-free per-note expression state.
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
    /// Create new per-note expression state
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

    /// Mark a note as active and reset its expression values
    #[inline]
    pub fn note_on(&self, note: u8) {
        if note < 128 {
            self.pitch_bend[note as usize].set(0.0);
            self.pressure[note as usize].set(0.0);
            self.slide[note as usize].set(0.5);
            self.active[note as usize].set(true);
        }
    }

    /// Mark a note as inactive
    #[inline]
    pub fn note_off(&self, note: u8) {
        if note < 128 {
            self.active[note as usize].set(false);
        }
    }

    /// Set pitch bend for a specific note (-1.0 to 1.0)
    #[inline]
    pub fn set_pitch_bend(&self, note: u8, value: f32) {
        if note < 128 {
            self.pitch_bend[note as usize].set(value.clamp(-1.0, 1.0));
        }
    }

    /// Set pressure for a specific note (0.0 to 1.0)
    #[inline]
    pub fn set_pressure(&self, note: u8, value: f32) {
        if note < 128 {
            self.pressure[note as usize].set(value.clamp(0.0, 1.0));
        }
    }

    /// Set slide (CC74) for a specific note (0.0 to 1.0)
    #[inline]
    pub fn set_slide(&self, note: u8, value: f32) {
        if note < 128 {
            self.slide[note as usize].set(value.clamp(0.0, 1.0));
        }
    }

    /// Set global pitch bend (affects all notes)
    #[inline]
    pub fn set_global_pitch_bend(&self, value: f32) {
        self.global_pitch_bend.set(value.clamp(-1.0, 1.0));
    }

    /// Set global pressure (affects all notes)
    #[inline]
    pub fn set_global_pressure(&self, value: f32) {
        self.global_pressure.set(value.clamp(0.0, 1.0));
    }

    /// Get pitch bend for a note (combined per-note + global)
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

    /// Get per-note pitch bend only
    #[inline]
    pub fn get_pitch_bend_per_note(&self, note: u8) -> f32 {
        if note < 128 {
            self.pitch_bend[note as usize].get()
        } else {
            0.0
        }
    }

    /// Get global pitch bend
    #[inline]
    pub fn get_pitch_bend_global(&self) -> f32 {
        self.global_pitch_bend.get()
    }

    /// Get pressure for a note (max of per-note and global)
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

    /// Get per-note pressure only
    #[inline]
    pub fn get_pressure_per_note(&self, note: u8) -> f32 {
        if note < 128 {
            self.pressure[note as usize].get()
        } else {
            0.0
        }
    }

    /// Get slide for a note
    #[inline]
    pub fn get_slide(&self, note: u8) -> f32 {
        if note < 128 {
            self.slide[note as usize].get()
        } else {
            0.5
        }
    }

    /// Check if a note is currently active
    #[inline]
    pub fn is_active(&self, note: u8) -> bool {
        if note < 128 {
            self.active[note as usize].get()
        } else {
            false
        }
    }

    /// Reset all expression values
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
}
