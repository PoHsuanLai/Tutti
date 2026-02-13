//! Fluent API handle for metering control.

use super::{MeteringManager, StereoAnalysisSnapshot};
use crate::compat::Arc;

/// Fluent API handle for metering control.
///
/// Created via `engine.metering()`.
///
/// # Example
/// ```ignore
/// // Enable multiple meters with chaining
/// engine.metering()
///     .amp()
///     .lufs()
///     .correlation();
///
/// // Read values
/// let m = engine.metering();
/// let (l_peak, r_peak, l_rms, r_rms) = m.amplitude();
/// let lufs = m.loudness_global().unwrap_or(-70.0);
/// ```
pub struct MeteringHandle {
    manager: Arc<MeteringManager>,
}

impl MeteringHandle {
    pub fn new(manager: Arc<MeteringManager>) -> Self {
        Self { manager }
    }

    pub fn amp(self) -> Self {
        self.manager.enable_amp();
        self
    }

    pub fn amp_off(self) -> Self {
        self.manager.disable_amp();
        self
    }

    pub fn is_amp(&self) -> bool {
        self.manager.amp_enabled()
    }

    /// Returns (left_peak, right_peak, left_rms, right_rms).
    pub fn amplitude(&self) -> (f32, f32, f32, f32) {
        self.manager.amplitude()
    }

    pub fn lufs(self) -> Self {
        self.manager.enable_lufs();
        self
    }

    pub fn lufs_off(self) -> Self {
        self.manager.disable_lufs();
        self
    }

    pub fn is_lufs(&self) -> bool {
        self.manager.is_lufs_enabled()
    }

    pub fn loudness_global(&self) -> crate::Result<f64> {
        self.manager.loudness_global()
    }

    /// 3-second window, LUFS.
    pub fn loudness_shortterm(&self) -> crate::Result<f64> {
        self.manager.loudness_shortterm()
    }

    /// Loudness range in LU.
    pub fn loudness_range(&self) -> crate::Result<f64> {
        self.manager.loudness_range()
    }

    /// Channel: 0=left, 1=right. Returns dBTP.
    pub fn true_peak(&self, channel: u32) -> crate::Result<f64> {
        self.manager.true_peak(channel)
    }

    pub fn reset_lufs(self) -> Self {
        self.manager.reset_lufs();
        self
    }

    pub fn correlation(self) -> Self {
        self.manager.enable_correlation();
        self
    }

    pub fn correlation_off(self) -> Self {
        self.manager.disable_correlation();
        self
    }

    pub fn is_correlation(&self) -> bool {
        self.manager.correlation_enabled()
    }

    pub fn stereo_analysis(&self) -> StereoAnalysisSnapshot {
        self.manager.stereo_analysis()
    }

    pub fn cpu(self) -> Self {
        self.manager.cpu().enable();
        self
    }

    pub fn cpu_off(self) -> Self {
        self.manager.cpu().disable();
        self
    }

    pub fn is_cpu(&self) -> bool {
        self.manager.cpu().is_enabled()
    }

    pub fn cpu_average(&self) -> f32 {
        self.manager.cpu().average_percent()
    }

    pub fn cpu_peak(&self) -> f32 {
        self.manager.cpu().peak_percent()
    }

    pub fn cpu_current(&self) -> f32 {
        self.manager.cpu().current_percent()
    }

    pub fn cpu_underruns(&self) -> u64 {
        self.manager.cpu().underruns()
    }

    pub fn cpu_reset(self) -> Self {
        self.manager.cpu().reset();
        self
    }

    pub fn inner(&self) -> &Arc<MeteringManager> {
        &self.manager
    }
}
