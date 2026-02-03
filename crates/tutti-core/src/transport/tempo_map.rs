//! Tempo map with 3-domain time system (superclock/beats/BBT).

use crate::compat::{Arc, Vec};

pub const SUPERCLOCK_TICKS_PER_SECOND: u64 = 282_240_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TempoPoint {
    pub(crate) beat: f64,
    pub(crate) bpm: f32,
    superclock: u64,
}

impl TempoPoint {
    pub(crate) fn new(beat: f64, bpm: f32) -> Self {
        Self {
            beat,
            bpm,
            superclock: 0,
        }
    }

    #[inline]
    fn superclock_per_beat(&self) -> f64 {
        SUPERCLOCK_TICKS_PER_SECOND as f64 * 60.0 / self.bpm as f64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSignature {
    pub numerator: u32,
    pub denominator: u32,
}

impl TimeSignature {
    pub const fn new(numerator: u32, denominator: u32) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    #[inline]
    pub fn beats_per_bar(&self) -> f64 {
        self.numerator as f64 * 4.0 / self.denominator as f64
    }
}

impl Default for TimeSignature {
    fn default() -> Self {
        Self::new(4, 4)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BBT {
    pub bar: u32,
    pub beat: u32,
    pub ticks: u32,
}

impl BBT {
    pub const TICKS_PER_BEAT: u32 = 960;

    pub fn new(bar: u32, beat: u32, ticks: u32) -> Self {
        Self { bar, beat, ticks }
    }
}

#[derive(Debug, Clone)]
pub struct TempoMapSnapshot {
    points: Vec<TempoPoint>,
    time_signature: TimeSignature,
    sample_rate: f64,
}

impl TempoMapSnapshot {
    #[inline]
    pub fn beats_to_seconds(&self, beats: f64) -> f64 {
        if self.points.len() == 1 {
            beats * 60.0 / self.points[0].bpm as f64
        } else {
            self.beats_to_seconds_variable(beats)
        }
    }

    fn beats_to_seconds_variable(&self, target_beats: f64) -> f64 {
        let mut seconds = 0.0;
        let mut prev_beat = 0.0;

        for (i, point) in self.points.iter().enumerate() {
            if point.beat >= target_beats {
                let prev_tempo = if i > 0 {
                    self.points[i - 1].bpm
                } else {
                    point.bpm
                };
                seconds += (target_beats - prev_beat) * 60.0 / prev_tempo as f64;
                return seconds;
            }

            if i > 0 {
                let prev_tempo = self.points[i - 1].bpm;
                seconds += (point.beat - prev_beat) * 60.0 / prev_tempo as f64;
            }
            prev_beat = point.beat;
        }

        let last_tempo = self.points.last().map(|p| p.bpm).unwrap_or(120.0);
        seconds += (target_beats - prev_beat) * 60.0 / last_tempo as f64;
        seconds
    }

    #[inline]
    pub fn seconds_to_beats(&self, seconds: f64) -> f64 {
        if self.points.len() == 1 {
            seconds * self.points[0].bpm as f64 / 60.0
        } else {
            self.seconds_to_beats_variable(seconds)
        }
    }

    fn seconds_to_beats_variable(&self, target_seconds: f64) -> f64 {
        let mut current_seconds = 0.0;
        let mut current_beats = 0.0;

        for i in 0..self.points.len() {
            let tempo = self.points[i].bpm;
            let next_beat = if i + 1 < self.points.len() {
                self.points[i + 1].beat
            } else {
                f64::MAX
            };

            let segment_beats = next_beat - self.points[i].beat;
            let segment_seconds = segment_beats * 60.0 / tempo as f64;

            if current_seconds + segment_seconds >= target_seconds {
                let remaining_seconds = target_seconds - current_seconds;
                return current_beats + remaining_seconds * tempo as f64 / 60.0;
            }

            current_seconds += segment_seconds;
            current_beats = next_beat;
        }

        current_beats
    }

    #[inline]
    pub fn beats_to_samples(&self, beats: f64) -> u64 {
        let seconds = self.beats_to_seconds(beats);
        (seconds * self.sample_rate) as u64
    }

    #[inline]
    pub fn samples_to_beats(&self, samples: u64) -> f64 {
        let seconds = samples as f64 / self.sample_rate;
        self.seconds_to_beats(seconds)
    }

    pub fn beats_to_bbt(&self, beats: f64) -> BBT {
        let beats_per_bar = self.time_signature.beats_per_bar();

        let total_bars = (beats / beats_per_bar).floor();
        let beat_in_bar = beats - (total_bars * beats_per_bar);
        let beat_whole = beat_in_bar.floor();
        let ticks = ((beat_in_bar - beat_whole) * BBT::TICKS_PER_BEAT as f64) as u32;

        BBT {
            bar: total_bars as u32 + 1,
            beat: beat_whole as u32 + 1, // 1-indexed
            ticks,
        }
    }

    pub fn bbt_to_beats(&self, bbt: BBT) -> f64 {
        let beats_per_bar = self.time_signature.beats_per_bar();
        let bar_beats = (bbt.bar.saturating_sub(1)) as f64 * beats_per_bar;
        let beat_beats = (bbt.beat.saturating_sub(1)) as f64;
        let tick_beats = bbt.ticks as f64 / BBT::TICKS_PER_BEAT as f64;

        bar_beats + beat_beats + tick_beats
    }

    #[inline]
    pub fn beats_per_second(&self) -> f64 {
        self.points[0].bpm as f64 / 60.0
    }

    #[inline]
    pub fn samples_per_beat(&self) -> f64 {
        self.sample_rate / self.beats_per_second()
    }

    #[inline]
    pub fn tempo(&self) -> f32 {
        self.points[0].bpm
    }

    #[inline]
    pub fn time_signature(&self) -> TimeSignature {
        self.time_signature
    }

    #[inline]
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }
}

#[derive(Debug, Clone)]
pub struct TempoMap {
    points: Vec<TempoPoint>,
    time_signature: TimeSignature,
    sample_rate: f64,
    snapshot: Arc<TempoMapSnapshot>,
}

impl TempoMap {
    pub fn new(initial_bpm: f32, sample_rate: f64) -> Self {
        let points = vec![TempoPoint::new(0.0, initial_bpm)];
        let time_signature = TimeSignature::default();

        let snapshot = Arc::new(TempoMapSnapshot {
            points: points.clone(),
            time_signature,
            sample_rate,
        });

        Self {
            points,
            time_signature,
            sample_rate,
            snapshot,
        }
    }

    pub fn snapshot(&self) -> Arc<TempoMapSnapshot> {
        Arc::clone(&self.snapshot)
    }

    pub fn set_tempo(&mut self, bpm: f32) {
        let bpm = bpm.clamp(1.0, 999.0);
        self.points.clear();
        self.points.push(TempoPoint::new(0.0, bpm));
        self.rebuild_snapshot();
    }

    pub fn tempo(&self) -> f32 {
        self.points[0].bpm
    }

    pub fn add_tempo_point(&mut self, beat: f64, bpm: f32) {
        let bpm = bpm.clamp(1.0, 999.0);

        self.points.retain(|p| (p.beat - beat).abs() > 0.001);

        self.points.push(TempoPoint::new(beat, bpm));

        self.points.sort_by(|a, b| {
            a.beat
                .partial_cmp(&b.beat)
                .expect("Beat values should not be NaN")
        });

        if self.points[0].beat > 0.0 {
            self.points
                .insert(0, TempoPoint::new(0.0, self.points[0].bpm));
        }

        self.rebuild_snapshot();
    }

    pub fn remove_tempo_point(&mut self, beat: f64) {
        if beat <= 0.001 {
            return;
        }
        self.points.retain(|p| (p.beat - beat).abs() > 0.001);
        self.rebuild_snapshot();
    }

    pub fn clear_tempo_automation(&mut self) {
        let initial_bpm = self.points[0].bpm;
        self.points.clear();
        self.points.push(TempoPoint::new(0.0, initial_bpm));
        self.rebuild_snapshot();
    }

    pub fn set_time_signature(&mut self, numerator: u32, denominator: u32) {
        self.time_signature = TimeSignature::new(numerator, denominator);
        self.rebuild_snapshot();
    }

    pub fn time_signature(&self) -> TimeSignature {
        self.time_signature
    }

    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
        self.rebuild_snapshot();
    }

    fn rebuild_snapshot(&mut self) {
        let mut superclock = 0u64;
        for i in 0..self.points.len() {
            self.points[i].superclock = superclock;

            if i + 1 < self.points.len() {
                let beat_delta = self.points[i + 1].beat - self.points[i].beat;
                let sc_delta = (beat_delta * self.points[i].superclock_per_beat()) as u64;
                superclock += sc_delta;
            }
        }

        self.snapshot = Arc::new(TempoMapSnapshot {
            points: self.points.clone(),
            time_signature: self.time_signature,
            sample_rate: self.sample_rate,
        });
    }

    #[inline]
    pub fn beats_to_seconds(&self, beats: f64) -> f64 {
        self.snapshot.beats_to_seconds(beats)
    }

    #[inline]
    pub fn seconds_to_beats(&self, seconds: f64) -> f64 {
        self.snapshot.seconds_to_beats(seconds)
    }

    #[inline]
    pub fn beats_to_samples(&self, beats: f64) -> u64 {
        self.snapshot.beats_to_samples(beats)
    }

    #[inline]
    pub fn samples_to_beats(&self, samples: u64) -> f64 {
        self.snapshot.samples_to_beats(samples)
    }

    #[inline]
    pub fn beats_to_bbt(&self, beats: f64) -> BBT {
        self.snapshot.beats_to_bbt(beats)
    }

    #[inline]
    pub fn bbt_to_beats(&self, bbt: BBT) -> f64 {
        self.snapshot.bbt_to_beats(bbt)
    }
}

impl Default for TempoMap {
    fn default() -> Self {
        Self::new(120.0, 44100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_tempo_conversion() {
        let map = TempoMap::new(120.0, 44100.0);
        assert!((map.beats_to_seconds(2.0) - 1.0).abs() < 0.001);
        assert!((map.seconds_to_beats(1.0) - 2.0).abs() < 0.001);
        assert_eq!(map.beats_to_samples(2.0), 44100);
        assert!((map.samples_to_beats(44100) - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_bbt_conversion() {
        let map = TempoMap::new(120.0, 44100.0);
        let bbt = map.beats_to_bbt(0.0);
        assert_eq!(bbt.bar, 1);
        assert_eq!(bbt.beat, 1);
        assert_eq!(bbt.ticks, 0);
        let bbt = map.beats_to_bbt(4.0);
        assert_eq!(bbt.bar, 2);
        assert_eq!(bbt.beat, 1);
        let bbt = map.beats_to_bbt(5.5);
        assert_eq!(bbt.bar, 2);
        assert_eq!(bbt.beat, 2);
        assert_eq!(bbt.ticks, 480);
        let beats = map.bbt_to_beats(BBT::new(2, 2, 480));
        assert!((beats - 5.5).abs() < 0.001);
    }

    #[test]
    fn test_tempo_change() {
        let mut map = TempoMap::new(120.0, 44100.0);
        map.add_tempo_point(4.0, 60.0); // Slow down at beat 4

        // First 4 beats at 120 BPM = 2 seconds
        // Then beats 4-8 at 60 BPM = 4 seconds
        // Total: beat 8 = 6 seconds
        let seconds = map.beats_to_seconds(8.0);
        assert!(
            (seconds - 6.0).abs() < 0.01,
            "Expected 6.0, got {}",
            seconds
        );
    }

    #[test]
    fn test_time_signature() {
        let mut map = TempoMap::new(120.0, 44100.0);
        map.set_time_signature(3, 4); // Waltz time

        // 3/4 = 3 beats per bar

        let bbt = map.beats_to_bbt(6.0);
        assert_eq!(bbt.bar, 3);
        assert_eq!(bbt.beat, 1);
    }

    #[test]
    fn test_snapshot_isolation() {
        let mut map = TempoMap::new(120.0, 44100.0);
        let snap1 = map.snapshot();

        map.set_tempo(60.0);
        let snap2 = map.snapshot();

        // Snapshots should be independent
        assert!((snap1.tempo() - 120.0).abs() < 0.001);
        assert!((snap2.tempo() - 60.0).abs() < 0.001);
    }

    #[test]
    fn test_tempo_bounds() {
        let mut map = TempoMap::new(120.0, 44100.0);

        // Tempo should be clamped
        map.set_tempo(0.5);
        assert!((map.tempo() - 1.0).abs() < 0.001);

        map.set_tempo(1500.0);
        assert!((map.tempo() - 999.0).abs() < 0.001);
    }

    #[test]
    fn test_multiple_tempo_changes() {
        let mut map = TempoMap::new(60.0, 48000.0);

        // 60 BPM: 1 beat = 1 second
        map.add_tempo_point(4.0, 120.0);
        map.add_tempo_point(8.0, 240.0);

        // Beats 0-4 at 60 BPM = 4 seconds
        // Beats 4-8 at 120 BPM = 2 seconds
        // Total to beat 8 = 6 seconds
        let seconds_at_8 = map.beats_to_seconds(8.0);
        assert!(
            (seconds_at_8 - 6.0).abs() < 0.01,
            "Expected 6.0, got {}",
            seconds_at_8
        );

        // Beats 8-12 at 240 BPM = 1 second
        // Total to beat 12 = 7 seconds
        let seconds_at_12 = map.beats_to_seconds(12.0);
        assert!(
            (seconds_at_12 - 7.0).abs() < 0.01,
            "Expected 7.0, got {}",
            seconds_at_12
        );
    }

    #[test]
    fn test_seconds_to_beats_with_tempo_changes() {
        let mut map = TempoMap::new(60.0, 48000.0);
        map.add_tempo_point(4.0, 120.0); // Speed up at beat 4

        // At 60 BPM: 2 seconds = 2 beats (before beat 4)
        let beats = map.seconds_to_beats(2.0);
        assert!((beats - 2.0).abs() < 0.01, "Expected 2.0, got {}", beats);

        // At 60 BPM: 4 seconds = 4 beats
        let beats = map.seconds_to_beats(4.0);
        assert!((beats - 4.0).abs() < 0.01, "Expected 4.0, got {}", beats);

        // 5 seconds: 4 beats at 60 BPM (4s) + 2 beats at 120 BPM (1s) = 6 beats
        let beats = map.seconds_to_beats(5.0);
        assert!((beats - 6.0).abs() < 0.01, "Expected 6.0, got {}", beats);
    }

    #[test]
    fn test_superclock_constant() {
        // Verify superclock has good divisibility properties
        // 282,240,000 / 44100 = 6400 (exact)
        // 282,240,000 / 48000 = 5880 (exact)
        assert_eq!(SUPERCLOCK_TICKS_PER_SECOND % 44100, 0);
        assert_eq!(SUPERCLOCK_TICKS_PER_SECOND % 48000, 0);
    }

    #[test]
    fn test_tempo_point_construction() {
        let point = TempoPoint::new(8.0, 140.0);
        assert!((point.beat - 8.0).abs() < 0.001);
        assert!((point.bpm - 140.0).abs() < 0.001);
    }

    #[test]
    fn test_sample_rate_change() {
        let mut map = TempoMap::new(120.0, 44100.0);

        // At 44100 Hz, 2 beats at 120 BPM = 44100 samples
        assert_eq!(map.beats_to_samples(2.0), 44100);

        // Change sample rate to 48000
        map.set_sample_rate(48000.0);

        // At 48000 Hz, 2 beats at 120 BPM = 48000 samples
        assert_eq!(map.beats_to_samples(2.0), 48000);
    }

    #[test]
    fn test_snapshot_beats_per_second() {
        let map = TempoMap::new(120.0, 44100.0);
        let snap = map.snapshot();
        assert!((snap.beats_per_second() - 2.0).abs() < 0.001);

        // Samples per beat = 44100 / 2 = 22050
        assert!((snap.samples_per_beat() - 22050.0).abs() < 0.1);
    }
}
