//! Central metering manager.

use super::{AtomicAmplitude, AtomicStereoAnalysis, CpuMeter, StereoAnalysisSnapshot};
use crate::compat::{Arc, HashMap, Mutex};
use crossbeam_channel::Receiver;
use ebur128::{EbuR128, Mode};

/// Stereo sample pair (left, right).
type StereoSample = (f32, f32);

/// Receiver for master channel metering data.
type MasterMeterRx = Arc<Mutex<Option<Receiver<StereoSample>>>>;

/// Receivers for per-channel metering data, keyed by channel index.
type ChannelMeterRxs = Arc<Mutex<HashMap<usize, Receiver<StereoSample>>>>;

/// Central metering manager for amplitude, stereo analysis, CPU, and LUFS.
pub struct MeteringManager {
    amplitude: Arc<AtomicAmplitude>,
    stereo: Arc<AtomicStereoAnalysis>,
    cpu: Arc<CpuMeter>,
    ebur128: Arc<Mutex<EbuR128>>,

    amp_enabled: Arc<crate::lockfree::AtomicFlag>,
    correlation_enabled: Arc<crate::lockfree::AtomicFlag>,
    lufs_enabled: Arc<crate::lockfree::AtomicFlag>,

    master_rx: MasterMeterRx,
    channel_rxs: ChannelMeterRxs,

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
            amplitude: Arc::new(AtomicAmplitude::new()),
            stereo: Arc::new(AtomicStereoAnalysis::new()),
            cpu: Arc::new(CpuMeter::new(sample_rate)),
            ebur128: Arc::new(Mutex::new(ebur128)),

            amp_enabled: Arc::new(crate::lockfree::AtomicFlag::new(false)),
            correlation_enabled: Arc::new(crate::lockfree::AtomicFlag::new(false)),
            lufs_enabled: Arc::new(crate::lockfree::AtomicFlag::new(false)),

            master_rx: Arc::new(Mutex::new(None)),
            channel_rxs: Arc::new(Mutex::new(HashMap::new())),

            sample_rate,
        }
    }

    /// Returns the sample rate used for metering calculations.
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// Enables amplitude metering (peak levels).
    pub fn enable_amp(&self) {
        self.amp_enabled.set(true);
    }

    /// Disables amplitude metering.
    pub fn disable_amp(&self) {
        self.amp_enabled.set(false);
    }

    /// Returns whether amplitude metering is enabled.
    pub fn amp_enabled(&self) -> bool {
        self.amp_enabled.get()
    }

    /// Returns current amplitude levels: (left_peak, right_peak, left_rms, right_rms).
    pub fn amplitude(&self) -> (f32, f32, f32, f32) {
        self.amplitude.get()
    }

    /// Returns the atomic amplitude meter for direct access from audio thread.
    pub fn amplitude_atomic(&self) -> &Arc<AtomicAmplitude> {
        &self.amplitude
    }

    /// Enables stereo correlation analysis.
    pub fn enable_correlation(&self) {
        self.correlation_enabled.set(true);
    }

    /// Disables stereo correlation analysis.
    pub fn disable_correlation(&self) {
        self.correlation_enabled.set(false);
    }

    /// Returns whether stereo correlation analysis is enabled.
    pub fn correlation_enabled(&self) -> bool {
        self.correlation_enabled.get()
    }

    /// Returns current stereo analysis (correlation, balance, width).
    pub fn stereo_analysis(&self) -> StereoAnalysisSnapshot {
        self.stereo.get()
    }

    /// Returns the atomic stereo analyzer for direct access from audio thread.
    pub fn stereo_atomic(&self) -> &Arc<AtomicStereoAnalysis> {
        &self.stereo
    }

    /// Returns the CPU meter for tracking audio callback load.
    pub fn cpu(&self) -> &Arc<CpuMeter> {
        &self.cpu
    }

    /// Enables LUFS loudness metering.
    pub fn enable_lufs(&self) {
        self.lufs_enabled.set(true);
    }

    /// Disables LUFS loudness metering.
    pub fn disable_lufs(&self) {
        self.lufs_enabled.set(false);
    }

    /// Returns whether LUFS metering is enabled.
    pub fn is_lufs_enabled(&self) -> bool {
        self.lufs_enabled.get()
    }

    /// Returns integrated loudness (LUFS) over the entire measurement period.
    pub fn loudness_global(&self) -> crate::Result<f64> {
        self.ebur128
            .lock()
            .loudness_global()
            .map_err(|_| crate::Error::LufsNotReady)
    }

    /// Returns short-term loudness (3-second window, LUFS).
    pub fn loudness_shortterm(&self) -> crate::Result<f64> {
        self.ebur128
            .lock()
            .loudness_shortterm()
            .map_err(|_| crate::Error::LufsNotReady)
    }

    /// Returns loudness range (LRA) in LU.
    pub fn loudness_range(&self) -> crate::Result<f64> {
        self.ebur128
            .lock()
            .loudness_range()
            .map_err(|_| crate::Error::LufsNotReady)
    }

    /// Returns true peak level for a channel (0=left, 1=right) in dBTP.
    pub fn true_peak(&self, channel: u32) -> crate::Result<f64> {
        self.ebur128
            .lock()
            .true_peak(channel)
            .map_err(|_| crate::Error::LufsNotReady)
    }

    /// Resets LUFS measurement history.
    pub fn reset_lufs(&self) {
        self.ebur128.lock().reset();
    }

    /// Returns direct access to the EBU R128 meter.
    pub fn ebur128(&self) -> &Arc<Mutex<EbuR128>> {
        &self.ebur128
    }

    /// Sets the receiver for master channel metering data.
    pub fn set_master_consumer(&self, rx: Receiver<StereoSample>) {
        *self.master_rx.lock() = Some(rx);
    }

    /// Takes the master consumer receiver (returns receiver and sample rate).
    pub fn take_master_consumer(&self) -> Option<(Receiver<StereoSample>, f64)> {
        self.master_rx.lock().take().map(|r| (r, self.sample_rate))
    }

    /// Takes all channel consumer receivers.
    pub fn take_channel_consumers(&self) -> HashMap<usize, Receiver<StereoSample>> {
        core::mem::take(&mut *self.channel_rxs.lock())
    }

    /// Creates a new channel buffer and returns the sender for the audio thread.
    pub fn create_channel_buffer(&self, channel: usize) -> crossbeam_channel::Sender<StereoSample> {
        let (tx, rx) = crossbeam_channel::bounded(8192);
        self.channel_rxs.lock().insert(channel, rx);
        tx
    }

    /// Removes a channel buffer.
    pub fn remove_channel_buffer(&self, channel: usize) {
        self.channel_rxs.lock().remove(&channel);
    }
}
