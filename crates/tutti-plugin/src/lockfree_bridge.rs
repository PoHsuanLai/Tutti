//! Lock-free plugin bridge for RT-safe audio processing.
//!
//! Audio thread → lock-free queues → bridge thread → IPC → plugin server.

use crate::error::Result;
use crate::protocol::{BridgeMessage, HostMessage, MidiEvent, PluginMetadata};
use crate::shared_memory::SharedAudioBuffer;
use crate::transport::MessageTransport;
use crossbeam::queue::ArrayQueue;
use smallvec::SmallVec;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const COMMAND_QUEUE_SIZE: usize = 128;
const RESPONSE_QUEUE_SIZE: usize = 128;

#[derive(Debug)]
pub enum BridgeCommand {
    ProcessAudio {
        buffer_id: u32,
        num_samples: usize,
        midi_events: crate::protocol::MidiEventVec,
    },
    ProcessAudioFull {
        buffer_id: u32,
        num_samples: usize,
        midi_events: crate::protocol::MidiEventVec,
        param_changes: crate::protocol::ParameterChanges,
        note_expression: crate::protocol::NoteExpressionChanges,
        transport: crate::protocol::TransportInfo,
    },
    SetParameter {
        index: i32,
        value: f32,
    },
    GetParameter {
        index: i32,
        response_id: u32,
    },
    SetSampleRate {
        rate: f32,
    },
    Reset,
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum BridgeResponse {
    AudioProcessed {
        buffer_id: u32,
        midi_output: crate::protocol::MidiEventVec,
    },
    AudioProcessedFull {
        buffer_id: u32,
        midi_output: crate::protocol::MidiEventVec,
        param_output: crate::protocol::ParameterChanges,
        note_expression_output: crate::protocol::NoteExpressionChanges,
    },
    ParameterValue {
        response_id: u32,
        value: Option<f32>,
    },
    Error {
        message: String,
    },
}

/// Lock-free bridge for RT-safe plugin communication.
///
/// Cloning is cheap - shared state is behind Arcs.
#[derive(Clone)]
pub struct LockFreeBridge {
    command_queue: Arc<ArrayQueue<BridgeCommand>>,
    response_queue: Arc<ArrayQueue<BridgeResponse>>,
    audio_buffer: Arc<SharedAudioBuffer>,
    buffer_id_counter: Arc<AtomicU32>,
    #[allow(dead_code)]
    response_id_counter: Arc<AtomicU32>,
    running: Arc<AtomicBool>,
    metadata: Arc<parking_lot::Mutex<Option<PluginMetadata>>>,
    num_channels: usize,
    max_samples: usize,
}

/// Bridge thread handle. Drops gracefully when dropped.
pub struct BridgeThreadHandle {
    bridge: LockFreeBridge,
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl LockFreeBridge {
    /// Create bridge and spawn thread for IPC communication.
    pub fn new(
        transport: MessageTransport,
        audio_buffer: Arc<SharedAudioBuffer>,
    ) -> Result<(Self, BridgeThreadHandle)> {
        let command_queue = Arc::new(ArrayQueue::new(COMMAND_QUEUE_SIZE));
        let response_queue = Arc::new(ArrayQueue::new(RESPONSE_QUEUE_SIZE));
        let running = Arc::new(AtomicBool::new(true));

        // Cache buffer dimensions
        let num_channels = audio_buffer.channels();
        let max_samples = audio_buffer.samples();

        // Create the bridge struct
        let bridge = Self {
            command_queue: Arc::clone(&command_queue),
            response_queue: Arc::clone(&response_queue),
            audio_buffer,
            buffer_id_counter: Arc::new(AtomicU32::new(0)),
            response_id_counter: Arc::new(AtomicU32::new(0)),
            running: Arc::clone(&running),
            metadata: Arc::new(parking_lot::Mutex::new(None)),
            num_channels,
            max_samples,
        };

        // Spawn bridge thread
        let bridge_thread =
            Self::spawn_bridge_thread(command_queue, response_queue, running, transport);

        let handle = BridgeThreadHandle {
            bridge: bridge.clone(),
            thread_handle: Some(bridge_thread),
        };

        Ok((bridge, handle))
    }

    fn spawn_bridge_thread(
        command_queue: Arc<ArrayQueue<BridgeCommand>>,
        response_queue: Arc<ArrayQueue<BridgeResponse>>,
        running: Arc<AtomicBool>,
        mut transport: MessageTransport,
    ) -> thread::JoinHandle<()> {
        thread::Builder::new()
            .name("plugin-bridge".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(_e) => return,
                };

                runtime.block_on(async {
                    Self::bridge_thread_main(
                        command_queue,
                        response_queue,
                        running,
                        &mut transport,
                    )
                    .await;
                });
            })
            .expect("Failed to spawn bridge thread")
    }

    async fn bridge_thread_main(
        command_queue: Arc<ArrayQueue<BridgeCommand>>,
        response_queue: Arc<ArrayQueue<BridgeResponse>>,
        running: Arc<AtomicBool>,
        transport: &mut MessageTransport,
    ) {
        while running.load(Ordering::Relaxed) {
            if let Some(command) = command_queue.pop() {
                match Self::handle_command(command, transport, &response_queue).await {
                    Ok(()) => {}
                    Err(e) => {
                        let _ = response_queue.push(BridgeResponse::Error {
                            message: e.to_string(),
                        });
                    }
                }
            } else {
                tokio::time::sleep(Duration::from_micros(100)).await;
            }
        }
    }

    async fn handle_command(
        command: BridgeCommand,
        transport: &mut MessageTransport,
        response_queue: &Arc<ArrayQueue<BridgeResponse>>,
    ) -> Result<()> {
        match command {
            BridgeCommand::ProcessAudio {
                buffer_id,
                num_samples,
                midi_events,
            } => {
                let message = if midi_events.is_empty() {
                    HostMessage::ProcessAudio {
                        buffer_id,
                        num_samples,
                    }
                } else {
                    HostMessage::ProcessAudioMidi {
                        buffer_id,
                        num_samples,
                        midi_events,
                    }
                };

                transport.send_host_message(&message).await?;
                let response = transport.recv_bridge_message().await?;

                match response {
                    BridgeMessage::AudioProcessed { .. } => {
                        let _ = response_queue.push(BridgeResponse::AudioProcessed {
                            buffer_id,
                            midi_output: SmallVec::new(),
                        });
                    }
                    BridgeMessage::AudioProcessedMidi { midi_output, .. } => {
                        let _ = response_queue.push(BridgeResponse::AudioProcessed {
                            buffer_id,
                            midi_output,
                        });
                    }
                    BridgeMessage::Error { message } => {
                        let _ = response_queue.push(BridgeResponse::Error { message });
                    }
                    _ => {}
                }
            }

            BridgeCommand::SetParameter { index, value } => {
                transport
                    .send_host_message(&HostMessage::SetParameter {
                        id: index.to_string(),
                        value,
                    })
                    .await?;
            }

            BridgeCommand::GetParameter { index, response_id } => {
                transport
                    .send_host_message(&HostMessage::GetParameter {
                        id: index.to_string(),
                    })
                    .await?;

                let response = transport.recv_bridge_message().await?;

                if let BridgeMessage::ParameterValue { value } = response {
                    let _ =
                        response_queue.push(BridgeResponse::ParameterValue { response_id, value });
                }
            }

            BridgeCommand::SetSampleRate { rate } => {
                transport
                    .send_host_message(&HostMessage::SetSampleRate { rate })
                    .await?;
            }

            BridgeCommand::ProcessAudioFull {
                buffer_id,
                num_samples,
                midi_events,
                param_changes,
                note_expression,
                transport: transport_info,
            } => {
                let message = HostMessage::ProcessAudioFull {
                    buffer_id,
                    num_samples,
                    midi_events,
                    param_changes,
                    note_expression,
                    transport: transport_info,
                };

                transport.send_host_message(&message).await?;
                let response = transport.recv_bridge_message().await?;

                match response {
                    BridgeMessage::AudioProcessedFull {
                        midi_output,
                        param_output,
                        note_expression_output,
                        ..
                    } => {
                        let _ = response_queue.push(BridgeResponse::AudioProcessedFull {
                            buffer_id,
                            midi_output,
                            param_output,
                            note_expression_output,
                        });
                    }
                    BridgeMessage::AudioProcessedMidi { midi_output, .. } => {
                        let _ = response_queue.push(BridgeResponse::AudioProcessed {
                            buffer_id,
                            midi_output,
                        });
                    }
                    BridgeMessage::AudioProcessed { .. } => {
                        let _ = response_queue.push(BridgeResponse::AudioProcessed {
                            buffer_id,
                            midi_output: SmallVec::new(),
                        });
                    }
                    BridgeMessage::Error { message } => {
                        let _ = response_queue.push(BridgeResponse::Error { message });
                    }
                    _ => {}
                }
            }

            BridgeCommand::Reset => {
                transport.send_host_message(&HostMessage::Reset).await?;
            }

            BridgeCommand::Shutdown => {
                transport.send_host_message(&HostMessage::Shutdown).await?;
            }
        }

        Ok(())
    }

    /// Process audio (RT-safe, lock-free). Never blocks or allocates.
    pub fn process_audio_rt(
        &self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        midi_events: crate::protocol::MidiEventVec,
    ) -> bool {
        if inputs.is_empty() || inputs[0].is_empty() {
            return false;
        }
        let num_samples = inputs[0].len();

        for (ch, input) in inputs.iter().enumerate() {
            if self.audio_buffer.write_channel(ch, input).is_err() {
                return false;
            }
        }

        let buffer_id = self.buffer_id_counter.fetch_add(1, Ordering::Relaxed);

        if self
            .command_queue
            .push(BridgeCommand::ProcessAudio {
                buffer_id,
                num_samples,
                midi_events,
            })
            .is_err()
        {
            return false;
        }

        if let Some(response) = self.response_queue.pop() {
            match response {
                BridgeResponse::AudioProcessed {
                    buffer_id: resp_id, ..
                } => {
                    if resp_id == buffer_id {
                        for (ch, output) in outputs.iter_mut().enumerate() {
                            if let Ok(data) = self.audio_buffer.read_channel(ch) {
                                let copy_len = data.len().min(output.len());
                                output[..copy_len].copy_from_slice(&data[..copy_len]);
                            }
                        }
                        return true;
                    }
                }
                BridgeResponse::Error { message: _ } => {
                    return false;
                }
                _ => {}
            }
        }

        false
    }

    /// Process audio with f64 buffers (RT-safe, lock-free).
    pub fn process_audio_rt_f64(
        &self,
        inputs: &[&[f64]],
        outputs: &mut [&mut [f64]],
        midi_events: crate::protocol::MidiEventVec,
    ) -> bool {
        if inputs.is_empty() || inputs[0].is_empty() {
            return false;
        }
        let num_samples = inputs[0].len();

        for (ch, input) in inputs.iter().enumerate() {
            if self.audio_buffer.write_channel_f64(ch, input).is_err() {
                return false;
            }
        }

        let buffer_id = self.buffer_id_counter.fetch_add(1, Ordering::Relaxed);

        if self
            .command_queue
            .push(BridgeCommand::ProcessAudio {
                buffer_id,
                num_samples,
                midi_events,
            })
            .is_err()
        {
            return false;
        }

        if let Some(response) = self.response_queue.pop() {
            match response {
                BridgeResponse::AudioProcessed {
                    buffer_id: resp_id, ..
                } => {
                    if resp_id == buffer_id {
                        for (ch, output) in outputs.iter_mut().enumerate() {
                            if let Ok(data) = self.audio_buffer.read_channel_f64(ch) {
                                let copy_len = data.len().min(output.len());
                                output[..copy_len].copy_from_slice(&data[..copy_len]);
                            }
                        }
                        return true;
                    }
                }
                BridgeResponse::Error { message: _ } => return false,
                _ => {}
            }
        }
        false
    }

    pub fn set_parameter_rt(&self, index: i32, value: f32) -> bool {
        self.command_queue
            .push(BridgeCommand::SetParameter { index, value })
            .is_ok()
    }

    pub fn set_sample_rate_rt(&self, rate: f32) -> bool {
        self.command_queue
            .push(BridgeCommand::SetSampleRate { rate })
            .is_ok()
    }

    pub fn reset_rt(&self) -> bool {
        self.command_queue.push(BridgeCommand::Reset).is_ok()
    }

    pub fn write_input_channel(&self, channel: usize, data: &[f32]) -> Result<()> {
        self.audio_buffer.write_channel(channel, data)
    }

    pub fn read_output_channel(&self, channel: usize) -> Result<Vec<f32>> {
        self.audio_buffer.read_channel(channel)
    }

    pub fn read_output_channel_into(&self, channel: usize, output: &mut [f32]) -> Result<usize> {
        self.audio_buffer.read_channel_into(channel, output)
    }

    pub fn sample_format(&self) -> crate::protocol::SampleFormat {
        self.audio_buffer.sample_format()
    }

    pub fn write_input_channel_f64(&self, channel: usize, data: &[f64]) -> Result<()> {
        self.audio_buffer.write_channel_f64(channel, data)
    }

    pub fn read_output_channel_f64(&self, channel: usize) -> Result<Vec<f64>> {
        self.audio_buffer.read_channel_f64(channel)
    }

    pub fn read_output_channel_into_f64(
        &self,
        channel: usize,
        output: &mut [f64],
    ) -> Result<usize> {
        self.audio_buffer.read_channel_into_f64(channel, output)
    }

    pub fn channels(&self) -> usize {
        self.num_channels
    }

    pub fn max_samples(&self) -> usize {
        self.max_samples
    }

    pub fn process_rt(&self, num_samples: usize) -> bool {
        self.process_rt_with_midi(num_samples, &[])
    }

    pub fn process_rt_with_midi(&self, num_samples: usize, midi_events: &[MidiEvent]) -> bool {
        let buffer_id = self.buffer_id_counter.fetch_add(1, Ordering::Relaxed);

        if self
            .command_queue
            .push(BridgeCommand::ProcessAudio {
                buffer_id,
                num_samples,
                midi_events: midi_events.iter().copied().collect(),
            })
            .is_err()
        {
            return false;
        }

        if let Some(response) = self.response_queue.pop() {
            matches!(response, BridgeResponse::AudioProcessed { .. })
        } else {
            false
        }
    }

    pub fn process_rt_with_automation(
        &self,
        num_samples: usize,
        midi_events: &[MidiEvent],
        param_changes: &crate::protocol::ParameterChanges,
        note_expression: &crate::protocol::NoteExpressionChanges,
        transport: &crate::protocol::TransportInfo,
    ) -> bool {
        let buffer_id = self.buffer_id_counter.fetch_add(1, Ordering::Relaxed);

        if self
            .command_queue
            .push(BridgeCommand::ProcessAudioFull {
                buffer_id,
                num_samples,
                midi_events: midi_events.iter().copied().collect(),
                param_changes: param_changes.clone(),
                note_expression: note_expression.clone(),
                transport: *transport,
            })
            .is_err()
        {
            return false;
        }

        if let Some(response) = self.response_queue.pop() {
            matches!(
                response,
                BridgeResponse::AudioProcessed { .. } | BridgeResponse::AudioProcessedFull { .. }
            )
        } else {
            false
        }
    }

    pub fn metadata(&self) -> Option<PluginMetadata> {
        self.metadata.lock().clone()
    }

    pub fn set_metadata(&self, metadata: PluginMetadata) {
        *self.metadata.lock() = Some(metadata);
    }
}

impl BridgeThreadHandle {
    pub fn shutdown(&mut self) {
        self.bridge.running.store(false, Ordering::Relaxed);
        let _ = self.bridge.command_queue.push(BridgeCommand::Shutdown);

        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for BridgeThreadHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_queue_capacity() {
        let queue = ArrayQueue::<BridgeCommand>::new(COMMAND_QUEUE_SIZE);

        // Should be able to push up to capacity
        for i in 0..COMMAND_QUEUE_SIZE {
            assert!(queue
                .push(BridgeCommand::SetParameter {
                    index: i as i32,
                    value: 0.0
                })
                .is_ok());
        }

        // Next push should fail
        assert!(queue
            .push(BridgeCommand::SetParameter {
                index: 0,
                value: 0.0
            })
            .is_err());
    }

    #[test]
    fn test_response_queue_capacity() {
        let queue = ArrayQueue::<BridgeResponse>::new(RESPONSE_QUEUE_SIZE);

        // Should be able to push up to capacity
        for i in 0..RESPONSE_QUEUE_SIZE {
            assert!(queue
                .push(BridgeResponse::AudioProcessed {
                    buffer_id: i as u32,
                    midi_output: SmallVec::new(),
                })
                .is_ok());
        }

        // Next push should fail
        assert!(queue
            .push(BridgeResponse::AudioProcessed {
                buffer_id: 0,
                midi_output: SmallVec::new(),
            })
            .is_err());
    }
}
