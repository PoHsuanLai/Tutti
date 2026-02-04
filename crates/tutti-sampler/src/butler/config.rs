//! Butler thread configuration.

/// Configuration for butler thread buffer sizes and I/O parameters.
#[derive(Debug, Clone, Copy)]
pub struct BufferConfig {
    /// Buffer duration in seconds (default: 10.0)
    pub buffer_seconds: f64,
    /// Disk read chunk size in samples (default: 16384, aligned to 16KB)
    pub chunk_size: usize,
    /// Capture flush threshold in samples (default: 8192)
    pub flush_threshold: usize,
    /// Maximum number of cached audio files (default: 64)
    pub cache_max_entries: usize,
    /// Maximum total bytes for cache (default: 1GB)
    pub cache_max_bytes: u64,
    /// Seek crossfade length in samples (default: 512, ~12ms @ 44.1kHz)
    pub seek_crossfade_samples: usize,
    /// Speed ramp duration in samples (default: 1024, ~23ms @ 44.1kHz)
    pub speed_ramp_samples: u32,
    /// Enable parallel I/O refill using rayon (default: true)
    /// When enabled, multiple streams are refilled concurrently.
    /// Disable for debugging or single-threaded environments.
    pub parallel_io: bool,
}

impl Default for BufferConfig {
    fn default() -> Self {
        Self {
            buffer_seconds: 10.0,
            chunk_size: 16384,
            flush_threshold: 8192,
            cache_max_entries: 64,
            cache_max_bytes: 1024 * 1024 * 1024, // 1GB
            seek_crossfade_samples: 512,
            speed_ramp_samples: 1024,
            parallel_io: true,
        }
    }
}

impl BufferConfig {
    /// Create config with custom buffer duration.
    pub fn with_buffer_seconds(seconds: f64) -> Self {
        Self {
            buffer_seconds: seconds.max(1.0), // minimum 1 second
            ..Default::default()
        }
    }

    /// Calculate buffer size in samples for a given sample rate.
    pub fn buffer_samples(&self, sample_rate: f64) -> usize {
        ((self.buffer_seconds * sample_rate) as usize).max(4096)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = BufferConfig::default();
        assert_eq!(config.buffer_seconds, 10.0);
        assert_eq!(config.chunk_size, 16384);
        assert_eq!(config.flush_threshold, 8192);
        assert_eq!(config.cache_max_entries, 64);
        assert_eq!(config.cache_max_bytes, 1024 * 1024 * 1024);
        assert_eq!(config.seek_crossfade_samples, 512);
        assert_eq!(config.speed_ramp_samples, 1024);
        assert!(config.parallel_io);
    }

    #[test]
    fn test_buffer_samples() {
        let config = BufferConfig::with_buffer_seconds(5.0);
        assert_eq!(config.buffer_samples(44100.0), 220500);
        assert_eq!(config.buffer_samples(48000.0), 240000);
    }

    #[test]
    fn test_minimum_buffer() {
        let config = BufferConfig::with_buffer_seconds(0.001); // very small
        assert_eq!(config.buffer_seconds, 1.0); // clamped to 1 second
    }
}
