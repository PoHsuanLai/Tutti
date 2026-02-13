//! Real-time metering updates (called from audio callback).

use super::MeteringManager;
use crate::compat::Vec;
use core::time::Duration;

/// Pre-allocated buffers for RT-safe metering (deinterleave scratch space).
pub(crate) struct MeteringContext {
    left_buf: Vec<f32>,
    right_buf: Vec<f32>,
}

/// Maximum expected frame count per buffer (covers all common audio interfaces)
const MAX_FRAMES: usize = 8192;

impl MeteringContext {
    pub fn new() -> Self {
        Self {
            left_buf: Vec::with_capacity(MAX_FRAMES),
            right_buf: Vec::with_capacity(MAX_FRAMES),
        }
    }

    /// Clamped to MAX_FRAMES so resize stays within pre-allocated capacity.
    #[inline]
    fn ensure_capacity(&mut self, frames: usize) {
        let frames = frames.min(MAX_FRAMES);
        if self.left_buf.len() < frames {
            self.left_buf.resize(frames, 0.0);
            self.right_buf.resize(frames, 0.0);
        }
    }

    /// Deinterleave stereo output into left/right buffers.
    #[inline]
    fn deinterleave(&mut self, output: &[f32], frames: usize) {
        output
            .chunks_exact(2)
            .take(frames)
            .zip(self.left_buf.iter_mut().zip(self.right_buf.iter_mut()))
            .for_each(|(ch, (l, r))| {
                *l = ch[0];
                *r = ch[1];
            });
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
    /// Called from audio callback after DSP processing.
    /// `output` is interleaved stereo f32, `frames` is the number of stereo frames.
    #[inline]
    pub(crate) fn update_rt(
        &self,
        output: &[f32],
        frames: usize,
        elapsed: Duration,
        ctx: &mut MeteringContext,
    ) {
        debug_assert!(
            frames <= MAX_FRAMES,
            "Audio buffer frames ({frames}) exceeds MAX_FRAMES ({MAX_FRAMES})"
        );

        self.update_cpu(frames, elapsed);
        self.update_amplitude(output, frames);

        if self.correlation_enabled() {
            ctx.ensure_capacity(frames);
            ctx.deinterleave(output, frames);
            self.update_stereo(&ctx.left_buf[..frames], &ctx.right_buf[..frames]);
        }

        if self.is_lufs_enabled() {
            ctx.ensure_capacity(frames);
            ctx.deinterleave(output, frames);
            self.update_lufs(&ctx.left_buf[..frames], &ctx.right_buf[..frames]);
        }

        self.push_analysis_tap(output, frames);
    }

    #[inline]
    fn update_amplitude(&self, output: &[f32], frames: usize) {
        if !self.amp_enabled() {
            return;
        }

        let (peak_l, peak_r, sum_sq_l, sum_sq_r) = output.chunks_exact(2).take(frames).fold(
            (0.0f32, 0.0f32, 0.0f32, 0.0f32),
            |(pl, pr, sl, sr), ch| {
                let (l, r) = (ch[0], ch[1]);
                (pl.max(l.abs()), pr.max(r.abs()), sl + l * l, sr + r * r)
            },
        );

        let rms_l = (sum_sq_l / frames as f32).sqrt();
        let rms_r = (sum_sq_r / frames as f32).sqrt();

        self.amplitude_atomic().set(peak_l, peak_r, rms_l, rms_r);
    }

    #[inline]
    fn update_stereo(&self, left: &[f32], right: &[f32]) {
        self.stereo_atomic().update_from_buffers(left, right);
    }

    /// Non-blocking: skips update if LUFS lock is contended.
    #[inline]
    fn update_lufs(&self, left: &[f32], right: &[f32]) {
        if let Some(mut ebur128) = self.ebur128().try_lock() {
            let _ = ebur128.add_frames_planar_f32(&[left, right]);
        }
    }

    #[inline]
    fn update_cpu(&self, frames: usize, elapsed: Duration) {
        self.cpu().record(frames, elapsed);
    }
}
