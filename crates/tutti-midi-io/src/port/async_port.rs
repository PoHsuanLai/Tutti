//! Lock-free MIDI port using SPSC ring buffers.
//!
//! - Input: midir callback (producer) -> audio thread (consumer)
//! - Output: audio thread (producer) -> output thread (consumer)

use ringbuf::{
    traits::{Consumer, Producer, Split},
    HeapRb,
};
use std::cell::UnsafeCell;
use std::time::Instant;

pub use crate::MidiEvent;

/// # Safety
/// Must only be used from a single thread (the midir callback thread).
/// SPSC ring buffers require exactly one producer -- concurrent pushes are UB.
pub struct InputProducerHandle {
    producer: *mut ringbuf::HeapProd<(Instant, MidiEvent)>,
}

// SAFETY: HeapProd is Send. Ownership is transferred to the midir callback thread.
unsafe impl Send for InputProducerHandle {}

// SAFETY: Only one thread calls push() (SPSC invariant).
unsafe impl Sync for InputProducerHandle {}

impl InputProducerHandle {
    #[inline]
    pub fn push(&self, event: MidiEvent, timestamp: Instant) -> bool {
        // SAFETY: Exclusive access as single producer (SPSC invariant).
        let prod = unsafe { &mut *self.producer };
        prod.try_push((timestamp, event)).is_ok()
    }
}

/// # Safety
/// Must only be used from a single thread (SPSC invariant).
#[cfg(feature = "midi2")]
pub struct UnifiedInputProducerHandle {
    producer: *mut ringbuf::HeapProd<crate::event::UnifiedMidiEvent>,
}

#[cfg(feature = "midi2")]
// SAFETY: HeapProd is Send.
unsafe impl Send for UnifiedInputProducerHandle {}

#[cfg(feature = "midi2")]
// SAFETY: Only one thread calls push() (SPSC invariant).
unsafe impl Sync for UnifiedInputProducerHandle {}

#[cfg(feature = "midi2")]
impl UnifiedInputProducerHandle {
    #[inline]
    pub fn push(&self, event: crate::event::UnifiedMidiEvent) -> bool {
        // SAFETY: Exclusive access as single producer (SPSC invariant).
        let prod = unsafe { &mut *self.producer };
        prod.try_push(event).is_ok()
    }
}

/// # Safety
/// Must only be used from a single thread (the audio thread).
/// Clone copies the pointer; only one thread may call push() (SPSC invariant).
#[derive(Clone)]
pub struct OutputProducerHandle {
    producer: *mut ringbuf::HeapProd<MidiEvent>,
}

// SAFETY: HeapProd is Send. Ownership is transferred to the audio thread.
unsafe impl Send for OutputProducerHandle {}

// SAFETY: Only one thread calls push() (SPSC invariant).
unsafe impl Sync for OutputProducerHandle {}

impl OutputProducerHandle {
    #[inline]
    pub fn push(&self, event: MidiEvent) -> bool {
        // SAFETY: Exclusive access as single producer (SPSC invariant).
        let prod = unsafe { &mut *self.producer };
        prod.try_push(event).is_ok()
    }
}

pub struct AsyncMidiPort {
    name: String,
    active: std::sync::atomic::AtomicBool,
    input_consumer: UnsafeCell<ringbuf::HeapCons<(Instant, MidiEvent)>>,
    input_producer: UnsafeCell<ringbuf::HeapProd<(Instant, MidiEvent)>>,
    output_producer: UnsafeCell<ringbuf::HeapProd<MidiEvent>>,
    output_consumer: UnsafeCell<ringbuf::HeapCons<MidiEvent>>,
    #[cfg(feature = "midi2")]
    unified_input_consumer: UnsafeCell<ringbuf::HeapCons<crate::event::UnifiedMidiEvent>>,
    #[cfg(feature = "midi2")]
    unified_input_producer: UnsafeCell<ringbuf::HeapProd<crate::event::UnifiedMidiEvent>>,
}

impl AsyncMidiPort {
    pub fn new(name: impl Into<String>, fifo_size: usize) -> Self {
        let name = name.into();
        let input_rb = HeapRb::<(Instant, MidiEvent)>::new(fifo_size);
        let (input_producer, input_consumer) = input_rb.split();
        let output_rb = HeapRb::<MidiEvent>::new(fifo_size);
        let (output_producer, output_consumer) = output_rb.split();

        #[cfg(feature = "midi2")]
        let unified_rb = HeapRb::<crate::event::UnifiedMidiEvent>::new(fifo_size);
        #[cfg(feature = "midi2")]
        let (unified_input_producer, unified_input_consumer) = unified_rb.split();

        Self {
            name,
            active: std::sync::atomic::AtomicBool::new(true),
            input_consumer: UnsafeCell::new(input_consumer),
            input_producer: UnsafeCell::new(input_producer),
            output_producer: UnsafeCell::new(output_producer),
            output_consumer: UnsafeCell::new(output_consumer),
            #[cfg(feature = "midi2")]
            unified_input_consumer: UnsafeCell::new(unified_input_consumer),
            #[cfg(feature = "midi2")]
            unified_input_producer: UnsafeCell::new(unified_input_producer),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.active.load(std::sync::atomic::Ordering::Acquire)
    }

    #[inline]
    pub fn set_active(&self, active: bool) {
        self.active
            .store(active, std::sync::atomic::Ordering::Release);
    }

    pub fn input_producer_handle(&self) -> InputProducerHandle {
        InputProducerHandle {
            producer: self.input_producer.get(),
        }
    }

    #[cfg(feature = "midi2")]
    pub fn unified_input_producer_handle(&self) -> UnifiedInputProducerHandle {
        UnifiedInputProducerHandle {
            producer: self.unified_input_producer.get(),
        }
    }

    pub fn output_producer_handle(&self) -> OutputProducerHandle {
        OutputProducerHandle {
            producer: self.output_producer.get(),
        }
    }

    #[inline]
    pub fn cycle_start_read_input_into(
        &self,
        buf: &mut Vec<(Instant, usize, MidiEvent)>,
        port_index: usize,
    ) {
        let consumer = unsafe { &mut *self.input_consumer.get() };
        while let Some((timestamp, event)) = consumer.try_pop() {
            buf.push((timestamp, port_index, event));
        }
    }

    #[inline]
    #[cfg(feature = "midi2")]
    pub fn cycle_start_read_unified_input_into(
        &self,
        buf: &mut Vec<(usize, crate::event::UnifiedMidiEvent)>,
        port_index: usize,
    ) {
        let consumer = unsafe { &mut *self.unified_input_consumer.get() };
        while let Some(event) = consumer.try_pop() {
            buf.push((port_index, event));
        }
    }

    #[inline]
    pub fn cycle_end_flush_output_into(
        &self,
        buf: &mut Vec<(usize, MidiEvent)>,
        port_index: usize,
    ) {
        let consumer = unsafe { &mut *self.output_consumer.get() };
        while let Some(event) = consumer.try_pop() {
            buf.push((port_index, event));
        }
    }
}

// SAFETY: All UnsafeCell fields follow SPSC invariants -- each ring buffer
// has exactly one producer and one consumer, never accessed concurrently.
unsafe impl Sync for AsyncMidiPort {}

impl std::fmt::Debug for AsyncMidiPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncMidiPort")
            .field("name", &self.name)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper: drain input into a fresh Vec.
    fn read_input(port: &AsyncMidiPort) -> Vec<MidiEvent> {
        let mut buf = Vec::new();
        port.cycle_start_read_input_into(&mut buf, 0);
        buf.into_iter().map(|(_, _, e)| e).collect()
    }

    /// Test helper: drain output into a fresh Vec.
    fn flush_output(port: &AsyncMidiPort) -> Vec<MidiEvent> {
        let mut buf = Vec::new();
        port.cycle_end_flush_output_into(&mut buf, 0);
        buf.into_iter().map(|(_, e)| e).collect()
    }

    #[test]
    fn test_input_flow() {
        let port = AsyncMidiPort::new("Input", 256);
        let producer_handle = port.input_producer_handle();

        let event = MidiEvent::note_on(0, 0, 0x3C, 0x7F);
        assert!(producer_handle.push(event, Instant::now()));

        let events = read_input(&port);
        assert_eq!(events.len(), 1);
        assert!(events[0].is_note_on());
        assert_eq!(events[0].note(), Some(0x3C));
        assert_eq!(events[0].velocity(), Some(0x7F));
    }

    #[test]
    fn test_output_flow() {
        let port = AsyncMidiPort::new("Output", 256);
        let output_handle = port.output_producer_handle();

        let event = MidiEvent::note_off(10, 0, 0x3C, 0);
        assert!(output_handle.push(event));

        let events = flush_output(&port);
        assert_eq!(events.len(), 1);
        assert!(events[0].is_note_off());
        assert_eq!(events[0].note(), Some(0x3C));
    }

    #[test]
    fn test_fifo_full() {
        let port = AsyncMidiPort::new("Full", 4);
        let output_handle = port.output_producer_handle();

        for i in 0..4 {
            let event = MidiEvent::note_on(i, 0, 0x3C, 0x7F);
            assert!(output_handle.push(event), "Failed to write event {}", i);
        }

        let event = MidiEvent::note_on(5, 0, 0x3C, 0x7F);
        assert!(!output_handle.push(event), "FIFO should be full");
    }

    #[test]
    fn test_active_flag_toggle() {
        let port = AsyncMidiPort::new("ActiveTest", 256);
        assert!(port.is_active());

        port.set_active(false);
        assert!(!port.is_active());

        port.set_active(true);
        assert!(port.is_active());
    }

    #[test]
    fn test_input_output_isolation() {
        let port = AsyncMidiPort::new("Isolation", 256);
        let input_handle = port.input_producer_handle();
        let output_handle = port.output_producer_handle();

        // Push to input only
        let event = MidiEvent::note_on(0, 0, 60, 100);
        input_handle.push(event, Instant::now());

        // Output should be empty
        assert!(
            flush_output(&port).is_empty(),
            "Output should not receive input events"
        );

        // Input should have the event
        assert_eq!(read_input(&port).len(), 1);

        // Push to output only
        let event = MidiEvent::note_off(0, 0, 60, 0);
        output_handle.push(event);

        // Input should be empty
        assert!(
            read_input(&port).is_empty(),
            "Input should not receive output events"
        );

        // Output should have the event
        assert_eq!(flush_output(&port).len(), 1);
    }
}
