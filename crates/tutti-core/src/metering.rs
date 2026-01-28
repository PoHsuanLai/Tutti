//! Real-time audio metering and CPU tracking.

use crate::{AtomicBool, AtomicFloat, AtomicU32, AtomicU64, Ordering};
use crossbeam_channel::Receiver;
use ebur128::{EbuR128, Mode};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type AnalysisReceiver = Receiver<(f32, f32)>;
type ChannelAnalysisReceivers = HashMap<usize, Receiver<(f32, f32)>>;
type AnalysisConsumer = (Receiver<(f32, f32)>, f64);

/// Lock-free amplitude storage (RMS L/R, Peak L/R).
pub struct AtomicAmplitude {
    rms_left: AtomicFloat,
    rms_right: AtomicFloat,
    peak_left: AtomicFloat,
    peak_right: AtomicFloat,
}

impl Default for AtomicAmplitude {
    fn default() -> Self {
        Self::new()
    }
}

impl AtomicAmplitude {
    pub fn new() -> Self {
        Self {
            rms_left: AtomicFloat::new(0.0),
            rms_right: AtomicFloat::new(0.0),
            peak_left: AtomicFloat::new(0.0),
            peak_right: AtomicFloat::new(0.0),
        }
    }

    #[inline]
    pub fn get(&self) -> (f32, f32, f32, f32) {
        (
            self.rms_left.get(),
            self.rms_right.get(),
            self.peak_left.get(),
            self.peak_right.get(),
        )
    }

    #[inline]
    pub fn set(&self, rms_l: f32, rms_r: f32, peak_l: f32, peak_r: f32) {
        self.rms_left.set(rms_l);
        self.rms_right.set(rms_r);
        self.peak_left.set(peak_l);
        self.peak_right.set(peak_r);
    }
}

/// Lock-free stereo analysis storage.
pub struct AtomicStereoAnalysis {
    correlation: AtomicFloat,
    width: AtomicFloat,
    balance: AtomicFloat,
    mid_level: AtomicFloat,
    side_level: AtomicFloat,
}

impl Default for AtomicStereoAnalysis {
    fn default() -> Self {
        Self::new()
    }
}

impl AtomicStereoAnalysis {
    pub fn new() -> Self {
        Self {
            correlation: AtomicFloat::new(0.0),
            width: AtomicFloat::new(1.0),
            balance: AtomicFloat::new(0.0),
            mid_level: AtomicFloat::new(0.0),
            side_level: AtomicFloat::new(0.0),
        }
    }

    #[inline]
    pub fn get(&self) -> StereoAnalysisSnapshot {
        StereoAnalysisSnapshot {
            correlation: self.correlation.get(),
            width: self.width.get(),
            balance: self.balance.get(),
            mid_level: self.mid_level.get(),
            side_level: self.side_level.get(),
        }
    }

    #[inline]
    pub fn set(&self, correlation: f32, width: f32, balance: f32, mid_level: f32, side_level: f32) {
        self.correlation.set(correlation);
        self.width.set(width);
        self.balance.set(balance);
        self.mid_level.set(mid_level);
        self.side_level.set(side_level);
    }

    pub fn update_from_buffers(&self, left: &[f32], right: &[f32]) {
        let len = left.len().min(right.len());
        if len == 0 {
            return;
        }

        let mut sum_l_sq = 0.0f64;
        let mut sum_r_sq = 0.0f64;
        let mut sum_lr = 0.0f64;
        let mut sum_mid_sq = 0.0f64;
        let mut sum_side_sq = 0.0f64;

        for i in 0..len {
            let l = left[i] as f64;
            let r = right[i] as f64;
            sum_l_sq += l * l;
            sum_r_sq += r * r;
            sum_lr += l * r;
            let mid = (l + r) * 0.5;
            let side = (l - r) * 0.5;
            sum_mid_sq += mid * mid;
            sum_side_sq += side * side;
        }

        let n = len as f64;
        let left_rms = (sum_l_sq / n).sqrt() as f32;
        let right_rms = (sum_r_sq / n).sqrt() as f32;
        let mid_rms = (sum_mid_sq / n).sqrt() as f32;
        let side_rms = (sum_side_sq / n).sqrt() as f32;

        let correlation = if sum_l_sq > 0.0 && sum_r_sq > 0.0 {
            (sum_lr / (sum_l_sq.sqrt() * sum_r_sq.sqrt())) as f32
        } else {
            0.0
        };

        let width = 1.0 - correlation;
        let total_level = left_rms + right_rms;
        let balance = if total_level > 0.0 {
            (right_rms - left_rms) / total_level
        } else {
            0.0
        };

        self.set(correlation, width, balance, mid_rms, side_rms);
    }
}

/// Stereo analysis snapshot.
#[derive(Debug, Clone, Copy, Default)]
pub struct StereoAnalysisSnapshot {
    pub correlation: f32,
    pub width: f32,
    pub balance: f32,
    pub mid_level: f32,
    pub side_level: f32,
}

/// CPU metrics.
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
    current_load: AtomicFloat,
    peak_load: AtomicFloat,
    average_load: AtomicFloat,
    underrun_count: AtomicU64,
    sample_rate: f64,
    enabled: AtomicBool,
    samples_count: AtomicU32,
}

impl CpuMeter {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            current_load: AtomicFloat::new(0.0),
            peak_load: AtomicFloat::new(0.0),
            average_load: AtomicFloat::new(0.0),
            underrun_count: AtomicU64::new(0),
            sample_rate,
            enabled: AtomicBool::new(false),
            samples_count: AtomicU32::new(0),
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

        self.current_load.set(load);

        if load > self.peak_load.get() {
            self.peak_load.set(load);
        }

        let count = self.samples_count.fetch_add(1, Ordering::Relaxed);
        let alpha = 1.0 / (count.min(100) + 1) as f32;
        let avg = self.average_load.get();
        self.average_load.set(avg * (1.0 - alpha) + load * alpha);

        if elapsed.as_secs_f64() > max_time {
            self.underrun_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn get_metrics(&self, buffer_size: usize) -> CpuMetrics {
        let max_time_us = (buffer_size as f64 / self.sample_rate) * 1_000_000.0;
        let actual_time_us = (self.current_load.get() as f64) * max_time_us;

        CpuMetrics {
            average: self.average_load.get() * 100.0,
            peak: self.peak_load.get() * 100.0,
            current: self.current_load.get() * 100.0,
            underruns: self.underrun_count.load(Ordering::Relaxed),
            buffer_size,
            max_time_us,
            actual_time_us,
        }
    }

    pub fn get_average_usage(&self) -> f32 {
        self.average_load.get() * 100.0
    }

    pub fn reset(&self) {
        self.current_load.set(0.0);
        self.peak_load.set(0.0);
        self.average_load.set(0.0);
        self.underrun_count.store(0, Ordering::Relaxed);
        self.samples_count.store(0, Ordering::Relaxed);
    }
}

/// Metering manager.
pub struct MeteringManager {
    amp_monitor_enabled: Arc<crate::lockfree::AtomicFlag>,
    current_amplitude: Arc<AtomicAmplitude>,
    stereo_analysis: Arc<AtomicStereoAnalysis>,
    correlation_enabled: Arc<crate::lockfree::AtomicFlag>,
    analysis_buffer_rx: Arc<Mutex<Option<AnalysisReceiver>>>,
    channel_analysis_rxs: Arc<Mutex<ChannelAnalysisReceivers>>,
    cpu_meter: Arc<CpuMeter>,
    ebur128: Arc<Mutex<EbuR128>>,
    lufs_enabled: Arc<crate::lockfree::AtomicFlag>,
    sample_rate: f64,
}

impl MeteringManager {
    pub(crate) fn new(sample_rate: f64) -> Self {
        let ebur128 = EbuR128::new(
            2,
            sample_rate as u32,
            Mode::I | Mode::S | Mode::LRA | Mode::TRUE_PEAK,
        )
        .expect("Failed to create EBU R128 meter");

        Self {
            amp_monitor_enabled: Arc::new(crate::lockfree::AtomicFlag::new(false)),
            current_amplitude: Arc::new(AtomicAmplitude::new()),
            stereo_analysis: Arc::new(AtomicStereoAnalysis::new()),
            correlation_enabled: Arc::new(crate::lockfree::AtomicFlag::new(false)),
            analysis_buffer_rx: Arc::new(Mutex::new(None)),
            channel_analysis_rxs: Arc::new(Mutex::new(HashMap::new())),
            cpu_meter: Arc::new(CpuMeter::new(sample_rate)),
            ebur128: Arc::new(Mutex::new(ebur128)),
            lufs_enabled: Arc::new(crate::lockfree::AtomicFlag::new(false)),
            sample_rate,
        }
    }

    pub fn set_master_consumer(&self, receiver: Receiver<(f32, f32)>) -> crate::Result<()> {
        *self
            .analysis_buffer_rx
            .lock()
            .map_err(|_| crate::Error::LockPoisoned)? = Some(receiver);
        Ok(())
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub fn start_amp_monitor(&self) {
        self.amp_monitor_enabled.set(true);
    }

    pub fn stop_amp_monitor(&self) {
        self.amp_monitor_enabled.set(false);
    }

    pub fn is_amp_monitor_enabled(&self) -> bool {
        self.amp_monitor_enabled.get()
    }

    pub fn get_amplitude(&self) -> (f32, f32, f32, f32) {
        self.current_amplitude.get()
    }

    pub fn start_correlation_monitor(&self) {
        self.correlation_enabled.set(true);
    }

    pub fn stop_correlation_monitor(&self) {
        self.correlation_enabled.set(false);
    }

    pub fn is_correlation_monitor_enabled(&self) -> bool {
        self.correlation_enabled.get()
    }

    pub fn get_stereo_analysis(&self) -> StereoAnalysisSnapshot {
        self.stereo_analysis.get()
    }

    pub fn stereo_analysis(&self) -> &Arc<AtomicStereoAnalysis> {
        &self.stereo_analysis
    }

    pub fn correlation_enabled(&self) -> &Arc<crate::lockfree::AtomicFlag> {
        &self.correlation_enabled
    }

    pub fn take_analysis_consumer(&self) -> crate::Result<Option<AnalysisConsumer>> {
        let receiver = self
            .analysis_buffer_rx
            .lock()
            .map_err(|_| crate::Error::LockPoisoned)?
            .take();
        Ok(receiver.map(|r| (r, self.sample_rate)))
    }

    pub fn take_channel_analysis_consumers(
        &self,
    ) -> crate::Result<HashMap<usize, Receiver<(f32, f32)>>> {
        Ok(std::mem::take(
            &mut *self
                .channel_analysis_rxs
                .lock()
                .map_err(|_| crate::Error::LockPoisoned)?,
        ))
    }

    pub fn create_channel_analysis_buffer(
        &self,
        channel_index: usize,
    ) -> crate::Result<crossbeam_channel::Sender<(f32, f32)>> {
        let (tx, rx) = crossbeam_channel::bounded::<(f32, f32)>(8192);
        self.channel_analysis_rxs
            .lock()
            .map_err(|_| crate::Error::LockPoisoned)?
            .insert(channel_index, rx);
        Ok(tx)
    }

    pub fn remove_channel_analysis_buffer(&self, channel_index: usize) -> crate::Result<()> {
        self.channel_analysis_rxs
            .lock()
            .map_err(|_| crate::Error::LockPoisoned)?
            .remove(&channel_index);
        Ok(())
    }

    pub fn cpu_meter(&self) -> Arc<CpuMeter> {
        Arc::clone(&self.cpu_meter)
    }

    pub fn amp_monitor_enabled(&self) -> &Arc<crate::lockfree::AtomicFlag> {
        &self.amp_monitor_enabled
    }

    pub fn current_amplitude(&self) -> &Arc<AtomicAmplitude> {
        &self.current_amplitude
    }

    pub fn enable_cpu_monitoring(&self) {
        self.cpu_meter.enable();
    }

    pub fn disable_cpu_monitoring(&self) {
        self.cpu_meter.disable();
    }

    pub fn is_cpu_monitoring_enabled(&self) -> bool {
        self.cpu_meter.is_enabled()
    }

    pub fn get_cpu_metrics(&self, buffer_size: usize) -> CpuMetrics {
        self.cpu_meter.get_metrics(buffer_size)
    }

    pub fn get_cpu_usage(&self) -> f32 {
        self.cpu_meter.get_average_usage()
    }

    pub fn reset_cpu_metrics(&self) {
        self.cpu_meter.reset();
    }

    pub fn enable_lufs(&self) {
        self.lufs_enabled.set(true);
    }

    pub fn disable_lufs(&self) {
        self.lufs_enabled.set(false);
    }

    pub fn is_lufs_enabled(&self) -> bool {
        self.lufs_enabled.get()
    }

    pub fn loudness_global(&self) -> crate::Result<f64> {
        self.ebur128
            .lock()
            .map_err(|_| crate::Error::LockPoisoned)?
            .loudness_global()
            .map_err(|_| crate::Error::NotImplemented("LUFS measurement not ready".to_string()))
    }

    pub fn loudness_shortterm(&self) -> crate::Result<f64> {
        self.ebur128
            .lock()
            .map_err(|_| crate::Error::LockPoisoned)?
            .loudness_shortterm()
            .map_err(|_| crate::Error::NotImplemented("LUFS measurement not ready".to_string()))
    }

    pub fn loudness_range(&self) -> crate::Result<f64> {
        self.ebur128
            .lock()
            .map_err(|_| crate::Error::LockPoisoned)?
            .loudness_range()
            .map_err(|_| crate::Error::NotImplemented("LUFS measurement not ready".to_string()))
    }

    pub fn true_peak(&self, channel: u32) -> crate::Result<f64> {
        self.ebur128
            .lock()
            .map_err(|_| crate::Error::LockPoisoned)?
            .true_peak(channel)
            .map_err(|_| crate::Error::NotImplemented("LUFS measurement not ready".to_string()))
    }

    pub fn reset_lufs(&self) -> crate::Result<()> {
        self.ebur128
            .lock()
            .map_err(|_| crate::Error::LockPoisoned)?
            .reset();
        Ok(())
    }

    pub fn lufs_enabled(&self) -> &Arc<crate::lockfree::AtomicFlag> {
        &self.lufs_enabled
    }

    pub fn ebur128(&self) -> &Arc<Mutex<EbuR128>> {
        &self.ebur128
    }
}

impl Default for MeteringManager {
    fn default() -> Self {
        Self::new(44100.0)
    }
}
