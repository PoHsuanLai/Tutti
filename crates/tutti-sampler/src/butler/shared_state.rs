//! Shared state between butler and audio thread.
//!
//! All fields are atomic or lock-free for RT-safe cross-thread access.
//! The audio thread must never block, so we use ArcSwap for buffer access.

use arc_swap::ArcSwap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use tutti_core::AtomicFloat;

/// Shared state between butler and audio thread.
/// All fields are atomic or lock-free for RT-safe cross-thread access.
pub struct SharedStreamState {
    speed: AtomicFloat,
    target_speed: AtomicFloat,
    speed_ramp_progress: AtomicFloat,
    speed_ramp_samples: AtomicU32,
    /// 0 = forward, 1 = reverse
    direction: AtomicU8,
    seeking: AtomicBool,
    underrun_count: AtomicU64,
    /// 0-1000 representing 0.0-1.0
    buffer_fill_level: AtomicU32,

    seek_fadeout: ArcSwap<Vec<(f32, f32)>>,
    seek_fadein: ArcSwap<Vec<(f32, f32)>>,
    seek_crossfade_pos: AtomicU32,
    /// 0 = not active
    seek_crossfade_len: AtomicU32,

    loop_fadeout: ArcSwap<Vec<(f32, f32)>>,
    loop_fadein: ArcSwap<Vec<(f32, f32)>>,
    loop_crossfade_pos: AtomicU32,
    /// 0 = not active
    loop_crossfade_len: AtomicU32,

    /// file_sample_rate / session_sample_rate. 1.0 = no conversion.
    src_ratio: AtomicFloat,
}

impl Default for SharedStreamState {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedStreamState {
    pub fn new() -> Self {
        Self {
            speed: AtomicFloat::new(1.0),
            target_speed: AtomicFloat::new(1.0),
            speed_ramp_progress: AtomicFloat::new(1.0),
            speed_ramp_samples: AtomicU32::new(0),
            direction: AtomicU8::new(0),
            seeking: AtomicBool::new(false),
            underrun_count: AtomicU64::new(0),
            buffer_fill_level: AtomicU32::new(0),
            seek_fadeout: ArcSwap::from_pointee(Vec::new()),
            seek_fadein: ArcSwap::from_pointee(Vec::new()),
            seek_crossfade_pos: AtomicU32::new(0),
            seek_crossfade_len: AtomicU32::new(0),
            loop_fadeout: ArcSwap::from_pointee(Vec::new()),
            loop_fadein: ArcSwap::from_pointee(Vec::new()),
            loop_crossfade_pos: AtomicU32::new(0),
            loop_crossfade_len: AtomicU32::new(0),
            src_ratio: AtomicFloat::new(1.0),
        }
    }

    #[inline]
    pub fn speed(&self) -> f32 {
        self.speed.get()
    }

    /// Clamped to 0.25..4.0.
    pub fn set_speed(&self, speed: f32) {
        let clamped = speed.clamp(0.25, 4.0);
        self.speed.set(clamped);
        self.target_speed.set(clamped);
        self.speed_ramp_progress.set(1.0);
    }

    /// Speed will gradually transition over `ramp_samples`. Clamped to 0.25..4.0.
    pub fn set_speed_with_ramp(&self, new_speed: f32, ramp_samples: u32) {
        let clamped = new_speed.clamp(0.25, 4.0);
        self.target_speed.set(clamped);
        self.speed_ramp_progress.set(0.0);
        self.speed_ramp_samples
            .store(ramp_samples, Ordering::Release);
    }

    /// Interpolated speed if ramping, otherwise target speed.
    #[inline]
    pub fn effective_speed(&self) -> f32 {
        let progress = self.speed_ramp_progress.get();
        if progress >= 1.0 {
            return self.target_speed.get();
        }
        let current = self.speed.get();
        let target = self.target_speed.get();
        current + (target - current) * progress
    }

    /// Advance speed ramp by one sample (call from audio thread).
    #[inline]
    pub fn advance_speed_ramp(&self) {
        let samples = self.speed_ramp_samples.load(Ordering::Relaxed);
        if samples == 0 {
            return;
        }

        let progress = self.speed_ramp_progress.get();
        if progress >= 1.0 {
            return;
        }

        let increment = 1.0 / samples as f32;
        let new_progress = (progress + increment).min(1.0);
        self.speed_ramp_progress.set(new_progress);

        if new_progress >= 1.0 {
            self.speed.set(self.target_speed.get());
        }
    }

    #[inline]
    pub fn is_ramping(&self) -> bool {
        self.speed_ramp_progress.get() < 1.0
    }

    #[inline]
    pub fn is_reverse(&self) -> bool {
        self.direction.load(Ordering::Acquire) == 1
    }

    pub fn set_reverse(&self, reverse: bool) {
        self.direction
            .store(if reverse { 1 } else { 0 }, Ordering::Release);
    }

    #[inline]
    pub fn is_seeking(&self) -> bool {
        self.seeking.load(Ordering::Acquire)
    }

    pub fn set_seeking(&self, seeking: bool) {
        self.seeking.store(seeking, Ordering::Release);
    }

    #[inline]
    pub fn src_ratio(&self) -> f32 {
        self.src_ratio.get()
    }

    pub fn set_src_ratio(&self, ratio: f32) {
        self.src_ratio.set(ratio);
    }

    #[inline]
    pub fn report_underrun(&self) {
        self.underrun_count.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn underrun_count(&self) -> u64 {
        self.underrun_count.load(Ordering::Relaxed)
    }

    /// Atomically reads and resets underrun count.
    pub fn take_underrun_count(&self) -> u64 {
        self.underrun_count.swap(0, Ordering::Relaxed)
    }

    pub fn set_buffer_fill(&self, level: f32) {
        let scaled = (level.clamp(0.0, 1.0) * 1000.0) as u32;
        self.buffer_fill_level.store(scaled, Ordering::Relaxed);
    }

    /// 0.0 = empty, 1.0 = full. Near 0.0 means underrun risk.
    #[inline]
    pub fn buffer_fill(&self) -> f32 {
        self.buffer_fill_level.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// Called by butler thread. Allocation is OK here (non-RT).
    /// Audio thread blends fadeout/fadein samples lock-free.
    pub fn start_seek_crossfade(&self, fadeout: Vec<(f32, f32)>, fadein: Vec<(f32, f32)>) {
        let len = fadeout.len().min(fadein.len()) as u32;
        if len == 0 {
            return;
        }

        self.seek_fadeout.store(Arc::new(fadeout));
        self.seek_fadein.store(Arc::new(fadein));

        self.seek_crossfade_pos.store(0, Ordering::Release);
        self.seek_crossfade_len.store(len, Ordering::Release);
    }

    #[inline]
    pub fn is_seek_crossfading(&self) -> bool {
        let pos = self.seek_crossfade_pos.load(Ordering::Acquire);
        let len = self.seek_crossfade_len.load(Ordering::Acquire);
        len > 0 && pos < len
    }

    #[inline]
    pub fn seek_crossfade_len(&self) -> u32 {
        self.seek_crossfade_len.load(Ordering::Acquire)
    }

    /// Returns next blended sample, or None if crossfade is complete.
    /// Lock-free: only atomic loads, no blocking.
    pub fn next_seek_crossfade_sample(&self) -> Option<(f32, f32)> {
        let len = self.seek_crossfade_len.load(Ordering::Acquire);
        if len == 0 {
            return None;
        }

        let pos = self.seek_crossfade_pos.fetch_add(1, Ordering::AcqRel);
        if pos >= len {
            self.seek_crossfade_len.store(0, Ordering::Release);
            return None;
        }

        let t = pos as f32 / len as f32;

        let fadeout = self.seek_fadeout.load();
        let fadein = self.seek_fadein.load();

        if pos as usize >= fadeout.len() || pos as usize >= fadein.len() {
            return None;
        }

        let out_sample = fadeout[pos as usize];
        let in_sample = fadein[pos as usize];

        let left = out_sample.0 * (1.0 - t) + in_sample.0 * t;
        let right = out_sample.1 * (1.0 - t) + in_sample.1 * t;

        Some((left, right))
    }

    pub fn clear_seek_crossfade(&self) {
        self.seek_crossfade_len.store(0, Ordering::Release);
        self.seek_crossfade_pos.store(0, Ordering::Release);
        self.seek_fadeout.store(Arc::new(Vec::new()));
        self.seek_fadein.store(Arc::new(Vec::new()));
    }

    /// Called by butler when playback reaches loop boundary.
    /// Allocation is OK here (butler thread, non-RT).
    pub fn start_loop_crossfade(&self, fadeout: Vec<(f32, f32)>, fadein: Vec<(f32, f32)>) {
        let len = fadeout.len().min(fadein.len()) as u32;
        if len == 0 {
            return;
        }

        self.loop_fadeout.store(Arc::new(fadeout));
        self.loop_fadein.store(Arc::new(fadein));

        self.loop_crossfade_pos.store(0, Ordering::Release);
        self.loop_crossfade_len.store(len, Ordering::Release);
    }

    #[inline]
    pub fn is_loop_crossfading(&self) -> bool {
        let pos = self.loop_crossfade_pos.load(Ordering::Acquire);
        let len = self.loop_crossfade_len.load(Ordering::Acquire);
        len > 0 && pos < len
    }

    #[inline]
    pub fn loop_crossfade_len(&self) -> u32 {
        self.loop_crossfade_len.load(Ordering::Acquire)
    }

    /// Returns next blended sample, or None if crossfade is complete.
    /// Lock-free: only atomic loads, no blocking.
    pub fn next_loop_crossfade_sample(&self) -> Option<(f32, f32)> {
        let len = self.loop_crossfade_len.load(Ordering::Acquire);
        if len == 0 {
            return None;
        }

        let pos = self.loop_crossfade_pos.fetch_add(1, Ordering::AcqRel);
        if pos >= len {
            self.loop_crossfade_len.store(0, Ordering::Release);
            return None;
        }

        let fadeout = self.loop_fadeout.load();
        let fadein = self.loop_fadein.load();

        if pos as usize >= fadeout.len() || pos as usize >= fadein.len() {
            return None;
        }

        let t = pos as f32 / len as f32;
        let out = fadeout[pos as usize];
        let inp = fadein[pos as usize];

        Some((out.0 * (1.0 - t) + inp.0 * t, out.1 * (1.0 - t) + inp.1 * t))
    }

    pub fn clear_loop_crossfade(&self) {
        self.loop_crossfade_len.store(0, Ordering::Release);
        self.loop_crossfade_pos.store(0, Ordering::Release);
        self.loop_fadeout.store(Arc::new(Vec::new()));
        self.loop_fadein.store(Arc::new(Vec::new()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_values() {
        let state = SharedStreamState::new();
        assert_eq!(state.speed(), 1.0);
        assert!(!state.is_reverse());
        assert!(!state.is_seeking());
    }

    #[test]
    fn test_speed_clamping() {
        let state = SharedStreamState::new();

        state.set_speed(0.1);
        assert_eq!(state.speed(), 0.25);

        state.set_speed(10.0);
        assert_eq!(state.speed(), 4.0);

        state.set_speed(2.0);
        assert_eq!(state.speed(), 2.0);
    }

    #[test]
    fn test_direction() {
        let state = SharedStreamState::new();

        assert!(!state.is_reverse());

        state.set_reverse(true);
        assert!(state.is_reverse());

        state.set_reverse(false);
        assert!(!state.is_reverse());
    }

    #[test]
    fn test_seeking() {
        let state = SharedStreamState::new();

        assert!(!state.is_seeking());

        state.set_seeking(true);
        assert!(state.is_seeking());

        state.set_seeking(false);
        assert!(!state.is_seeking());
    }

    #[test]
    fn test_underrun_reporting() {
        let state = SharedStreamState::new();

        assert_eq!(state.underrun_count(), 0);

        state.report_underrun();
        state.report_underrun();
        state.report_underrun();
        assert_eq!(state.underrun_count(), 3);

        assert_eq!(state.take_underrun_count(), 3);
        assert_eq!(state.underrun_count(), 0);

        state.report_underrun();
        assert_eq!(state.underrun_count(), 1);
    }

    #[test]
    fn test_speed_ramp() {
        let state = SharedStreamState::new();

        assert_eq!(state.speed(), 1.0);
        assert_eq!(state.effective_speed(), 1.0);
        assert!(!state.is_ramping());

        state.set_speed_with_ramp(2.0, 10);
        assert!(state.is_ramping());
        assert_eq!(state.speed(), 1.0);
        assert_eq!(state.effective_speed(), 1.0);

        for _ in 0..5 {
            state.advance_speed_ramp();
        }
        let mid_speed = state.effective_speed();
        assert!((mid_speed - 1.5).abs() < 0.01);

        for _ in 0..5 {
            state.advance_speed_ramp();
        }
        assert!(!state.is_ramping());
        assert_eq!(state.effective_speed(), 2.0);
        assert_eq!(state.speed(), 2.0);
    }

    #[test]
    fn test_set_speed_cancels_ramp() {
        let state = SharedStreamState::new();

        state.set_speed_with_ramp(2.0, 100);
        assert!(state.is_ramping());

        state.set_speed(1.5);
        assert!(!state.is_ramping());
        assert_eq!(state.speed(), 1.5);
        assert_eq!(state.effective_speed(), 1.5);
    }

    #[test]
    fn test_seek_crossfade() {
        let state = SharedStreamState::new();

        assert!(!state.is_seek_crossfading());
        assert!(state.next_seek_crossfade_sample().is_none());

        let fadeout = vec![(1.0, 1.0); 4];
        let fadein = vec![(0.0, 0.0); 4];

        state.start_seek_crossfade(fadeout, fadein);

        assert!(state.is_seek_crossfading());
        assert_eq!(state.seek_crossfade_len(), 4);

        let sample = state.next_seek_crossfade_sample().unwrap();
        assert!((sample.0 - 1.0).abs() < 0.01);

        let sample = state.next_seek_crossfade_sample().unwrap();
        assert!((sample.0 - 0.75).abs() < 0.01);

        let sample = state.next_seek_crossfade_sample().unwrap();
        assert!((sample.0 - 0.5).abs() < 0.01);

        let sample = state.next_seek_crossfade_sample().unwrap();
        assert!((sample.0 - 0.25).abs() < 0.01);

        assert!(!state.is_seek_crossfading());
        assert!(state.next_seek_crossfade_sample().is_none());
    }

    #[test]
    fn test_seek_crossfade_clear() {
        let state = SharedStreamState::new();

        let fadeout = vec![(1.0, 1.0); 10];
        let fadein = vec![(0.0, 0.0); 10];
        state.start_seek_crossfade(fadeout, fadein);

        assert!(state.is_seek_crossfading());

        state.next_seek_crossfade_sample();
        state.next_seek_crossfade_sample();

        state.clear_seek_crossfade();
        assert!(!state.is_seek_crossfading());
        assert!(state.next_seek_crossfade_sample().is_none());
    }

    #[test]
    fn test_buffer_fill_level() {
        let state = SharedStreamState::new();

        assert_eq!(state.buffer_fill(), 0.0);

        state.set_buffer_fill(0.5);
        assert!((state.buffer_fill() - 0.5).abs() < 0.01);

        state.set_buffer_fill(1.0);
        assert!((state.buffer_fill() - 1.0).abs() < 0.01);

        state.set_buffer_fill(0.0);
        assert!((state.buffer_fill() - 0.0).abs() < 0.01);

        state.set_buffer_fill(-0.5);
        assert_eq!(state.buffer_fill(), 0.0);

        state.set_buffer_fill(1.5);
        assert!((state.buffer_fill() - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_loop_crossfade() {
        let state = SharedStreamState::new();

        assert!(!state.is_loop_crossfading());
        assert!(state.next_loop_crossfade_sample().is_none());

        let fadeout = vec![(1.0, 1.0); 4];
        let fadein = vec![(0.0, 0.0); 4];

        state.start_loop_crossfade(fadeout, fadein);

        assert!(state.is_loop_crossfading());
        assert_eq!(state.loop_crossfade_len(), 4);

        let sample = state.next_loop_crossfade_sample().unwrap();
        assert!((sample.0 - 1.0).abs() < 0.01);

        let sample = state.next_loop_crossfade_sample().unwrap();
        assert!((sample.0 - 0.75).abs() < 0.01);

        let sample = state.next_loop_crossfade_sample().unwrap();
        assert!((sample.0 - 0.5).abs() < 0.01);

        let sample = state.next_loop_crossfade_sample().unwrap();
        assert!((sample.0 - 0.25).abs() < 0.01);

        assert!(!state.is_loop_crossfading());
        assert!(state.next_loop_crossfade_sample().is_none());
    }

    #[test]
    fn test_loop_crossfade_clear() {
        let state = SharedStreamState::new();

        let fadeout = vec![(1.0, 1.0); 10];
        let fadein = vec![(0.0, 0.0); 10];
        state.start_loop_crossfade(fadeout, fadein);

        assert!(state.is_loop_crossfading());

        state.next_loop_crossfade_sample();
        state.next_loop_crossfade_sample();

        state.clear_loop_crossfade();
        assert!(!state.is_loop_crossfading());
        assert!(state.next_loop_crossfade_sample().is_none());
    }

    #[test]
    fn test_loop_crossfade_empty_buffers() {
        let state = SharedStreamState::new();

        state.start_loop_crossfade(Vec::new(), Vec::new());
        assert!(!state.is_loop_crossfading());

        state.start_loop_crossfade(vec![(1.0, 1.0)], Vec::new());
        assert!(!state.is_loop_crossfading());
    }
}
