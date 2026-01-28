//! Sample playback nodes.
//!
//! - `SamplerUnit`: In-memory playback
//! - `StreamingSamplerUnit`: Disk streaming playback

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tutti_core::{AudioUnit, BufferMut, BufferRef, SignalFrame, Wave};

/// In-memory sample playback.
pub struct SamplerUnit {
    wave: Arc<Wave>,
    position: AtomicU64,

    playing: AtomicBool,

    looping: AtomicBool,

    gain: f32,

    speed: f32,

    sample_rate: f32,
}

impl Clone for SamplerUnit {
    fn clone(&self) -> Self {
        Self {
            wave: Arc::clone(&self.wave),
            position: AtomicU64::new(self.position.load(Ordering::Relaxed)),
            playing: AtomicBool::new(self.playing.load(Ordering::Relaxed)),
            looping: AtomicBool::new(self.looping.load(Ordering::Relaxed)),
            gain: self.gain,
            speed: self.speed,
            sample_rate: self.sample_rate,
        }
    }
}

impl SamplerUnit {
    /// Create new sampler.
    pub fn new(wave: Arc<Wave>) -> Self {
        let sample_rate = wave.sample_rate() as f32;
        Self {
            wave,
            position: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            looping: AtomicBool::new(false),
            gain: 1.0,
            speed: 1.0,
            sample_rate,
        }
    }

    /// Create with custom settings.
    pub fn with_settings(wave: Arc<Wave>, gain: f32, speed: f32, looping: bool) -> Self {
        let sample_rate = wave.sample_rate() as f32;
        Self {
            wave,
            position: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            looping: AtomicBool::new(looping),
            gain,
            speed,
            sample_rate,
        }
    }

    /// Trigger playback from start.
    pub fn trigger(&self) {
        self.position.store(0, Ordering::Relaxed);
        self.playing.store(true, Ordering::Relaxed);
    }

    /// Trigger playback from position.
    pub fn trigger_at(&self, position: u64) {
        self.position.store(position, Ordering::Relaxed);
        self.playing.store(true, Ordering::Relaxed);
    }

    /// Stop playback.
    pub fn stop(&self) {
        self.playing.store(false, Ordering::Relaxed);
    }

    /// Check if playing.
    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    /// Set loop mode.
    pub fn set_looping(&self, looping: bool) {
        self.looping.store(looping, Ordering::Relaxed);
    }

    /// Check if looping.
    pub fn is_looping(&self) -> bool {
        self.looping.load(Ordering::Relaxed)
    }

    /// Get current position.
    pub fn position(&self) -> u64 {
        self.position.load(Ordering::Relaxed)
    }

    /// Get duration in samples.
    pub fn duration_samples(&self) -> usize {
        self.wave.len()
    }

    /// Get duration in seconds.
    pub fn duration_seconds(&self) -> f64 {
        self.wave.duration()
    }

    #[inline]
    fn get_sample(&self, position: f64) -> (f32, f32) {
        let len = self.wave.len() as f64;
        if position >= len {
            return (0.0, 0.0);
        }

        let idx = position.floor() as usize;
        let frac = position.fract() as f32;

        // Get current sample
        let (l0, r0) = if self.wave.channels() >= 2 {
            (self.wave.at(0, idx), self.wave.at(1, idx))
        } else {
            let mono = self.wave.at(0, idx);
            (mono, mono)
        };

        // Get next sample for interpolation
        let next_idx = (idx + 1).min(self.wave.len().saturating_sub(1));
        let (l1, r1) = if self.wave.channels() >= 2 {
            (self.wave.at(0, next_idx), self.wave.at(1, next_idx))
        } else {
            let mono = self.wave.at(0, next_idx);
            (mono, mono)
        };

        // Linear interpolation
        let left = l0 + (l1 - l0) * frac;
        let right = r0 + (r1 - r0) * frac;

        (left * self.gain, right * self.gain)
    }
}

impl AudioUnit for SamplerUnit {
    fn inputs(&self) -> usize {
        0 // Sampler generates audio, no inputs
    }

    fn outputs(&self) -> usize {
        2 // Stereo output
    }

    fn reset(&mut self) {
        self.position.store(0, Ordering::Relaxed);
        self.playing.store(false, Ordering::Relaxed);
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate as f32;
    }

    fn tick(&mut self, _input: &[f32], output: &mut [f32]) {
        if !self.playing.load(Ordering::Relaxed) {
            // Not playing - output silence
            if output.len() >= 2 {
                output[0] = 0.0;
                output[1] = 0.0;
            }
            return;
        }

        // Get current position as float
        let pos_bits = self.position.load(Ordering::Relaxed);
        let pos = f64::from_bits(pos_bits);

        // Get sample with interpolation
        let (left, right) = self.get_sample(pos);

        if output.len() >= 2 {
            output[0] = left;
            output[1] = right;
        }

        // Advance position
        let new_pos = pos + self.speed as f64;
        let len = self.wave.len() as f64;

        if new_pos >= len {
            if self.looping.load(Ordering::Relaxed) {
                // Wrap around
                let wrapped = new_pos % len;
                self.position.store(wrapped.to_bits(), Ordering::Relaxed);
            } else {
                // Stop playback
                self.playing.store(false, Ordering::Relaxed);
                self.position.store(len.to_bits(), Ordering::Relaxed);
            }
        } else {
            self.position.store(new_pos.to_bits(), Ordering::Relaxed);
        }
    }

    fn process(&mut self, size: usize, _input: &BufferRef, output: &mut BufferMut) {
        if !self.playing.load(Ordering::Relaxed) {
            // Not playing - output silence
            for i in 0..size {
                output.set_f32(0, i, 0.0);
                output.set_f32(1, i, 0.0);
            }
            return;
        }

        let mut pos_bits = self.position.load(Ordering::Relaxed);
        let len = self.wave.len() as f64;
        let looping = self.looping.load(Ordering::Relaxed);

        for i in 0..size {
            let pos = f64::from_bits(pos_bits);

            if pos >= len {
                if looping {
                    let wrapped = pos % len;
                    pos_bits = wrapped.to_bits();
                } else {
                    // Stop and output silence for remaining samples
                    self.playing.store(false, Ordering::Relaxed);
                    for j in i..size {
                        output.set_f32(0, j, 0.0);
                        output.set_f32(1, j, 0.0);
                    }
                    break;
                }
            }

            let (left, right) = self.get_sample(f64::from_bits(pos_bits));
            output.set_f32(0, i, left);
            output.set_f32(1, i, right);

            // Advance position
            let new_pos = f64::from_bits(pos_bits) + self.speed as f64;
            pos_bits = new_pos.to_bits();
        }

        self.position.store(pos_bits, Ordering::Relaxed);
    }

    fn get_id(&self) -> u64 {
        // Use wave pointer as unique ID
        Arc::as_ptr(&self.wave) as *const () as usize as u64
    }

    fn route(&mut self, _input: &SignalFrame, _frequency: f64) -> SignalFrame {
        // Sampler outputs signal, doesn't route
        SignalFrame::new(2)
    }

    fn footprint(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

mod streaming {
    use super::*;
    use crate::butler::RegionBufferConsumer;
    use parking_lot::Mutex;

    /// Disk streaming sampler.
    pub struct StreamingSamplerUnit {
        consumer: Arc<Mutex<RegionBufferConsumer>>,
        playing: AtomicBool,

        /// Gain (0.0 - 1.0+)
        gain: f32,

        /// Sample rate
        sample_rate: f32,
    }

    impl Clone for StreamingSamplerUnit {
        fn clone(&self) -> Self {
            Self {
                consumer: Arc::clone(&self.consumer),
                playing: AtomicBool::new(self.playing.load(Ordering::Relaxed)),
                gain: self.gain,
                sample_rate: self.sample_rate,
            }
        }
    }

    impl StreamingSamplerUnit {
        /// Create a new streaming sampler
        ///
        /// # Arguments
        /// * `consumer` - Ring buffer consumer from Butler
        /// * `sample_rate` - Audio sample rate
        pub fn new(consumer: Arc<Mutex<RegionBufferConsumer>>, sample_rate: f32) -> Self {
            Self {
                consumer,
                playing: AtomicBool::new(false),
                gain: 1.0,
                sample_rate,
            }
        }

        /// Start playback
        pub fn play(&self) {
            self.playing.store(true, Ordering::Relaxed);
        }

        /// Stop playback
        pub fn stop(&self) {
            self.playing.store(false, Ordering::Relaxed);
        }

        /// Check if currently playing
        pub fn is_playing(&self) -> bool {
            self.playing.load(Ordering::Relaxed)
        }

        /// Set gain
        pub fn set_gain(&mut self, gain: f32) {
            self.gain = gain;
        }
    }

    impl AudioUnit for StreamingSamplerUnit {
        fn inputs(&self) -> usize {
            0
        }

        fn outputs(&self) -> usize {
            2
        }

        fn reset(&mut self) {
            self.playing.store(false, Ordering::Relaxed);
        }

        fn set_sample_rate(&mut self, sample_rate: f64) {
            self.sample_rate = sample_rate as f32;
        }

        fn tick(&mut self, _input: &[f32], output: &mut [f32]) {
            if !self.playing.load(Ordering::Relaxed) {
                if output.len() >= 2 {
                    output[0] = 0.0;
                    output[1] = 0.0;
                }
                return;
            }

            // Try to lock consumer (should be fast since butler doesn't hold it long)
            if let Some(mut guard) = self.consumer.try_lock() {
                if let Some((left, right)) = guard.read() {
                    if output.len() >= 2 {
                        output[0] = left * self.gain;
                        output[1] = right * self.gain;
                    }
                } else {
                    // Buffer underrun - output silence
                    if output.len() >= 2 {
                        output[0] = 0.0;
                        output[1] = 0.0;
                    }
                }
            } else {
                // Couldn't lock - output silence
                if output.len() >= 2 {
                    output[0] = 0.0;
                    output[1] = 0.0;
                }
            }
        }

        fn process(&mut self, size: usize, _input: &BufferRef, output: &mut BufferMut) {
            if !self.playing.load(Ordering::Relaxed) {
                for i in 0..size {
                    output.set_f32(0, i, 0.0);
                    output.set_f32(1, i, 0.0);
                }
                return;
            }

            // Try to lock consumer
            if let Some(mut guard) = self.consumer.try_lock() {
                for i in 0..size {
                    if let Some((left, right)) = guard.read() {
                        output.set_f32(0, i, left * self.gain);
                        output.set_f32(1, i, right * self.gain);
                    } else {
                        // Buffer underrun
                        output.set_f32(0, i, 0.0);
                        output.set_f32(1, i, 0.0);
                    }
                }
            } else {
                // Couldn't lock - output silence
                for i in 0..size {
                    output.set_f32(0, i, 0.0);
                    output.set_f32(1, i, 0.0);
                }
            }
        }

        fn get_id(&self) -> u64 {
            Arc::as_ptr(&self.consumer) as *const () as usize as u64
        }

        fn route(&mut self, _input: &SignalFrame, _frequency: f64) -> SignalFrame {
            SignalFrame::new(2)
        }

        fn footprint(&self) -> usize {
            std::mem::size_of::<Self>()
        }
    }
}

pub use streaming::StreamingSamplerUnit;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sampler_unit_creation() {
        // Create a simple test wave (mono, 100 samples)
        let wave = Wave::with_capacity(1, 44100.0, 100);
        let sampler = SamplerUnit::new(Arc::new(wave));

        assert!(!sampler.is_playing());
        assert!(!sampler.is_looping());
        assert_eq!(sampler.position(), 0);
    }

    #[test]
    fn test_sampler_trigger() {
        let wave = Wave::with_capacity(1, 44100.0, 100);
        let sampler = SamplerUnit::new(Arc::new(wave));

        sampler.trigger();
        assert!(sampler.is_playing());
        assert_eq!(sampler.position(), 0);

        sampler.stop();
        assert!(!sampler.is_playing());
    }

    #[test]
    fn test_sampler_outputs_silence_when_stopped() {
        let wave = Wave::with_capacity(1, 44100.0, 100);
        let mut sampler = SamplerUnit::new(Arc::new(wave));

        let mut output = [0.0f32; 2];
        sampler.tick(&[], &mut output);

        assert_eq!(output[0], 0.0);
        assert_eq!(output[1], 0.0);
    }
}
