//! Varispeed and playback direction control.

/// Playback direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayDirection {
    #[default]
    Forward,
    Reverse,
}

impl PlayDirection {
    /// Check if playing forward.
    pub fn is_forward(&self) -> bool {
        matches!(self, Self::Forward)
    }

    /// Check if playing in reverse.
    pub fn is_reverse(&self) -> bool {
        matches!(self, Self::Reverse)
    }
}

/// Varispeed configuration for a stream.
#[derive(Debug, Clone, Copy)]
pub struct Varispeed {
    /// Playback direction
    pub direction: PlayDirection,
    /// Playback speed multiplier (1.0 = normal, 0.5 = half, 2.0 = double)
    pub speed: f32,
}

impl Default for Varispeed {
    fn default() -> Self {
        Self {
            direction: PlayDirection::Forward,
            speed: 1.0,
        }
    }
}

impl Varispeed {
    /// Create forward playback at normal speed.
    pub fn forward() -> Self {
        Self::default()
    }

    /// Create reverse playback at normal speed.
    pub fn reverse() -> Self {
        Self {
            direction: PlayDirection::Reverse,
            speed: 1.0,
        }
    }

    /// Create with custom speed (positive = forward, negative = reverse).
    pub fn with_speed(speed: f32) -> Self {
        if speed < 0.0 {
            Self {
                direction: PlayDirection::Reverse,
                speed: speed.abs(),
            }
        } else {
            Self {
                direction: PlayDirection::Forward,
                speed,
            }
        }
    }

    /// Check if playing forward.
    pub fn is_forward(&self) -> bool {
        self.direction.is_forward()
    }

    /// Check if playing in reverse.
    pub fn is_reverse(&self) -> bool {
        self.direction.is_reverse()
    }

    /// Get effective speed (always positive).
    pub fn effective_speed(&self) -> f32 {
        self.speed.abs().max(0.01) // minimum speed to prevent division by zero
    }

    /// Get signed speed (negative for reverse).
    pub fn signed_speed(&self) -> f32 {
        if self.is_reverse() {
            -self.speed.abs()
        } else {
            self.speed.abs()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_varispeed() {
        let v = Varispeed::default();
        assert!(v.is_forward());
        assert_eq!(v.speed, 1.0);
        assert_eq!(v.signed_speed(), 1.0);
    }

    #[test]
    fn test_reverse_varispeed() {
        let v = Varispeed::reverse();
        assert!(v.is_reverse());
        assert_eq!(v.speed, 1.0);
        assert_eq!(v.signed_speed(), -1.0);
    }

    #[test]
    fn test_with_speed_positive() {
        let v = Varispeed::with_speed(2.0);
        assert!(v.is_forward());
        assert_eq!(v.speed, 2.0);
    }

    #[test]
    fn test_with_speed_negative() {
        let v = Varispeed::with_speed(-1.5);
        assert!(v.is_reverse());
        assert_eq!(v.speed, 1.5);
        assert_eq!(v.signed_speed(), -1.5);
    }

    #[test]
    fn test_effective_speed_minimum() {
        let v = Varispeed::with_speed(0.0);
        assert!(v.effective_speed() >= 0.01);
    }
}
