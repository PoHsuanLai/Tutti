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
    /// Create a new metering handle wrapping the given manager.
    pub fn new(manager: Arc<MeteringManager>) -> Self {
        Self { manager }
    }

    /// Enable amplitude (peak/RMS) metering.
    pub fn amp(self) -> Self {
        self.manager.enable_amp();
        self
    }

    /// Disable amplitude metering.
    pub fn amp_off(self) -> Self {
        self.manager.disable_amp();
        self
    }

    /// Check if amplitude metering is enabled.
    pub fn is_amp(&self) -> bool {
        self.manager.amp_enabled()
    }

    /// Get current amplitude levels: (left_peak, right_peak, left_rms, right_rms).
    pub fn amplitude(&self) -> (f32, f32, f32, f32) {
        self.manager.amplitude()
    }

    /// Enable LUFS loudness metering.
    pub fn lufs(self) -> Self {
        self.manager.enable_lufs();
        self
    }

    /// Disable LUFS loudness metering.
    pub fn lufs_off(self) -> Self {
        self.manager.disable_lufs();
        self
    }

    /// Check if LUFS metering is enabled.
    pub fn is_lufs(&self) -> bool {
        self.manager.is_lufs_enabled()
    }

    /// Get integrated loudness (LUFS) over entire measurement period.
    pub fn loudness_global(&self) -> crate::Result<f64> {
        self.manager.loudness_global()
    }

    /// Get short-term loudness (3-second window, LUFS).
    pub fn loudness_shortterm(&self) -> crate::Result<f64> {
        self.manager.loudness_shortterm()
    }

    /// Get loudness range (LRA) in LU.
    pub fn loudness_range(&self) -> crate::Result<f64> {
        self.manager.loudness_range()
    }

    /// Get true peak level for a channel (0=left, 1=right) in dBTP.
    pub fn true_peak(&self, channel: u32) -> crate::Result<f64> {
        self.manager.true_peak(channel)
    }

    /// Reset LUFS measurement history.
    pub fn reset_lufs(self) -> Self {
        self.manager.reset_lufs();
        self
    }

    /// Enable stereo correlation analysis.
    pub fn correlation(self) -> Self {
        self.manager.enable_correlation();
        self
    }

    /// Disable stereo correlation analysis.
    pub fn correlation_off(self) -> Self {
        self.manager.disable_correlation();
        self
    }

    /// Check if stereo correlation analysis is enabled.
    pub fn is_correlation(&self) -> bool {
        self.manager.correlation_enabled()
    }

    /// Get current stereo analysis (correlation, balance, width).
    pub fn stereo_analysis(&self) -> StereoAnalysisSnapshot {
        self.manager.stereo_analysis()
    }

    /// Enable CPU load metering.
    pub fn cpu(self) -> Self {
        self.manager.cpu().enable();
        self
    }

    /// Disable CPU load metering.
    pub fn cpu_off(self) -> Self {
        self.manager.cpu().disable();
        self
    }

    /// Check if CPU metering is enabled.
    pub fn is_cpu(&self) -> bool {
        self.manager.cpu().is_enabled()
    }

    /// Get average CPU load as a percentage (0-100).
    pub fn cpu_average(&self) -> f32 {
        self.manager.cpu().average_percent()
    }

    /// Get peak CPU load as a percentage (0-100).
    pub fn cpu_peak(&self) -> f32 {
        self.manager.cpu().peak_percent()
    }

    /// Get current CPU load as a percentage (0-100).
    pub fn cpu_current(&self) -> f32 {
        self.manager.cpu().current_percent()
    }

    /// Get number of audio underruns.
    pub fn cpu_underruns(&self) -> u64 {
        self.manager.cpu().underruns()
    }

    /// Reset CPU metrics (peak, average, underrun count).
    pub fn cpu_reset(self) -> Self {
        self.manager.cpu().reset();
        self
    }

    /// Get direct access to the underlying MeteringManager.
    ///
    /// Use this for advanced features like analysis taps or atomic access.
    pub fn inner(&self) -> &Arc<MeteringManager> {
        &self.manager
    }
}
