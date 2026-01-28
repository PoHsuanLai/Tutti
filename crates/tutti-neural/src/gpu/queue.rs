//! Lock-free SPSC queues for neural parameters.

use thingbuf::mpsc::{channel, Receiver, Sender};

/// Lock-free SPSC queue for neural parameters.
pub struct NeuralParamQueue {
    receiver: Receiver<ControlParams>,
    sender: Option<Sender<ControlParams>>,
}

impl NeuralParamQueue {
    /// Create a queue with the given capacity (rounded up to power of 2).
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.next_power_of_two();

        let (sender, receiver) = channel(capacity);

        Self {
            receiver,
            sender: Some(sender),
        }
    }

    /// Take the sender for use on the inference thread.
    pub fn take_sender(&mut self) -> Option<Sender<ControlParams>> {
        self.sender.take()
    }

    /// Try to pop the latest parameters (non-blocking).
    pub fn try_pop(&self) -> Option<ControlParams> {
        self.receiver.try_recv().ok()
    }

    #[cfg(test)]
    pub(crate) fn try_push(&self, params: ControlParams) -> Result<(), ControlParams> {
        if let Some(sender) = &self.sender {
            sender.try_send(params).map_err(|e| e.into_inner())
        } else {
            Err(params)
        }
    }
}

unsafe impl Sync for NeuralParamQueue {}

/// Sender for neural parameters.
pub struct ParamSender {
    sender: Sender<ControlParams>,
}

impl ParamSender {
    pub fn new(sender: Sender<ControlParams>) -> Self {
        Self { sender }
    }

    pub fn try_send(&self, params: ControlParams) -> Result<(), ControlParams> {
        self.sender.try_send(params).map_err(|e| e.into_inner())
    }
}

/// Control parameters from neural inference
///
/// These are the output of DDSP neural inference, used by the synthesizer
/// to generate audio in real-time.
#[derive(Debug, Clone, Default)]
pub struct ControlParams {
    /// Fundamental frequencies (f0) per sample
    ///
    /// Length: buffer_size (e.g., 512 samples)
    /// Range: 20 Hz - 4000 Hz (typical vocal range)
    pub f0: Vec<f32>,

    /// Harmonic amplitudes per sample
    ///
    /// Length: buffer_size (e.g., 512 samples)
    /// Range: 0.0 - 1.0 (normalized amplitude)
    pub amplitudes: Vec<f32>,
}
