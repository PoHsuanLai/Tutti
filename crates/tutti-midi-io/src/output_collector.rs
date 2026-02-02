//! Lock-free MIDI output collection from audio nodes.

use crate::event::MidiEvent;
use parking_lot::Mutex;
use ringbuf::{traits::*, HeapCons, HeapProd, HeapRb};

/// Default capacity for the MIDI output ring buffer
const DEFAULT_CAPACITY: usize = 256;

/// Producer for pushing MIDI events from audio thread.
pub struct MidiOutputProducer {
    producer: HeapProd<MidiEvent>,
}

impl MidiOutputProducer {
    /// Push a MIDI event to the output buffer
    ///
    /// Returns true if the event was successfully pushed, false if buffer is full.
    #[inline]
    pub fn push(&mut self, event: MidiEvent) -> bool {
        self.producer.try_push(event).is_ok()
    }

    /// Push multiple MIDI events
    #[inline]
    pub fn push_slice(&mut self, events: &[MidiEvent]) -> usize {
        self.producer.push_slice(events)
    }
}

/// Consumer for draining MIDI events from output thread.
pub struct MidiOutputConsumer {
    consumer: HeapCons<MidiEvent>,
}

impl MidiOutputConsumer {
    /// Pop a single event
    #[inline]
    pub fn pop(&mut self) -> Option<MidiEvent> {
        self.consumer.try_pop()
    }

    /// Drain all pending events into a vector
    pub fn drain_all(&mut self) -> Vec<MidiEvent> {
        let count = self.consumer.occupied_len();
        let mut events = Vec::with_capacity(count);
        while let Some(event) = self.consumer.try_pop() {
            events.push(event);
        }
        events
    }

    /// Check if there are pending events
    #[inline]
    pub fn has_pending(&self) -> bool {
        !self.consumer.is_empty()
    }

    /// Get number of pending events
    #[inline]
    pub fn pending_count(&self) -> usize {
        self.consumer.occupied_len()
    }
}

/// Create a new MIDI output channel
pub fn midi_output_channel() -> (MidiOutputProducer, MidiOutputConsumer) {
    midi_output_channel_with_capacity(DEFAULT_CAPACITY)
}

/// Create a new MIDI output channel with specified capacity
pub fn midi_output_channel_with_capacity(
    capacity: usize,
) -> (MidiOutputProducer, MidiOutputConsumer) {
    let rb = HeapRb::new(capacity);
    let (producer, consumer) = rb.split();
    (
        MidiOutputProducer { producer },
        MidiOutputConsumer { consumer },
    )
}

/// Aggregates multiple MIDI output consumers.
pub struct MidiOutputAggregator {
    consumers: Mutex<Vec<MidiOutputConsumer>>,
}

impl MidiOutputAggregator {
    /// Create a new empty aggregator
    pub fn new() -> Self {
        Self {
            consumers: Mutex::new(Vec::new()),
        }
    }

    /// Add a consumer to the aggregator
    pub fn add_consumer(&self, consumer: MidiOutputConsumer) {
        self.consumers.lock().push(consumer);
    }

    /// Drain all events from all consumers
    pub fn drain_all(&self) -> Vec<MidiEvent> {
        let mut all_events = Vec::new();
        let mut consumers = self.consumers.lock();
        for consumer in consumers.iter_mut() {
            all_events.extend(consumer.drain_all());
        }
        all_events
    }

    /// Check if any consumer has pending events
    pub fn has_pending(&self) -> bool {
        let consumers = self.consumers.lock();
        consumers.iter().any(|c| c.has_pending())
    }
}

impl Default for MidiOutputAggregator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::super::event::MidiEvent;
    use super::*;
    use midi_msg::{Channel, ChannelVoiceMsg};

    #[test]
    fn test_channel_push_and_drain() {
        let (mut producer, mut consumer) = midi_output_channel();

        // Push some events
        let event1 = MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::NoteOn {
                note: 60,
                velocity: 100,
            },
        };
        let event2 = MidiEvent {
            frame_offset: 128,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::NoteOff {
                note: 60,
                velocity: 0,
            },
        };

        assert!(producer.push(event1));
        assert!(producer.push(event2));

        // Drain and verify
        let events = consumer.drain_all();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].note(), Some(60));
        assert_eq!(events[1].note(), Some(60));
    }

    #[test]
    fn test_aggregator() {
        let aggregator = MidiOutputAggregator::new();

        // Create two channels
        let (mut prod1, cons1) = midi_output_channel();
        let (mut prod2, cons2) = midi_output_channel();

        aggregator.add_consumer(cons1);
        aggregator.add_consumer(cons2);

        // Push to both
        prod1.push(MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::NoteOn {
                note: 60,
                velocity: 100,
            },
        });
        prod2.push(MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch2,
            msg: ChannelVoiceMsg::NoteOn {
                note: 72,
                velocity: 80,
            },
        });

        // Drain all
        let events = aggregator.drain_all();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_capacity_overflow() {
        let (mut producer, _consumer) = midi_output_channel_with_capacity(4);

        let event = MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::NoteOn {
                note: 60,
                velocity: 100,
            },
        };

        // Fill buffer
        assert!(producer.push(event));
        assert!(producer.push(event));
        assert!(producer.push(event));
        assert!(producer.push(event));

        // Should fail when full
        assert!(!producer.push(event));
    }
}
