//! Sample-accurate automation reader that evaluates envelopes from beat position input.

use crate::compat::{any, Arc, Vec};
use fundsp::prelude::*;

/// Automation envelope function type
pub type AutomationEnvelopeFn = Arc<dyn Fn(f64) -> f32 + Send + Sync>;

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

impl AutomationReaderInput {
    pub fn new(envelope: AutomationEnvelopeFn) -> Self {
        Self {
            envelope,
            sample_rate: DEFAULT_SR,
        }
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

            if (b2 - b1).abs() < 1e-10_f64 {
                return v1;
            }

            let t = ((beat - b1) / (b2 - b1)) as f32;
            v1 + (v2 - v1) * t
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfree::{AtomicFlag, AtomicFloat};
    use crate::transport::TransportClock;

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
    fn test_sample_accurate_automation() {
        // Integration test: clock + automation reader
        let tempo = Arc::new(AtomicFloat::new(120.0));
        let paused = Arc::new(AtomicFlag::new(false));
        let mut clock = TransportClock::new(tempo, paused, 44100.0);

        // Automation: ramp from 0 to 1 over 2 beats
        let envelope: AutomationEnvelopeFn =
            Arc::new(|beat: f64| (beat / 2.0).clamp(0.0, 1.0) as f32);
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
}
