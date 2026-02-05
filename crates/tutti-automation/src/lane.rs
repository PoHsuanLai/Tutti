//! AutomationLane AudioUnit node.

use audio_automation::AutomationEnvelope;
use tutti_core::{AudioUnit, BufferMut, BufferRef, SignalFrame, TransportHandle};

/// An automation lane that outputs control signals based on transport position.
///
/// Reads the current beat position from the transport and evaluates
/// the envelope to produce a control signal output.
///
/// # Example
///
/// ```ignore
/// use tutti_automation::{AutomationLane, AutomationEnvelope, AutomationPoint};
///
/// let mut envelope = AutomationEnvelope::new("volume");
/// envelope.add_point(AutomationPoint::new(0.0, 0.0));
/// envelope.add_point(AutomationPoint::new(4.0, 1.0));
///
/// let lane = AutomationLane::new(envelope, transport_handle);
/// ```
pub struct AutomationLane<T> {
    envelope: AutomationEnvelope<T>,
    transport: TransportHandle,
    last_value: f32,
}

impl<T> AutomationLane<T> {
    /// Create a new automation lane.
    pub fn new(envelope: AutomationEnvelope<T>, transport: TransportHandle) -> Self {
        Self {
            envelope,
            transport,
            last_value: 0.0,
        }
    }

    /// Replace the envelope.
    pub fn set_envelope(&mut self, envelope: AutomationEnvelope<T>) {
        self.envelope = envelope;
    }

    /// Get the envelope.
    pub fn envelope(&self) -> &AutomationEnvelope<T> {
        &self.envelope
    }

    /// Get mutable access to the envelope.
    pub fn envelope_mut(&mut self) -> &mut AutomationEnvelope<T> {
        &mut self.envelope
    }

    /// Get the last evaluated value.
    pub fn last_value(&self) -> f32 {
        self.last_value
    }

    /// Get value at a specific beat position.
    ///
    /// This is useful for querying automation values without using the transport.
    pub fn get_value_at(&self, beat: f64) -> f32 {
        self.envelope.get_value_at(beat).unwrap_or(0.0)
    }

    /// Get value accounting for transport loop.
    ///
    /// When the beat position is beyond the loop end, wraps it back into the loop range.
    /// This ensures automation repeats correctly during looped playback.
    ///
    /// # Arguments
    ///
    /// * `beat` - Current beat position
    /// * `loop_start` - Loop start in beats
    /// * `loop_end` - Loop end in beats (must be > loop_start)
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Loop from beat 4 to beat 8
    /// let value = lane.get_value_looped(10.0, 4.0, 8.0);
    /// // beat 10 wraps to beat 6 (10 - 4 = 6, which is within 4..8)
    /// ```
    pub fn get_value_looped(&self, beat: f64, loop_start: f64, loop_end: f64) -> f32 {
        let loop_len = loop_end - loop_start;

        if loop_len <= 0.0 {
            // Invalid loop range, use beat directly
            return self.envelope.get_value_at(beat).unwrap_or(0.0);
        }

        let effective_beat = if beat < loop_start {
            beat
        } else if beat < loop_end {
            beat
        } else {
            loop_start + ((beat - loop_start) % loop_len)
        };

        self.envelope.get_value_at(effective_beat).unwrap_or(0.0)
    }

    /// Update and return the current value using transport position.
    ///
    /// Automatically handles loop wrapping if loop is enabled on the transport.
    pub fn update(&mut self) -> f32 {
        let beat = self.transport.current_beat();

        self.last_value = if self.transport.is_loop_enabled() {
            if let Some((loop_start, loop_end)) = self.transport.get_loop_range() {
                self.get_value_looped(beat, loop_start, loop_end)
            } else {
                self.envelope.get_value_at(beat).unwrap_or(0.0)
            }
        } else {
            self.envelope.get_value_at(beat).unwrap_or(0.0)
        };

        self.last_value
    }
}

impl<T: Clone + Send + Sync + 'static> AudioUnit for AutomationLane<T> {
    fn inputs(&self) -> usize {
        0 // Generator - no audio input
    }

    fn outputs(&self) -> usize {
        1 // Single control signal output
    }

    fn tick(&mut self, _input: &[f32], output: &mut [f32]) {
        output[0] = self.update();
    }

    fn process(&mut self, size: usize, _input: &BufferRef, output: &mut BufferMut) {
        let value = self.update();

        for i in 0..size {
            output.set_f32(0, i, value);
        }
    }

    fn reset(&mut self) {
        self.last_value = 0.0;
    }

    fn set_sample_rate(&mut self, _sample_rate: f64) {
        // Could use for sub-block interpolation in the future
    }

    fn route(&mut self, _input: &SignalFrame, _frequency: f64) -> SignalFrame {
        SignalFrame::new(1)
    }

    fn get_id(&self) -> u64 {
        0x4155544F4D415445 // "AUTOMATE" in hex
    }

    fn footprint(&self) -> usize {
        core::mem::size_of::<Self>()
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

impl<T: Clone> Clone for AutomationLane<T> {
    fn clone(&self) -> Self {
        Self {
            envelope: self.envelope.clone(),
            transport: self.transport.clone(),
            last_value: self.last_value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio_automation::AutomationPoint;

    // Note: Full tests require a mock TransportHandle
    // These are basic unit tests for the envelope integration

    #[test]
    fn test_envelope_value() {
        let mut envelope: AutomationEnvelope<&str> = AutomationEnvelope::new("test");
        envelope.add_point(AutomationPoint::new(0.0, 0.0));
        envelope.add_point(AutomationPoint::new(4.0, 1.0));

        assert!((envelope.get_value_at(0.0).unwrap() - 0.0).abs() < 0.01);
        assert!((envelope.get_value_at(2.0).unwrap() - 0.5).abs() < 0.01);
        assert!((envelope.get_value_at(4.0).unwrap() - 1.0).abs() < 0.01);
    }

    /// Test helper to create envelope and test loop wrapping without transport
    fn create_test_envelope() -> AutomationEnvelope<&'static str> {
        let mut envelope: AutomationEnvelope<&str> = AutomationEnvelope::new("test");
        // Envelope: 0.0 at beat 0, 1.0 at beat 4, 0.5 at beat 8
        envelope.add_point(AutomationPoint::new(0.0, 0.0));
        envelope.add_point(AutomationPoint::new(4.0, 1.0));
        envelope.add_point(AutomationPoint::new(8.0, 0.5));
        envelope
    }

    #[test]
    fn test_loop_wrapping_basic() {
        let envelope = create_test_envelope();

        // Test the loop wrapping logic directly on envelope
        // Loop from beat 4 to beat 8 (4 beats long)
        let loop_start: f64 = 4.0;
        let loop_end: f64 = 8.0;
        let loop_len = loop_end - loop_start;

        // Beat 10 should wrap to beat 6 (10 - 4 = 6 % 4 = 2, then 4 + 2 = 6)
        let beat: f64 = 10.0;
        let wrapped = loop_start + ((beat - loop_start) % loop_len);
        assert!((wrapped - 6.0).abs() < 0.001, "Expected 6.0, got {}", wrapped);

        // At beat 6 (midpoint of loop), value should be interpolated
        let value = envelope.get_value_at(wrapped).unwrap();
        assert!(value > 0.5 && value < 1.0, "Expected value between 0.5-1.0 at beat 6, got {}", value);
    }

    #[test]
    fn test_loop_wrapping_at_boundaries() {
        let envelope = create_test_envelope();
        let loop_start: f64 = 4.0;
        let loop_end: f64 = 8.0;
        let loop_len = loop_end - loop_start;

        // Beat exactly at loop_end should wrap to loop_start
        let beat: f64 = 8.0;
        let wrapped = if beat < loop_end {
            beat
        } else {
            loop_start + ((beat - loop_start) % loop_len)
        };
        assert!((wrapped - 4.0).abs() < 0.001, "Expected 4.0, got {}", wrapped);

        // Value at beat 4 is 1.0
        let value = envelope.get_value_at(4.0).unwrap();
        assert!((value - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_loop_wrapping_multiple_iterations() {
        let _envelope = create_test_envelope();
        let loop_start: f64 = 4.0;
        let loop_end: f64 = 8.0;
        let loop_len = loop_end - loop_start;

        // Beat 20 should wrap multiple times: 20 - 4 = 16, 16 % 4 = 0, 4 + 0 = 4
        let beat: f64 = 20.0;
        let wrapped = loop_start + ((beat - loop_start) % loop_len);
        assert!((wrapped - 4.0).abs() < 0.001, "Expected 4.0, got {}", wrapped);
    }

    #[test]
    fn test_before_loop_start() {
        let envelope = create_test_envelope();

        // Before loop start, value should come directly from envelope
        let value = envelope.get_value_at(2.0).unwrap();
        assert!((value - 0.5).abs() < 0.01, "Expected 0.5 at beat 2, got {}", value);
    }
}
