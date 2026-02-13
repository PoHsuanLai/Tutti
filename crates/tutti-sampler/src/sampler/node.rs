//! Sample playback nodes.
//!
//! - `SamplerUnit`: In-memory playback with loop crossfade support
//! - `StreamingSamplerUnit`: Disk streaming playback

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tutti_core::{AudioUnit, BufferMut, BufferRef, SignalFrame, TransportReader, Wave};

use crate::butler::LoopCrossfade;

/// In-memory sample playback with optional loop crossfade.
///
/// By default, plays immediately when added to the graph (suitable for timeline clips
/// and offline export). Use `stop()` and `trigger()` for manual control if needed
/// (e.g., MIDI-triggered one-shots).
pub struct SamplerUnit {
    wave: Arc<Wave>,
    position: AtomicU64,

    /// Defaults to true (auto-play).
    playing: AtomicBool,

    looping: AtomicBool,

    gain: f32,

    speed: f32,

    sample_rate: f32,

    /// SRC ratio: file_sample_rate / session_sample_rate. 1.0 = no conversion.
    src_ratio: f32,

    /// Loop range (start, end) in samples. If None, loops entire sample.
    loop_range: Option<(u64, u64)>,

    /// Crossfade for smooth loop transitions.
    crossfade: Option<LoopCrossfade>,

    /// Optional transport for beat-synced playback.
    /// When set, sampler only plays when transport is rolling
    /// and uses beat position to compute sample offset.
    transport: Option<Arc<dyn TransportReader>>,

    /// Start position in beats on the timeline.
    start_beat: f64,

    /// Duration in beats (0.0 = play entire sample).
    duration_beats: f64,
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
            src_ratio: self.src_ratio,
            loop_range: self.loop_range,
            crossfade: self.crossfade.clone(),
            transport: self.transport.clone(),
            start_beat: self.start_beat,
            duration_beats: self.duration_beats,
        }
    }
}

impl SamplerUnit {
    pub fn new(wave: Arc<Wave>) -> Self {
        let sample_rate = wave.sample_rate() as f32;
        Self {
            wave,
            position: AtomicU64::new(0),
            playing: AtomicBool::new(true),
            looping: AtomicBool::new(false),
            gain: 1.0,
            speed: 1.0,
            sample_rate,
            src_ratio: 1.0,
            loop_range: None,
            crossfade: None,
            transport: None,
            start_beat: 0.0,
            duration_beats: 0.0,
        }
    }

    pub fn with_settings(wave: Arc<Wave>, gain: f32, speed: f32, looping: bool) -> Self {
        let sample_rate = wave.sample_rate() as f32;
        Self {
            wave,
            position: AtomicU64::new(0),
            playing: AtomicBool::new(true),
            looping: AtomicBool::new(looping),
            gain,
            speed,
            sample_rate,
            src_ratio: 1.0,
            loop_range: None,
            crossfade: None,
            transport: None,
            start_beat: 0.0,
            duration_beats: 0.0,
        }
    }

    pub fn with_transport(
        wave: Arc<Wave>,
        transport: Arc<dyn TransportReader>,
        start_beat: f64,
        duration_beats: f64,
    ) -> Self {
        let sample_rate = wave.sample_rate() as f32;
        Self {
            wave,
            position: AtomicU64::new(0),
            playing: AtomicBool::new(true),
            looping: AtomicBool::new(false),
            gain: 1.0,
            speed: 1.0,
            sample_rate,
            src_ratio: 1.0,
            loop_range: None,
            crossfade: None,
            transport: Some(transport),
            start_beat,
            duration_beats,
        }
    }

    pub fn set_transport(
        &mut self,
        transport: Arc<dyn TransportReader>,
        start_beat: f64,
        duration_beats: f64,
    ) {
        self.transport = Some(transport);
        self.start_beat = start_beat;
        self.duration_beats = duration_beats;
    }

    /// Used by export to inject export timeline.
    pub fn replace_transport(&mut self, transport: Arc<dyn TransportReader>) {
        self.transport = Some(transport);
    }

    pub fn has_transport(&self) -> bool {
        self.transport.is_some()
    }

    pub fn trigger(&self) {
        self.position.store(0, Ordering::Relaxed);
        self.playing.store(true, Ordering::Relaxed);
    }

    pub fn trigger_at(&self, position: u64) {
        self.position.store(position, Ordering::Relaxed);
        self.playing.store(true, Ordering::Relaxed);
    }

    pub fn stop(&self) {
        self.playing.store(false, Ordering::Relaxed);
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    pub fn set_looping(&self, looping: bool) {
        self.looping.store(looping, Ordering::Relaxed);
    }

    pub fn is_looping(&self) -> bool {
        self.looping.load(Ordering::Relaxed)
    }

    pub fn position(&self) -> u64 {
        self.position.load(Ordering::Relaxed)
    }

    pub fn start_beat(&self) -> f64 {
        self.start_beat
    }

    /// 0.0 means play entire sample.
    pub fn duration_beats(&self) -> f64 {
        self.duration_beats
    }

    pub fn duration_samples(&self) -> usize {
        self.wave.len()
    }

    pub fn duration_seconds(&self) -> f64 {
        self.wave.duration()
    }

    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain;
    }

    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Computes SRC ratio from file vs session sample rate.
    pub fn set_session_sample_rate(&mut self, session_rate: f64) {
        let file_rate = self.wave.sample_rate();
        self.src_ratio = if (file_rate - session_rate).abs() < 0.01 {
            1.0
        } else {
            (file_rate / session_rate) as f32
        };
    }

    pub fn set_loop_range(&mut self, loop_start: u64, loop_end: u64, crossfade_samples: usize) {
        self.loop_range = Some((loop_start, loop_end));
        self.looping.store(true, Ordering::Relaxed);

        if crossfade_samples > 0 {
            let mut xfade = LoopCrossfade::new(crossfade_samples);

            let preloop_samples: Vec<_> = (0..crossfade_samples)
                .map(|i| self.get_sample_raw(loop_start as f64 + i as f64))
                .collect();
            xfade.fill_preloop(&preloop_samples);

            self.crossfade = Some(xfade);
        } else {
            self.crossfade = None;
        }
    }

    pub fn clear_loop_range(&mut self) {
        self.loop_range = None;
        self.crossfade = None;
    }

    pub fn loop_range(&self) -> Option<(u64, u64)> {
        self.loop_range
    }

    #[inline]
    fn get_sample_raw(&self, position: f64) -> (f32, f32) {
        let len = self.wave.len() as f64;
        if position >= len {
            return (0.0, 0.0);
        }

        let idx = position.floor() as usize;
        let frac = position.fract() as f32;

        let (l0, r0) = if self.wave.channels() >= 2 {
            (self.wave.at(0, idx), self.wave.at(1, idx))
        } else {
            let mono = self.wave.at(0, idx);
            (mono, mono)
        };

        let next_idx = (idx + 1).min(self.wave.len().saturating_sub(1));
        let (l1, r1) = if self.wave.channels() >= 2 {
            (self.wave.at(0, next_idx), self.wave.at(1, next_idx))
        } else {
            let mono = self.wave.at(0, next_idx);
            (mono, mono)
        };

        let left = l0 + (l1 - l0) * frac;
        let right = r0 + (r1 - r0) * frac;

        (left, right)
    }

    #[inline]
    fn get_sample(&self, position: f64) -> (f32, f32) {
        let (l, r) = self.get_sample_raw(position);
        (l * self.gain, r * self.gain)
    }

    #[inline]
    fn transport_sample_position(&self) -> Option<f64> {
        let transport = self.transport.as_ref()?;
        if !transport.is_playing() {
            return None;
        }
        let current_beat = transport.current_beat();
        let beat_offset = current_beat - self.start_beat;
        if beat_offset < 0.0 {
            return None;
        }
        if self.duration_beats > 0.0 && beat_offset >= self.duration_beats {
            return None;
        }
        let tempo = transport.tempo() as f64;
        if tempo <= 0.0 {
            return None;
        }
        let seconds_offset = beat_offset * 60.0 / tempo;
        Some(seconds_offset * self.wave.sample_rate())
    }
}

impl AudioUnit for SamplerUnit {
    fn inputs(&self) -> usize {
        0
    }

    fn outputs(&self) -> usize {
        2
    }

    fn reset(&mut self) {
        self.position.store(0, Ordering::Relaxed);
        self.playing.store(false, Ordering::Relaxed);
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate as f32;
        self.set_session_sample_rate(sample_rate);
    }

    fn tick(&mut self, _input: &[f32], output: &mut [f32]) {
        // Transport-aware path: position derived from beat
        if self.transport.is_some() {
            if output.len() >= 2 {
                match self.transport_sample_position() {
                    None => {
                        output[0] = 0.0;
                        output[1] = 0.0;
                    }
                    Some(pos) => {
                        let (left, right) = self.get_sample(pos);
                        output[0] = left;
                        output[1] = right;
                    }
                }
            }
            return;
        }

        // Self-advancing path (no transport)
        if !self.playing.load(Ordering::Relaxed) {
            if output.len() >= 2 {
                output[0] = 0.0;
                output[1] = 0.0;
            }
            return;
        }

        let pos_bits = self.position.load(Ordering::Relaxed);
        let pos = f64::from_bits(pos_bits);

        let (mut left, mut right) = self.get_sample(pos);

        let (loop_start, loop_end) = self
            .loop_range
            .map(|(s, e)| (s as f64, e as f64))
            .unwrap_or((0.0, self.wave.len() as f64));

        if let Some(ref mut xfade) = self.crossfade {
            let crossfade_start = loop_end - xfade.len() as f64;
            if pos >= crossfade_start && pos < loop_end && !xfade.is_active() {
                xfade.start();
            }
            if xfade.is_active() {
                let sample = xfade.process((left, right));
                left = sample.0;
                right = sample.1;
            }
        }

        if output.len() >= 2 {
            output[0] = left;
            output[1] = right;
        }

        let new_pos = pos + (self.speed * self.src_ratio) as f64;

        if new_pos >= loop_end {
            if self.looping.load(Ordering::Relaxed) {
                let overshoot = new_pos - loop_end;
                let wrapped = loop_start + overshoot;
                self.position.store(wrapped.to_bits(), Ordering::Relaxed);

                if let Some(ref mut xfade) = self.crossfade {
                    xfade.reset();
                }
            } else {
                self.playing.store(false, Ordering::Relaxed);
                self.position.store(loop_end.to_bits(), Ordering::Relaxed);
            }
        } else {
            self.position.store(new_pos.to_bits(), Ordering::Relaxed);
        }
    }

    fn process(&mut self, size: usize, _input: &BufferRef, output: &mut BufferMut) {
        // Transport-aware path: compute position at block start, advance per-sample
        if self.transport.is_some() {
            match self.transport_sample_position() {
                None => {
                    for i in 0..size {
                        output.set_f32(0, i, 0.0);
                        output.set_f32(1, i, 0.0);
                    }
                }
                Some(start_pos) => {
                    let advance = (self.speed * self.src_ratio) as f64;
                    for i in 0..size {
                        let pos = start_pos + i as f64 * advance;
                        let (left, right) = self.get_sample(pos);
                        output.set_f32(0, i, left);
                        output.set_f32(1, i, right);
                    }
                }
            }
            return;
        }

        // Self-advancing path (no transport)
        if !self.playing.load(Ordering::Relaxed) {
            for i in 0..size {
                output.set_f32(0, i, 0.0);
                output.set_f32(1, i, 0.0);
            }
            return;
        }

        let mut pos_bits = self.position.load(Ordering::Relaxed);
        let looping = self.looping.load(Ordering::Relaxed);

        let (loop_start, loop_end) = self
            .loop_range
            .map(|(s, e)| (s as f64, e as f64))
            .unwrap_or((0.0, self.wave.len() as f64));

        let crossfade_start = self
            .crossfade
            .as_ref()
            .map(|xf| loop_end - xf.len() as f64)
            .unwrap_or(loop_end);

        for i in 0..size {
            let pos = f64::from_bits(pos_bits);

            if pos >= loop_end {
                if looping {
                    let overshoot = pos - loop_end;
                    let wrapped = loop_start + overshoot;
                    pos_bits = wrapped.to_bits();

                    if let Some(ref mut xfade) = self.crossfade {
                        xfade.reset();
                    }
                } else {
                    self.playing.store(false, Ordering::Relaxed);
                    for j in i..size {
                        output.set_f32(0, j, 0.0);
                        output.set_f32(1, j, 0.0);
                    }
                    break;
                }
            }

            let current_pos = f64::from_bits(pos_bits);
            let (mut left, mut right) = self.get_sample(current_pos);

            if let Some(ref mut xfade) = self.crossfade {
                if current_pos >= crossfade_start && current_pos < loop_end && !xfade.is_active() {
                    xfade.start();
                }
                if xfade.is_active() {
                    let sample = xfade.process((left, right));
                    left = sample.0;
                    right = sample.1;
                }
            }

            output.set_f32(0, i, left);
            output.set_f32(1, i, right);

            let new_pos = current_pos + (self.speed * self.src_ratio) as f64;
            pos_bits = new_pos.to_bits();
        }

        self.position.store(pos_bits, Ordering::Relaxed);
    }

    fn get_id(&self) -> u64 {
        Arc::as_ptr(&self.wave) as *const () as usize as u64
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn route(&mut self, _input: &SignalFrame, _frequency: f64) -> SignalFrame {
        SignalFrame::new(2)
    }

    fn footprint(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

mod streaming {
    use super::*;
    use crate::butler::{RegionBufferConsumer, SharedStreamState};
    use parking_lot::Mutex;

    /// Cubic Hermite interpolation for smooth varispeed playback.
    #[inline]
    pub(super) fn cubic_hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
        let c0 = y1;
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        ((c3 * t + c2) * t + c1) * t + c0
    }

    /// 8192 frames at 4x speed with interpolation padding.
    const MAX_FETCH_SAMPLES: usize = 8192 * 4 + 8;

    /// Disk streaming sampler with varispeed, seeking, and crossfade support.
    pub struct StreamingSamplerUnit {
        consumer: Arc<Mutex<RegionBufferConsumer>>,
        playing: AtomicBool,

        gain: f32,
        sample_rate: f32,

        /// Shared state for cross-thread communication (speed, direction, seeking).
        shared_state: Option<Arc<SharedStreamState>>,

        /// Fractional position for sub-sample interpolation.
        fractional_pos: f64,

        /// History buffer for cubic Hermite interpolation (last 4 samples).
        history: [(f32, f32); 4],

        /// Pre-allocated scratch buffer for fetched samples (RT-safe).
        fetch_scratch: Vec<(f32, f32)>,
    }

    impl Clone for StreamingSamplerUnit {
        fn clone(&self) -> Self {
            Self {
                consumer: Arc::clone(&self.consumer),
                playing: AtomicBool::new(self.playing.load(Ordering::Relaxed)),
                gain: self.gain,
                sample_rate: self.sample_rate,
                shared_state: self.shared_state.clone(),
                fractional_pos: self.fractional_pos,
                history: self.history,
                fetch_scratch: Vec::with_capacity(MAX_FETCH_SAMPLES),
            }
        }
    }

    impl StreamingSamplerUnit {
        pub fn new(
            consumer: Arc<Mutex<RegionBufferConsumer>>,
            shared_state: Arc<SharedStreamState>,
        ) -> Self {
            Self {
                consumer,
                playing: AtomicBool::new(true),
                gain: 1.0,
                sample_rate: 44100.0,
                shared_state: Some(shared_state),
                fractional_pos: 0.0,
                history: [(0.0, 0.0); 4],
                fetch_scratch: Vec::with_capacity(MAX_FETCH_SAMPLES),
            }
        }

        pub fn play(&self) {
            self.playing.store(true, Ordering::Relaxed);
        }

        pub fn stop(&self) {
            self.playing.store(false, Ordering::Relaxed);
        }

        pub fn is_playing(&self) -> bool {
            self.playing.load(Ordering::Relaxed)
        }

        pub fn set_gain(&mut self, gain: f32) {
            self.gain = gain;
        }

        #[inline]
        fn shift_history(&mut self) {
            self.history[0] = self.history[1];
            self.history[1] = self.history[2];
            self.history[2] = self.history[3];
        }

        /// Call after seek to reset interpolation state.
        pub fn reset_interpolation(&mut self) {
            self.fractional_pos = 0.0;
            self.history = [(0.0, 0.0); 4];
        }

        fn process_normal_samples(&mut self, size: usize, offset: usize, output: &mut BufferMut) {
            if size == 0 {
                return;
            }

            let src_ratio = self
                .shared_state
                .as_ref()
                .map(|s| s.src_ratio())
                .unwrap_or(1.0) as f64;
            let base_speed = self
                .shared_state
                .as_ref()
                .map(|s| s.effective_speed())
                .unwrap_or(1.0) as f64
                * src_ratio;

            let samples_needed = (size as f64 * base_speed).ceil() as usize + 4;

            // RT-safe: clear and reuse pre-allocated scratch buffer
            self.fetch_scratch.clear();

            if let Some(mut guard) = self.consumer.try_lock() {
                for _ in 0..samples_needed {
                    if let Some((left, right)) = guard.read() {
                        self.fetch_scratch
                            .push((left * self.gain, right * self.gain));
                    } else {
                        break;
                    }
                }
            }

            let mut fetch_idx = 0;
            for i in 0..size {
                let speed = self
                    .shared_state
                    .as_ref()
                    .map(|s| {
                        s.advance_speed_ramp();
                        s.effective_speed() as f64 * s.src_ratio() as f64
                    })
                    .unwrap_or(1.0);

                self.fractional_pos += speed;

                while self.fractional_pos >= 1.0 {
                    self.fractional_pos -= 1.0;
                    self.shift_history();

                    if fetch_idx < self.fetch_scratch.len() {
                        self.history[3] = self.fetch_scratch[fetch_idx];
                        fetch_idx += 1;
                    } else {
                        if let Some(ref state) = self.shared_state {
                            state.report_underrun();
                        }
                        self.history[3] = self.history[2];
                    }
                }

                let t = self.fractional_pos as f32;
                let left = cubic_hermite(
                    self.history[0].0,
                    self.history[1].0,
                    self.history[2].0,
                    self.history[3].0,
                    t,
                );
                let right = cubic_hermite(
                    self.history[0].1,
                    self.history[1].1,
                    self.history[2].1,
                    self.history[3].1,
                    t,
                );

                output.set_f32(0, offset + i, left);
                output.set_f32(1, offset + i, right);
            }
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
            self.reset_interpolation();
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

            if let Some(ref state) = self.shared_state {
                if let Some((left, right)) = state.next_seek_crossfade_sample() {
                    if output.len() >= 2 {
                        output[0] = left * self.gain;
                        output[1] = right * self.gain;
                    }
                    return;
                }

                if state.is_seeking() {
                    if output.len() >= 2 {
                        output[0] = 0.0;
                        output[1] = 0.0;
                    }
                    return;
                }
            }

            let speed = self
                .shared_state
                .as_ref()
                .map(|s| {
                    s.advance_speed_ramp();
                    s.effective_speed() as f64 * s.src_ratio() as f64
                })
                .unwrap_or(1.0);

            self.fractional_pos += speed;

            while self.fractional_pos >= 1.0 {
                self.fractional_pos -= 1.0;
                self.shift_history();

                if let Some(mut guard) = self.consumer.try_lock() {
                    if let Some((left, right)) = guard.read() {
                        self.history[3] = (left * self.gain, right * self.gain);
                    } else {
                        if let Some(ref state) = self.shared_state {
                            state.report_underrun();
                        }
                        self.history[3] = self.history[2];
                    }
                } else {
                    if let Some(ref state) = self.shared_state {
                        state.report_underrun();
                    }
                    self.history[3] = self.history[2];
                }
            }

            let t = self.fractional_pos as f32;
            let left = cubic_hermite(
                self.history[0].0,
                self.history[1].0,
                self.history[2].0,
                self.history[3].0,
                t,
            );
            let right = cubic_hermite(
                self.history[0].1,
                self.history[1].1,
                self.history[2].1,
                self.history[3].1,
                t,
            );

            if output.len() >= 2 {
                output[0] = left;
                output[1] = right;
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

            if let Some(ref state) = self.shared_state {
                if state.is_seek_crossfading() {
                    for i in 0..size {
                        if let Some((left, right)) = state.next_seek_crossfade_sample() {
                            output.set_f32(0, i, left * self.gain);
                            output.set_f32(1, i, right * self.gain);
                        } else {
                            self.process_normal_samples(size - i, i, output);
                            return;
                        }
                    }
                    return;
                }

                if state.is_loop_crossfading() {
                    for i in 0..size {
                        if let Some((left, right)) = state.next_loop_crossfade_sample() {
                            output.set_f32(0, i, left * self.gain);
                            output.set_f32(1, i, right * self.gain);
                        } else {
                            self.process_normal_samples(size - i, i, output);
                            return;
                        }
                    }
                    return;
                }

                if state.is_seeking() {
                    for i in 0..size {
                        output.set_f32(0, i, 0.0);
                        output.set_f32(1, i, 0.0);
                    }
                    return;
                }
            }

            self.process_normal_samples(size, 0, output);
        }

        fn get_id(&self) -> u64 {
            Arc::as_ptr(&self.consumer) as *const () as usize as u64
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
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
        let wave = Wave::with_capacity(1, 44100.0, 100);
        let sampler = SamplerUnit::new(Arc::new(wave));

        // SamplerUnit auto-plays by default (playing: true)
        assert!(sampler.is_playing());
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

        // Stop the auto-playing sampler
        sampler.stop();

        let mut output = [0.0f32; 2];
        sampler.tick(&[], &mut output);

        assert_eq!(output[0], 0.0);
        assert_eq!(output[1], 0.0);
    }

    #[test]
    fn test_loop_range_api() {
        let samples = vec![0.0f32; 1000];
        let wave = Wave::from_samples(44100.0, &samples);
        let mut sampler = SamplerUnit::new(Arc::new(wave));

        assert!(sampler.loop_range().is_none());

        sampler.set_loop_range(100, 500, 64);

        assert_eq!(sampler.loop_range(), Some((100, 500)));
        assert!(sampler.is_looping());

        sampler.clear_loop_range();
        assert!(sampler.loop_range().is_none());
    }

    #[test]
    fn test_loop_crossfade_integration() {
        let samples: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
        let wave = Wave::from_samples(44100.0, &samples);
        let mut sampler = SamplerUnit::new(Arc::new(wave));

        sampler.set_loop_range(10, 90, 10);
        sampler.trigger();

        for _ in 0..75 {
            let mut output = [0.0f32; 2];
            sampler.tick(&[], &mut output);
        }

        let mut output = [0.0f32; 2];
        sampler.tick(&[], &mut output);

        assert!(sampler.is_playing());
    }

    mod streaming_tests {
        #[test]
        fn test_cubic_hermite_interpolation() {
            use super::streaming::cubic_hermite;
            let result = cubic_hermite(0.0, 1.0, 2.0, 3.0, 0.0);
            assert!((result - 1.0).abs() < 0.001);

            let result = cubic_hermite(0.0, 1.0, 2.0, 3.0, 1.0);
            assert!((result - 2.0).abs() < 0.001);

            let result = cubic_hermite(0.0, 1.0, 2.0, 3.0, 0.5);
            assert!((result - 1.5).abs() < 0.1);
        }

        #[test]
        fn test_shared_stream_state_seeking() {
            use crate::butler::SharedStreamState;

            let state = SharedStreamState::new();

            assert!(!state.is_seeking());

            state.set_seeking(true);
            assert!(state.is_seeking());

            state.set_seeking(false);
            assert!(!state.is_seeking());
        }

        #[test]
        fn test_shared_stream_state_speed() {
            use crate::butler::SharedStreamState;

            let state = SharedStreamState::new();

            assert_eq!(state.speed(), 1.0);

            state.set_speed(0.5);
            assert_eq!(state.speed(), 0.5);

            state.set_speed(2.0);
            assert_eq!(state.speed(), 2.0);

            state.set_speed(0.1);
            assert_eq!(state.speed(), 0.25);

            state.set_speed(10.0);
            assert_eq!(state.speed(), 4.0);
        }
    }
}
