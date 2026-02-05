//! Real-time metering updates (called from audio callback).

use super::MeteringManager;
use crate::compat::Vec;
use core::time::Duration;

/// Real-time metering context passed to the audio callback.
///
/// Holds pre-allocated buffers to avoid allocations in the RT path.
/// Buffers are pre-allocated with capacity for MAX_FRAMES to ensure
/// resize() within capacity doesn't allocate.
pub struct MeteringContext {
    left_buf: Vec<f32>,
    right_buf: Vec<f32>,
}

/// Maximum expected frame count per buffer (covers all common audio interfaces)
const MAX_FRAMES: usize = 8192;

impl MeteringContext {
    /// Create a new metering context with pre-allocated buffers.
    ///
    /// RT-safe: Buffers are pre-allocated to avoid allocation in audio callback.
    pub fn new() -> Self {
        Self {
            left_buf: Vec::with_capacity(MAX_FRAMES),
            right_buf: Vec::with_capacity(MAX_FRAMES),
        }
    }

    /// Ensure buffers are large enough for the given frame count.
    ///
    /// RT-safe: resize within pre-allocated capacity doesn't allocate.
    #[inline]
    fn ensure_capacity(&mut self, frames: usize) {
        if self.left_buf.len() < frames {
            // RT-safe: resize within capacity is just length adjustment + fill
            self.left_buf.resize(frames, 0.0);
            self.right_buf.resize(frames, 0.0);
        }
    }

    /// Deinterleave stereo output into left/right buffers.
    #[inline]
    fn deinterleave(&mut self, output: &[f32], frames: usize) {
        for i in 0..frames {
            self.left_buf[i] = output[i * 2];
            self.right_buf[i] = output[i * 2 + 1];
        }
    }
}

impl Default for MeteringContext {
    fn default() -> Self {
        Self::new()
    }
}

impl MeteringManager {
    /// Update all enabled meters from the audio output buffer.
    ///
    /// Call this from the audio callback after DSP processing.
    /// The `elapsed` duration is used for CPU metering.
    ///
    /// # Arguments
    /// * `output` - Interleaved stereo f32 samples
    /// * `frames` - Number of stereo frames
    /// * `elapsed` - Time spent processing this buffer (for CPU metering)
    /// * `ctx` - Pre-allocated buffers for RT-safe operation
    #[inline]
    pub fn update_rt(
        &self,
        output: &[f32],
        frames: usize,
        elapsed: Duration,
        ctx: &mut MeteringContext,
    ) {
        // CPU metering (must be first - uses elapsed time)
        self.update_cpu(frames, elapsed);

        // Amplitude metering (peak/RMS)
        self.update_amplitude(output, frames);

        // Stereo correlation (needs deinterleaved buffers)
        if self.correlation_enabled() {
            ctx.ensure_capacity(frames);
            ctx.deinterleave(output, frames);
            self.update_stereo(&ctx.left_buf[..frames], &ctx.right_buf[..frames]);
        }

        // LUFS loudness (needs deinterleaved buffers)
        if self.is_lufs_enabled() {
            ctx.ensure_capacity(frames);
            ctx.deinterleave(output, frames);
            self.update_lufs(&ctx.left_buf[..frames], &ctx.right_buf[..frames]);
        }

        // Analysis tap (for external analysis threads)
        self.push_analysis_tap(output, frames);
    }

    /// Update amplitude metering (peak and RMS).
    #[inline]
    fn update_amplitude(&self, output: &[f32], frames: usize) {
        if !self.amp_enabled() {
            return;
        }

        let mut peak_l: f32 = 0.0;
        let mut peak_r: f32 = 0.0;
        let mut sum_sq_l: f32 = 0.0;
        let mut sum_sq_r: f32 = 0.0;

        for i in 0..frames {
            let l = output[i * 2];
            let r = output[i * 2 + 1];
            peak_l = peak_l.max(l.abs());
            peak_r = peak_r.max(r.abs());
            sum_sq_l += l * l;
            sum_sq_r += r * r;
        }

        let rms_l = (sum_sq_l / frames as f32).sqrt();
        let rms_r = (sum_sq_r / frames as f32).sqrt();

        self.amplitude_atomic().set(peak_l, peak_r, rms_l, rms_r);
    }

    /// Update stereo correlation metering.
    #[inline]
    fn update_stereo(&self, left: &[f32], right: &[f32]) {
        self.stereo_atomic().update_from_buffers(left, right);
    }

    /// Update LUFS loudness metering (non-blocking).
    #[inline]
    fn update_lufs(&self, left: &[f32], right: &[f32]) {
        if let Some(mut ebur128) = self.ebur128().try_lock() {
            let _ = ebur128.add_frames_planar_f32(&[left, right]);
        }
    }

    /// Update CPU metering.
    #[inline]
    fn update_cpu(&self, frames: usize, elapsed: Duration) {
        self.cpu().record(frames, elapsed);
    }
}
