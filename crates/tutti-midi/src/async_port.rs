//! Async MIDI Port - Lock-free MIDI I/O using ring buffers
//!
//! Fully lock-free SPSC pattern:
//! - Input: midir callback (producer) → audio thread (consumer)
//! - Output: audio thread (producer) → output thread (consumer)
//!
//! Both sides use UnsafeCell for zero-overhead access. Safety is guaranteed
//! by the SPSC (Single Producer Single Consumer) invariant.

use ringbuf::{
    traits::{Consumer, Producer, Split},
    HeapRb,
};
use std::cell::UnsafeCell;

pub use crate::MidiEvent;

/// Lock-free producer handle for MIDI input (used by midir callback).
///
/// # Safety
/// This handle must only be used from a single thread (the midir callback thread).
/// SPSC ring buffers require exactly one producer - concurrent pushes are undefined behavior.
pub struct InputProducerHandle {
    producer: *mut ringbuf::HeapProd<MidiEvent>,
}

// SAFETY: InputProducerHandle is Send because the underlying HeapProd is Send.
// It's used to transfer ownership from the port creator to the midir callback thread.
unsafe impl Send for InputProducerHandle {}

// SAFETY: InputProducerHandle is Sync because we document that only one thread
// may call push() at a time (the midir callback thread). This is the SPSC invariant.
unsafe impl Sync for InputProducerHandle {}

impl InputProducerHandle {
    /// Push a MIDI event to the input buffer (lock-free).
    ///
    /// # Safety
    /// Must only be called from a single thread (the midir callback thread).
    #[inline]
    pub fn push(&self, event: MidiEvent) -> bool {
        // SAFETY: We have exclusive access as the single producer (SPSC invariant).
        // The midir callback is the only caller of this method.
        let prod = unsafe { &mut *self.producer };
        prod.try_push(event).is_ok()
    }
}

/// Lock-free producer handle for unified MIDI input (MIDI 1.0 or 2.0).
///
/// # Safety
/// This handle must only be used from a single thread.
/// SPSC ring buffers require exactly one producer - concurrent pushes are undefined behavior.
#[cfg(feature = "midi2")]
pub struct UnifiedInputProducerHandle {
    producer: *mut ringbuf::HeapProd<crate::event::UnifiedMidiEvent>,
}

#[cfg(feature = "midi2")]
// SAFETY: UnifiedInputProducerHandle is Send because the underlying HeapProd is Send.
// It's used to transfer ownership from the port creator to the caller's thread.
unsafe impl Send for UnifiedInputProducerHandle {}

#[cfg(feature = "midi2")]
// SAFETY: UnifiedInputProducerHandle is Sync because we document that only one thread
// may call push() at a time. This is the SPSC invariant.
unsafe impl Sync for UnifiedInputProducerHandle {}

#[cfg(feature = "midi2")]
impl UnifiedInputProducerHandle {
    /// Push a unified MIDI event to the input buffer (lock-free).
    ///
    /// # Safety
    /// Must only be called from a single thread (SPSC invariant).
    #[inline]
    pub fn push(&self, event: crate::event::UnifiedMidiEvent) -> bool {
        // SAFETY: We have exclusive access as the single producer (SPSC invariant).
        let prod = unsafe { &mut *self.producer };
        prod.try_push(event).is_ok()
    }
}

/// Lock-free MIDI port using SPSC ring buffers.
///
/// Uses UnsafeCell for zero-overhead access on both producer and consumer sides.
/// Safety is guaranteed by the SPSC invariant: one producer, one consumer per buffer.
pub struct AsyncMidiPort {
    name: String,
    active: std::sync::atomic::AtomicBool,
    input_consumer: UnsafeCell<ringbuf::HeapCons<MidiEvent>>,
    input_producer: UnsafeCell<ringbuf::HeapProd<MidiEvent>>,
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
        let input_rb = HeapRb::<MidiEvent>::new(fifo_size);
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

    /// Check if port is active. RT-safe (lock-free atomic load).
    #[inline]
    pub fn is_active(&self) -> bool {
        self.active.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Set port active state. Can be called from any thread.
    #[inline]
    pub fn set_active(&self, active: bool) {
        self.active
            .store(active, std::sync::atomic::Ordering::Release);
    }

    /// Get a lock-free producer handle for MIDI input.
    ///
    /// # Safety
    /// The returned handle must only be used from a single thread (typically
    /// the midir callback thread). This is the SPSC invariant.
    pub fn input_producer_handle(&self) -> InputProducerHandle {
        InputProducerHandle {
            producer: self.input_producer.get(),
        }
    }

    /// Get a lock-free producer handle for unified MIDI input.
    ///
    /// # Safety
    /// The returned handle must only be used from a single thread (SPSC invariant).
    #[cfg(feature = "midi2")]
    pub fn unified_input_producer_handle(&self) -> UnifiedInputProducerHandle {
        UnifiedInputProducerHandle {
            producer: self.unified_input_producer.get(),
        }
    }

    // ==================== RT Thread Methods ====================

    pub fn cycle_start_read_input(&self, _nframes: usize) -> Vec<MidiEvent> {
        let mut events = Vec::new();
        let consumer = unsafe { &mut *self.input_consumer.get() };
        while let Some(event) = consumer.try_pop() {
            events.push(event);
        }
        events
    }

    /// Read all pending unified MIDI events from the input buffer. RT-safe (lock-free).
    #[cfg(feature = "midi2")]
    pub fn cycle_start_read_unified_input(&self) -> Vec<crate::event::UnifiedMidiEvent> {
        let mut events = Vec::new();
        let consumer = unsafe { &mut *self.unified_input_consumer.get() };
        while let Some(event) = consumer.try_pop() {
            events.push(event);
        }
        events
    }

    pub fn write_event(&self, event: MidiEvent) -> bool {
        let producer = unsafe { &mut *self.output_producer.get() };
        producer.try_push(event).is_ok()
    }

    pub fn cycle_end_flush_output(&self) -> Vec<MidiEvent> {
        let mut events = Vec::new();
        let consumer = unsafe { &mut *self.output_consumer.get() };
        while let Some(event) = consumer.try_pop() {
            events.push(event);
        }
        events
    }
}

// SAFETY: AsyncMidiPort is Sync because all UnsafeCell fields follow SPSC invariants:
// 1. input_producer: Only accessed by midir callback thread (single producer)
// 2. input_consumer: Only accessed by audio thread (single consumer)
// 3. output_producer: Only accessed by audio thread (single producer)
// 4. output_consumer: Only accessed by output thread (single consumer)
// 5. unified_input_producer: Only accessed by the programmatic input thread (single producer)
// 6. unified_input_consumer: Only accessed by audio thread (single consumer)
// Each buffer has exactly one producer and one consumer, never accessed concurrently.
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

    #[test]
    fn test_create_port() {
        let port = AsyncMidiPort::new("TestPort", 256);
        assert_eq!(port.name(), "TestPort");
    }

    #[test]
    fn test_input_flow() {
        let port = AsyncMidiPort::new("Input", 256);
        let producer_handle = port.input_producer_handle();

        // Simulate midir callback writing an event
        let event = MidiEvent::note_on(0, 0, 0x3C, 0x7F); // Note On, channel 0, note 60, velocity 127
        assert!(producer_handle.push(event));

        // Simulate audio thread reading events
        let events = port.cycle_start_read_input(512);
        assert_eq!(events.len(), 1);
        assert!(events[0].is_note_on());
        assert_eq!(events[0].note(), Some(0x3C));
        assert_eq!(events[0].velocity(), Some(0x7F));
    }

    #[test]
    fn test_output_flow() {
        let port = AsyncMidiPort::new("Output", 256);

        // Audio thread writes an event
        let event = MidiEvent::note_off(10, 0, 0x3C, 0); // Note Off at frame 10, channel 0, note 60
        assert!(port.write_event(event));

        // Flush output (audio thread sends to hardware)
        let events = port.cycle_end_flush_output();
        assert_eq!(events.len(), 1);
        assert!(events[0].is_note_off());
        assert_eq!(events[0].note(), Some(0x3C));
    }

    #[test]
    fn test_fifo_full() {
        let port = AsyncMidiPort::new("Full", 4); // Small FIFO

        // Fill the output FIFO
        for i in 0..4 {
            let event = MidiEvent::note_on(i, 0, 0x3C, 0x7F);
            assert!(port.write_event(event), "Failed to write event {}", i);
        }

        // Next write should fail (FIFO full)
        let event = MidiEvent::note_on(5, 0, 0x3C, 0x7F);
        assert!(!port.write_event(event), "FIFO should be full");
    }
}
