//! Audio engine configuration.

use crate::{Error, Result};

/// Configuration for the audio engine.
#[derive(Debug, Clone)]
pub struct TuttiConfig {
    pub sample_rate: f64,
}

impl Default for TuttiConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44100.0,
        }
    }
}

impl TuttiConfig {
    pub fn validate(&self) -> Result<()> {
        if self.sample_rate < 8000.0 || self.sample_rate > 384000.0 {
            return Err(Error::InvalidConfig(format!(
                "sample_rate {} out of range (8000-384000 Hz)",
                self.sample_rate
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = TuttiConfig::default();
        assert_eq!(config.sample_rate, 44100.0);
        assert!(config.validate().is_ok());
    }
}
