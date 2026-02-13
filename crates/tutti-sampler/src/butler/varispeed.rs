//! Varispeed and playback direction control.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayDirection {
    #[default]
    Forward,
    Reverse,
}

impl PlayDirection {
    pub fn is_forward(&self) -> bool {
        matches!(self, Self::Forward)
    }

    pub fn is_reverse(&self) -> bool {
        matches!(self, Self::Reverse)
    }
}

/// Varispeed configuration for a stream.
#[derive(Debug, Clone, Copy)]
pub struct Varispeed {
    pub direction: PlayDirection,
    /// 1.0 = normal, 0.5 = half, 2.0 = double
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
    pub fn reverse() -> Self {
        Self {
            direction: PlayDirection::Reverse,
            speed: 1.0,
        }
    }

    pub fn is_forward(&self) -> bool {
        self.direction.is_forward()
    }

    pub fn is_reverse(&self) -> bool {
        self.direction.is_reverse()
    }

    /// Always positive, minimum 0.01.
    pub fn effective_speed(&self) -> f32 {
        self.speed.abs().max(0.01)
    }

    /// Negative for reverse.
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
    fn test_effective_speed_minimum() {
        let v = Varispeed {
            speed: 0.0,
            ..Default::default()
        };
        assert!(v.effective_speed() >= 0.01);
    }
}
