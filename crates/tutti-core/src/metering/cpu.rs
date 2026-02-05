//! CPU usage tracking for audio callbacks.

use crate::{AtomicBool, AtomicFloat, AtomicU32, AtomicU64, Ordering};
use core::time::Duration;

/// CPU metrics snapshot.
#[derive(Debug, Clone, Default)]
pub struct CpuMetrics {
    pub average: f32,
    pub peak: f32,
    pub current: f32,
    pub underruns: u64,
    pub buffer_size: usize,
    pub max_time_us: f64,
    pub actual_time_us: f64,
}

/// CPU meter for audio callback performance tracking.
pub struct CpuMeter {
    current: AtomicFloat,
    peak: AtomicFloat,
    average: AtomicFloat,
    underruns: AtomicU64,
    samples: AtomicU32,
    sample_rate: f64,
    enabled: AtomicBool,
}

impl CpuMeter {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            current: AtomicFloat::new(0.0),
            peak: AtomicFloat::new(0.0),
            average: AtomicFloat::new(0.0),
            underruns: AtomicU64::new(0),
            samples: AtomicU32::new(0),
            sample_rate,
            enabled: AtomicBool::new(false),
        }
    }

    pub fn enable(&self) {
        self.enabled.store(true, Ordering::Release);
    }

    pub fn disable(&self) {
        self.enabled.store(false, Ordering::Release);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    pub fn record(&self, buffer_size: usize, elapsed: Duration) {
        if !self.is_enabled() {
            return;
        }

        let max_time = buffer_size as f64 / self.sample_rate;
        let load = (elapsed.as_secs_f64() / max_time) as f32;

        self.current.set(load);

        if load > self.peak.get() {
            self.peak.set(load);
        }

        // Exponential moving average
        let count = self.samples.fetch_add(1, Ordering::Relaxed);
        let alpha = 1.0 / (count.min(100) + 1) as f32;
        let avg = self.average.get();
        self.average.set(avg * (1.0 - alpha) + load * alpha);

        if elapsed.as_secs_f64() > max_time {
            self.underruns.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn metrics(&self, buffer_size: usize) -> CpuMetrics {
        let max_time_us = (buffer_size as f64 / self.sample_rate) * 1_000_000.0;
        let actual_time_us = (self.current.get() as f64) * max_time_us;

        CpuMetrics {
            average: self.average.get() * 100.0,
            peak: self.peak.get() * 100.0,
            current: self.current.get() * 100.0,
            underruns: self.underruns.load(Ordering::Relaxed),
            buffer_size,
            max_time_us,
            actual_time_us,
        }
    }

    pub fn average_percent(&self) -> f32 {
        self.average.get() * 100.0
    }

    pub fn peak_percent(&self) -> f32 {
        self.peak.get() * 100.0
    }

    pub fn current_percent(&self) -> f32 {
        self.current.get() * 100.0
    }

    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }

    pub fn reset(&self) {
        self.current.set(0.0);
        self.peak.set(0.0);
        self.average.set(0.0);
        self.underruns.store(0, Ordering::Relaxed);
        self.samples.store(0, Ordering::Relaxed);
    }
}
