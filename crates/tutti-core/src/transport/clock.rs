//! Sample-accurate transport clock.

use crate::compat::{any, Arc, Vec};
use crate::lockfree::{AtomicDouble, AtomicFlag, AtomicFloat};
use fundsp::prelude::*;

pub struct TransportClock {
    current_beat: f64,
    tempo: Arc<AtomicFloat>,
    paused: Arc<AtomicFlag>,
    seek_target: Arc<AtomicDouble>,
    seek_pending: Arc<AtomicFlag>,
    position_writeback: Option<Arc<AtomicDouble>>,
    loop_enabled: Option<Arc<AtomicFlag>>,
    loop_start: Option<Arc<AtomicDouble>>,
    loop_end: Option<Arc<AtomicDouble>>,
    sample_rate: f64,
    beat_per_sample: f64,
    last_tempo: f32,
}

impl TransportClock {
    pub fn new(tempo: Arc<AtomicFloat>, paused: Arc<AtomicFlag>, sample_rate: f64) -> Self {
        let initial_tempo = tempo.get();
        let beat_per_sample = (initial_tempo as f64 / 60.0) / sample_rate;

        Self {
            current_beat: 0.0,
            tempo,
            paused,
            seek_target: Arc::new(AtomicDouble::new(0.0)),
            seek_pending: Arc::new(AtomicFlag::new(false)),
            position_writeback: None,
            loop_enabled: None,
            loop_start: None,
            loop_end: None,
            sample_rate,
            beat_per_sample,
            last_tempo: initial_tempo,
        }
    }

    pub fn with_seek(
        tempo: Arc<AtomicFloat>,
        paused: Arc<AtomicFlag>,
        seek_target: Arc<AtomicDouble>,
        seek_pending: Arc<AtomicFlag>,
        sample_rate: f64,
    ) -> Self {
        let initial_tempo = tempo.get();
        let beat_per_sample = (initial_tempo as f64 / 60.0) / sample_rate;

        Self {
            current_beat: 0.0,
            tempo,
            paused,
            seek_target,
            seek_pending,
            position_writeback: None,
            loop_enabled: None,
            loop_start: None,
            loop_end: None,
            sample_rate,
            beat_per_sample,
            last_tempo: initial_tempo,
        }
    }

    pub fn with_loop(
        tempo: Arc<AtomicFloat>,
        paused: Arc<AtomicFlag>,
        loop_enabled: Arc<AtomicFlag>,
        loop_start: Arc<AtomicDouble>,
        loop_end: Arc<AtomicDouble>,
        sample_rate: f64,
    ) -> Self {
        let initial_tempo = tempo.get();
        let beat_per_sample = (initial_tempo as f64 / 60.0) / sample_rate;

        Self {
            current_beat: 0.0,
            tempo,
            paused,
            seek_target: Arc::new(AtomicDouble::new(0.0)),
            seek_pending: Arc::new(AtomicFlag::new(false)),
            position_writeback: None,
            loop_enabled: Some(loop_enabled),
            loop_start: Some(loop_start),
            loop_end: Some(loop_end),
            sample_rate,
            beat_per_sample,
            last_tempo: initial_tempo,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_full_control(
        tempo: Arc<AtomicFloat>,
        paused: Arc<AtomicFlag>,
        seek_target: Arc<AtomicDouble>,
        seek_pending: Arc<AtomicFlag>,
        loop_enabled: Arc<AtomicFlag>,
        loop_start: Arc<AtomicDouble>,
        loop_end: Arc<AtomicDouble>,
        position_writeback: Arc<AtomicDouble>,
        sample_rate: f64,
    ) -> Self {
        let initial_tempo = tempo.get();
        let beat_per_sample = (initial_tempo as f64 / 60.0) / sample_rate;

        Self {
            current_beat: 0.0,
            tempo,
            paused,
            seek_target,
            seek_pending,
            position_writeback: Some(position_writeback),
            loop_enabled: Some(loop_enabled),
            loop_start: Some(loop_start),
            loop_end: Some(loop_end),
            sample_rate,
            beat_per_sample,
            last_tempo: initial_tempo,
        }
    }

    pub fn with_position_writeback(mut self, writeback: Arc<AtomicDouble>) -> Self {
        self.position_writeback = Some(writeback);
        self
    }

    pub fn position_writeback(&mut self) -> Arc<AtomicDouble> {
        Arc::clone(
            self.position_writeback
                .get_or_insert_with(|| Arc::new(AtomicDouble::new(0.0))),
        )
    }

    pub fn seek_target(&self) -> Arc<AtomicDouble> {
        Arc::clone(&self.seek_target)
    }

    pub fn seek_pending(&self) -> Arc<AtomicFlag> {
        Arc::clone(&self.seek_pending)
    }

    pub fn seek(&self, beat: f64) {
        self.seek_target.set(beat);
        self.seek_pending.set(true);
    }

    pub fn current_beat(&self) -> f64 {
        self.current_beat
    }

    #[inline]
    fn update_tempo_if_changed(&mut self) {
        let current_tempo = self.tempo.get();
        if (current_tempo - self.last_tempo).abs() > 0.001 {
            self.beat_per_sample = (current_tempo as f64 / 60.0) / self.sample_rate;
            self.last_tempo = current_tempo;
        }
    }

    #[inline]
    fn apply_pending_seek(&mut self) {
        if self.seek_pending.get() {
            self.current_beat = self.seek_target.get();
            self.seek_pending.set(false);
        }
    }

    #[inline]
    fn apply_loop_wrap(&mut self) {
        let loop_enabled = self.loop_enabled.as_ref().map(|l| l.get()).unwrap_or(false);
        if !loop_enabled {
            return;
        }

        let loop_start = self.loop_start.as_ref().map(|l| l.get()).unwrap_or(0.0);
        let loop_end = self.loop_end.as_ref().map(|l| l.get()).unwrap_or(f64::MAX);

        if loop_end > loop_start && self.current_beat >= loop_end {
            // Wrap precisely: preserve the overshoot
            let overshoot = self.current_beat - loop_end;
            let loop_length = loop_end - loop_start;
            self.current_beat = loop_start + (overshoot % loop_length);
        }
    }

    pub fn loop_atomics(&self) -> Option<(Arc<AtomicFlag>, Arc<AtomicDouble>, Arc<AtomicDouble>)> {
        match (&self.loop_enabled, &self.loop_start, &self.loop_end) {
            (Some(enabled), Some(start), Some(end)) => {
                Some((Arc::clone(enabled), Arc::clone(start), Arc::clone(end)))
            }
            _ => None,
        }
    }

    pub fn set_loop_points(&self, start: f64, end: f64) {
        if let Some(ref loop_start) = self.loop_start {
            loop_start.set(start);
        }
        if let Some(ref loop_end) = self.loop_end {
            loop_end.set(end);
        }
    }

    pub fn set_loop_enabled(&self, enabled: bool) {
        if let Some(ref loop_enabled) = self.loop_enabled {
            loop_enabled.set(enabled);
        }
    }
}

impl AudioUnit for TransportClock {
    fn inputs(&self) -> usize {
        0 // Generator - no inputs
    }

    fn outputs(&self) -> usize {
        1 // Single output: beat position
    }

    fn reset(&mut self) {
        self.current_beat = 0.0;
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
        self.beat_per_sample = (self.tempo.get() as f64 / 60.0) / sample_rate;
    }

    #[inline]
    fn tick(&mut self, _input: &[f32], output: &mut [f32]) {
        // Apply pending seek
        self.apply_pending_seek();

        // Update tempo if changed
        self.update_tempo_if_changed();

        // Output current beat
        output[0] = self.current_beat as f32;

        // Advance if not paused
        if !self.paused.get() {
            self.current_beat += self.beat_per_sample;

            // Sample-accurate loop wrapping
            self.apply_loop_wrap();
        }
    }

    fn process(&mut self, size: usize, _input: &BufferRef, output: &mut BufferMut) {
        // Apply pending seek
        self.apply_pending_seek();

        // Update tempo if changed
        self.update_tempo_if_changed();

        let is_paused = self.paused.get();

        // Read loop state once per buffer (loop params don't change mid-buffer)
        let loop_enabled = self.loop_enabled.as_ref().map(|l| l.get()).unwrap_or(false);
        let loop_start = self.loop_start.as_ref().map(|l| l.get()).unwrap_or(0.0);
        let loop_end = self.loop_end.as_ref().map(|l| l.get()).unwrap_or(f64::MAX);

        if is_paused {
            // Output constant beat when paused
            let beat = self.current_beat as f32;
            for i in 0..size {
                output.set_f32(0, i, beat);
            }
        } else if loop_enabled && loop_end > loop_start {
            // Output advancing beat with sample-accurate loop wrapping
            for i in 0..size {
                output.set_f32(0, i, self.current_beat as f32);
                self.current_beat += self.beat_per_sample;

                // Sample-accurate loop wrap
                if self.current_beat >= loop_end {
                    // Wrap precisely: preserve the overshoot
                    let overshoot = self.current_beat - loop_end;
                    let loop_length = loop_end - loop_start;
                    self.current_beat = loop_start + (overshoot % loop_length);
                }
            }
        } else {
            // Output advancing beat (no looping)
            for i in 0..size {
                output.set_f32(0, i, self.current_beat as f32);
                self.current_beat += self.beat_per_sample;
            }
        }

        // Write back position for UI sync (once per buffer, not per sample)
        if let Some(ref writeback) = self.position_writeback {
            writeback.set(self.current_beat);
        }
    }

    fn get_id(&self) -> u64 {
        const TRANSPORT_CLOCK_ID: u64 = 0x_5452_4E53_434C_4B00; // "TRNSCLK\0"
        TRANSPORT_CLOCK_ID
    }

    fn as_any(&self) -> &dyn any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn any::Any {
        self
    }

    fn route(&mut self, _input: &SignalFrame, _frequency: f64) -> SignalFrame {
        // Transport clock outputs a value signal (beat position)
        let mut output = SignalFrame::new(1);
        output.set(0, Signal::Value(self.current_beat));
        output
    }

    fn footprint(&self) -> usize {
        core::mem::size_of::<Self>()
    }
}

impl Clone for TransportClock {
    fn clone(&self) -> Self {
        Self {
            current_beat: self.current_beat,
            tempo: Arc::clone(&self.tempo),
            paused: Arc::clone(&self.paused),
            seek_target: Arc::clone(&self.seek_target),
            seek_pending: Arc::clone(&self.seek_pending),
            position_writeback: self.position_writeback.as_ref().map(Arc::clone),
            loop_enabled: self.loop_enabled.as_ref().map(Arc::clone),
            loop_start: self.loop_start.as_ref().map(Arc::clone),
            loop_end: self.loop_end.as_ref().map(Arc::clone),
            sample_rate: self.sample_rate,
            beat_per_sample: self.beat_per_sample,
            last_tempo: self.last_tempo,
        }
    }
}

/// Automation reader that takes beat position as INPUT (sample-accurate).
/// Unlike `AutomationReader` which reads from a `Shared` atomic (buffer-level accuracy),
/// this version takes the beat position as an input signal, enabling true sample-accurate
/// automation when connected to a `TransportClock`.
/// ## Inputs
/// - Port 0: Beat position (from TransportClock)
/// ## Outputs
/// - Port 0: Automation value (evaluated envelope)
pub struct AutomationReaderInput {
    envelope: AutomationEnvelopeFn,
    sample_rate: f64,
}

/// Automation envelope function type
pub type AutomationEnvelopeFn = Arc<dyn Fn(f64) -> f32 + Send + Sync>;

impl AutomationReaderInput {
    pub fn new(envelope: AutomationEnvelopeFn) -> Self {
        Self {
            envelope,
            sample_rate: DEFAULT_SR,
        }
    }

    pub fn from_envelope<T>(envelope: audio_automation::AutomationEnvelope<T>) -> Self
    where
        T: Clone + Send + Sync + 'static,
    {
        let envelope_arc = Arc::new(envelope);
        let envelope_fn: AutomationEnvelopeFn =
            Arc::new(move |beat: f64| envelope_arc.get_value_at(beat).unwrap_or(0.0));
        Self::new(envelope_fn)
    }

    pub fn from_envelope_arc<T>(envelope: Arc<audio_automation::AutomationEnvelope<T>>) -> Self
    where
        T: Clone + Send + Sync + 'static,
    {
        let envelope_fn: AutomationEnvelopeFn =
            Arc::new(move |beat: f64| envelope.get_value_at(beat).unwrap_or(0.0));
        Self::new(envelope_fn)
    }

    pub fn from_points(points: Vec<(f64, f32)>) -> Self {
        let envelope: AutomationEnvelopeFn = Arc::new(move |beat: f64| {
            if points.is_empty() {
                return 0.0;
            }

            // Find surrounding points
            let mut prev_idx = 0;
            let mut next_idx = 0;

            for (i, (b, _)) in points.iter().enumerate() {
                if *b <= beat {
                    prev_idx = i;
                }
                if *b >= beat && next_idx == 0 {
                    next_idx = i;
                    break;
                }
            }

            // Clamp to ends
            if beat <= points[0].0 {
                return points[0].1;
            }
            if beat >= points[points.len() - 1].0 || next_idx == 0 {
                return points[points.len() - 1].1;
            }

            // Linear interpolation
            let (b1, v1) = points[prev_idx];
            let (b2, v2) = points[next_idx];

            if (b2 - b1).abs() < 1e-10 {
                return v1;
            }

            let t = ((beat - b1) / (b2 - b1)) as f32;
            v1 + (v2 - v1) * t
        });

        Self::new(envelope)
    }

    pub fn from_points_with_curves(points: Vec<(f64, f32, audio_automation::CurveType)>) -> Self {
        let envelope: AutomationEnvelopeFn = Arc::new(move |beat: f64| {
            if points.is_empty() {
                return 0.0;
            }

            // Find surrounding points
            let mut prev_idx = 0;
            let mut next_idx = 0;

            for (i, (b, _, _)) in points.iter().enumerate() {
                if *b <= beat {
                    prev_idx = i;
                }
                if *b >= beat && next_idx == 0 {
                    next_idx = i;
                    break;
                }
            }

            // Clamp to ends
            if beat <= points[0].0 {
                return points[0].1;
            }
            if beat >= points[points.len() - 1].0 || next_idx == 0 {
                return points[points.len() - 1].1;
            }

            // Interpolate using curve type
            let (b1, v1, _) = points[prev_idx];
            let (b2, v2, curve) = points[next_idx];

            if (b2 - b1).abs() < 1e-10 {
                return v1;
            }

            let t = ((beat - b1) / (b2 - b1)) as f32;
            curve.interpolate(v1, v2, t)
        });

        Self::new(envelope)
    }
}

impl AudioUnit for AutomationReaderInput {
    fn inputs(&self) -> usize {
        1 // Beat position input
    }

    fn outputs(&self) -> usize {
        1 // Automation value output
    }

    fn reset(&mut self) {
        // Nothing to reset
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
    }

    #[inline]
    fn tick(&mut self, input: &[f32], output: &mut [f32]) {
        let beat = input[0] as f64;
        output[0] = (self.envelope)(beat);
    }

    fn process(&mut self, size: usize, input: &BufferRef, output: &mut BufferMut) {
        // Sample-accurate: evaluate envelope for each sample's beat position
        for i in 0..size {
            let beat = input.at_f32(0, i) as f64;
            let value = (self.envelope)(beat);
            output.set_f32(0, i, value);
        }
    }

    fn get_id(&self) -> u64 {
        const AUTOMATION_INPUT_ID: u64 = 0x_4155_544F_494E_5054; // "AUTOINPT"
        AUTOMATION_INPUT_ID
    }

    fn as_any(&self) -> &dyn any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn any::Any {
        self
    }

    fn route(&mut self, input: &SignalFrame, _frequency: f64) -> SignalFrame {
        // Automation outputs a value signal based on input beat position
        let mut output = SignalFrame::new(1);
        if let Signal::Value(beat) = input.at(0) {
            output.set(0, Signal::Value((self.envelope)(beat) as f64));
        } else {
            output.set(0, Signal::Value(0.0));
        }
        output
    }

    fn footprint(&self) -> usize {
        core::mem::size_of::<Self>()
    }
}

impl Clone for AutomationReaderInput {
    fn clone(&self) -> Self {
        Self {
            envelope: Arc::clone(&self.envelope),
            sample_rate: self.sample_rate,
        }
    }
}

/// Helper curves for creating common automation envelopes
pub mod curves {
    use super::*;

    pub fn linear_ramp(
        start_beat: f64,
        end_beat: f64,
        start_value: f32,
        end_value: f32,
    ) -> AutomationEnvelopeFn {
        Arc::new(move |beat: f64| {
            if beat <= start_beat {
                start_value
            } else if beat >= end_beat {
                end_value
            } else {
                let t = ((beat - start_beat) / (end_beat - start_beat)) as f32;
                start_value + (end_value - start_value) * t
            }
        })
    }

    pub fn exponential_ramp(
        start_beat: f64,
        end_beat: f64,
        start_value: f32,
        end_value: f32,
    ) -> AutomationEnvelopeFn {
        let start_value = start_value.max(0.001);
        let end_value = end_value.max(0.001);

        Arc::new(move |beat: f64| {
            if beat <= start_beat {
                start_value
            } else if beat >= end_beat {
                end_value
            } else {
                let t = ((beat - start_beat) / (end_beat - start_beat)) as f32;
                start_value * (end_value / start_value).powf(t)
            }
        })
    }

    pub fn constant(value: f32) -> AutomationEnvelopeFn {
        Arc::new(move |_beat: f64| value)
    }

    pub fn steps(steps: Vec<(f64, f32)>) -> AutomationEnvelopeFn {
        Arc::new(move |beat: f64| {
            let mut value = 0.0;
            for (b, v) in &steps {
                if beat >= *b {
                    value = *v;
                } else {
                    break;
                }
            }
            value
        })
    }

    pub fn sine_lfo(
        frequency_beats: f64,
        min_value: f32,
        max_value: f32,
        phase_offset: f32,
    ) -> AutomationEnvelopeFn {
        let amplitude = (max_value - min_value) / 2.0;
        let center = (max_value + min_value) / 2.0;

        Arc::new(move |beat: f64| {
            let phase = core::f32::consts::TAU * (beat / frequency_beats) as f32 + phase_offset;
            center + amplitude * phase.sin()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_atomics() -> (Arc<AtomicFloat>, Arc<AtomicFlag>) {
        (
            Arc::new(AtomicFloat::new(120.0)), // 120 BPM
            Arc::new(AtomicFlag::new(false)),  // Not paused
        )
    }

    #[test]
    fn test_transport_clock_creation() {
        let (tempo, paused) = create_test_atomics();
        let clock = TransportClock::new(tempo, paused, 44100.0);

        assert_eq!(clock.inputs(), 0);
        assert_eq!(clock.outputs(), 1);
        assert!((clock.current_beat() - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_transport_clock_tick() {
        let (tempo, paused) = create_test_atomics();
        let mut clock = TransportClock::new(tempo, paused, 44100.0);

        let mut output = [0.0f32];

        // First tick should output 0
        clock.tick(&[], &mut output);
        assert!((output[0] - 0.0).abs() < 0.001);

        // After many ticks, beat should advance
        for _ in 0..44100 {
            clock.tick(&[], &mut output);
        }

        // At 120 BPM, 44100 samples = 1 second = 2 beats
        assert!((output[0] - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_transport_clock_pause() {
        let (tempo, paused) = create_test_atomics();
        let mut clock = TransportClock::new(tempo, paused.clone(), 44100.0);

        let mut output = [0.0f32];

        // Advance a bit
        for _ in 0..1000 {
            clock.tick(&[], &mut output);
        }
        let beat_before_pause = output[0];

        // Pause
        paused.set(true);

        // Tick more
        for _ in 0..1000 {
            clock.tick(&[], &mut output);
        }

        // Beat should not have changed
        assert!((output[0] - beat_before_pause).abs() < 0.001);
    }

    #[test]
    fn test_transport_clock_seek() {
        let (tempo, paused) = create_test_atomics();
        let clock = TransportClock::new(tempo, paused, 44100.0);

        // Seek to beat 4
        clock.seek(4.0);

        let mut clock = clock; // Make mutable for tick
        let mut output = [0.0f32];
        clock.tick(&[], &mut output);

        assert!((output[0] - 4.0).abs() < 0.001);
    }

    #[test]
    fn test_transport_clock_tempo_change() {
        let (tempo, paused) = create_test_atomics();
        let mut clock = TransportClock::new(tempo.clone(), paused, 44100.0);

        let mut output = [0.0f32];

        // Tick at 120 BPM
        for _ in 0..44100 {
            clock.tick(&[], &mut output);
        }

        // Change to 240 BPM (double speed)
        tempo.set(240.0);

        // Tick another second
        for _ in 0..44100 {
            clock.tick(&[], &mut output);
        }

        // Should have advanced 4 beats in total (2 + 4 = 6 with tempo change mid-way)
        // Actually: 2 beats at 120 BPM + 4 beats at 240 BPM = 6 beats
        assert!((output[0] - 6.0).abs() < 0.1);
    }

    #[test]
    fn test_automation_reader_input_creation() {
        let envelope: AutomationEnvelopeFn = Arc::new(|beat| beat as f32);
        let auto = AutomationReaderInput::new(envelope);

        assert_eq!(auto.inputs(), 1);
        assert_eq!(auto.outputs(), 1);
    }

    #[test]
    fn test_automation_reader_input_tick() {
        let envelope: AutomationEnvelopeFn = Arc::new(|beat| (beat * 0.5) as f32);
        let mut auto = AutomationReaderInput::new(envelope);

        let input = [2.0f32]; // Beat position = 2.0
        let mut output = [0.0f32];

        auto.tick(&input, &mut output);

        // 2.0 * 0.5 = 1.0
        assert!((output[0] - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_automation_reader_input_from_points() {
        let points = vec![(0.0, 0.0), (1.0, 1.0), (2.0, 0.5)];
        let mut auto = AutomationReaderInput::from_points(points);

        let input = [0.5f32];
        let mut output = [0.0f32];

        auto.tick(&input, &mut output);

        // Linear interpolation at 0.5 between (0,0) and (1,1) = 0.5
        assert!((output[0] - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_curves_linear_ramp() {
        let envelope = curves::linear_ramp(1.0, 3.0, 0.0, 1.0);

        assert!((envelope(0.0) - 0.0).abs() < 0.001); // Before start
        assert!((envelope(1.0) - 0.0).abs() < 0.001); // At start
        assert!((envelope(2.0) - 0.5).abs() < 0.001); // Middle
        assert!((envelope(3.0) - 1.0).abs() < 0.001); // At end
        assert!((envelope(4.0) - 1.0).abs() < 0.001); // After end
    }

    #[test]
    fn test_curves_constant() {
        let envelope = curves::constant(0.75);

        assert!((envelope(0.0) - 0.75).abs() < 0.001);
        assert!((envelope(100.0) - 0.75).abs() < 0.001);
    }

    #[test]
    fn test_sample_accurate_automation() {
        // Integration test: clock + automation reader
        let (tempo, paused) = create_test_atomics();
        let mut clock = TransportClock::new(tempo, paused, 44100.0);

        // Automation: ramp from 0 to 1 over 2 beats
        let envelope = curves::linear_ramp(0.0, 2.0, 0.0, 1.0);
        let mut auto = AutomationReaderInput::new(envelope);

        // Process several samples and verify sample-accurate automation
        let mut values = Vec::new();
        for _ in 0..1000 {
            let mut clock_out = [0.0f32];
            clock.tick(&[], &mut clock_out);

            let mut auto_out = [0.0f32];
            auto.tick(&clock_out, &mut auto_out);

            values.push(auto_out[0]);
        }

        // Values should be monotonically increasing (since we're ramping up)
        for i in 1..values.len() {
            assert!(
                values[i] >= values[i - 1] - 0.0001,
                "Values should be monotonic: {} >= {}",
                values[i],
                values[i - 1]
            );
        }

        // Each sample should have a slightly different value (sample-accurate)
        let unique_values: std::collections::HashSet<u32> =
            values.iter().map(|v| (v * 10000.0) as u32).collect();
        assert!(
            unique_values.len() > 100,
            "Should have many unique values for sample accuracy"
        );
    }

    #[test]
    fn test_transport_clock_loop_wrapping() {
        let (tempo, paused) = create_test_atomics();
        let loop_enabled = Arc::new(AtomicFlag::new(true));
        let loop_start = Arc::new(AtomicDouble::new(0.0));
        let loop_end = Arc::new(AtomicDouble::new(4.0)); // 4 beat loop

        let mut clock =
            TransportClock::with_loop(tempo, paused, loop_enabled, loop_start, loop_end, 44100.0);

        let mut output = [0.0f32];

        // At 120 BPM, beat_per_sample = (120/60) / 44100 = 2 / 44100 ≈ 0.0000453
        // To reach beat 4 and wrap, we need 4 / 0.0000453 ≈ 88235 samples
        // Use slightly more to ensure we wrap
        for _ in 0..90000 {
            clock.tick(&[], &mut output);
        }

        // Should have wrapped back near the start (within the 4-beat loop)
        // 90000 samples at 120 BPM = ~4.08 beats, wraps to ~0.08
        assert!(
            output[0] < 0.5,
            "Expected beat near 0 after loop, got {}",
            output[0]
        );
        assert!(output[0] >= 0.0, "Beat should be >= 0 after wrap");
    }

    #[test]
    fn test_transport_clock_loop_disabled() {
        let (tempo, paused) = create_test_atomics();
        let loop_enabled = Arc::new(AtomicFlag::new(false)); // Disabled
        let loop_start = Arc::new(AtomicDouble::new(0.0));
        let loop_end = Arc::new(AtomicDouble::new(4.0));

        let mut clock =
            TransportClock::with_loop(tempo, paused, loop_enabled, loop_start, loop_end, 44100.0);

        let mut output = [0.0f32];

        // Run past the loop point - 90000 samples at 120 BPM = ~4.08 beats
        for _ in 0..90000 {
            clock.tick(&[], &mut output);
        }

        // Should NOT have wrapped (loop disabled), so beat > 4.0
        assert!(
            output[0] > 4.0,
            "Expected beat > 4.0 with loop disabled, got {}",
            output[0]
        );
    }

    #[test]
    fn test_transport_clock_loop_overshoot_precision() {
        // Test that overshoot is preserved when looping
        let (tempo, paused) = create_test_atomics();
        let loop_enabled = Arc::new(AtomicFlag::new(true));
        let loop_start = Arc::new(AtomicDouble::new(0.0));
        let loop_end = Arc::new(AtomicDouble::new(1.0)); // 1 beat loop for fast testing

        let mut clock =
            TransportClock::with_loop(tempo, paused, loop_enabled, loop_start, loop_end, 44100.0);

        // Set position just before loop end
        // At 120 BPM, beat_per_sample ≈ 0.0000453
        // Set to 0.9999, then after 1 tick we're at 0.9999 + 0.0000453 ≈ 0.99994
        // Still below 1.0, so need more ticks to actually wrap
        clock.seek(0.99999);
        let mut output = [0.0f32];
        clock.tick(&[], &mut output); // Apply seek, output ~0.99999, advance to ~1.00004, wrap to ~0.00004

        // After the first tick that triggers the wrap, the internal beat is wrapped
        // The next tick will output the wrapped value
        clock.tick(&[], &mut output);

        // The beat should have wrapped and be small but positive
        assert!(output[0] >= 0.0, "Beat should be >= 0 after wrap");
        assert!(
            output[0] < 0.01,
            "Beat should be near start after wrap: {}",
            output[0]
        );
    }
}
