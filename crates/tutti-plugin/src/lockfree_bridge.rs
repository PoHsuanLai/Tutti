//! Lock-free plugin bridge for RT-safe audio processing.
//!
//! Audio thread → lock-free queues → bridge thread → IPC → plugin server.

use crate::error::Result;
use crate::protocol::{BridgeMessage, HostMessage, IpcMidiEvent};
use crate::shared_memory::SharedAudioBuffer;
use crate::transport::MessageTransport;
use crossbeam::queue::ArrayQueue;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const COMMAND_QUEUE_SIZE: usize = 128;
const RESPONSE_QUEUE_SIZE: usize = 128;

#[derive(Debug)]
enum BridgeCommand {
    Process {
        buffer_id: u32,
        num_samples: usize,
        midi_events: crate::protocol::MidiEventVec,
        param_changes: crate::protocol::ParameterChanges,
        note_expression: crate::protocol::NoteExpressionChanges,
        transport: crate::protocol::TransportInfo,
    },
    SetParameter {
        param_id: u32,
        value: f32,
    },
    SetSampleRate {
        rate: f64,
    },
    Reset,
    Shutdown,
}

#[derive(Debug, Clone)]
enum BridgeResponse {
    AudioProcessed,
    Error,
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
    running: Arc<AtomicBool>,
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

        let bridge = Self {
            command_queue: Arc::clone(&command_queue),
            response_queue: Arc::clone(&response_queue),
            audio_buffer,
            buffer_id_counter: Arc::new(AtomicU32::new(0)),
            running: Arc::clone(&running),
        };

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
                if Self::handle_command(command, transport, &response_queue).await.is_err() {
                    let _ = response_queue.push(BridgeResponse::Error);
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
            BridgeCommand::Process {
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
                    midi_events: midi_events.iter().map(IpcMidiEvent::from).collect(),
                    param_changes,
                    note_expression,
                    transport: transport_info,
                };

                transport.send_host_message(&message).await?;
                let response = transport.recv_bridge_message().await?;

                match response {
                    BridgeMessage::AudioProcessedFull { .. }
                    | BridgeMessage::AudioProcessedMidi { .. }
                    | BridgeMessage::AudioProcessed { .. } => {
                        let _ = response_queue.push(BridgeResponse::AudioProcessed);
                    }
                    BridgeMessage::Error { .. } => {
                        let _ = response_queue.push(BridgeResponse::Error);
                    }
                    _ => {}
                }
            }

            BridgeCommand::SetParameter { param_id, value } => {
                transport
                    .send_host_message(&HostMessage::SetParameter { param_id, value })
                    .await?;
            }

            BridgeCommand::SetSampleRate { rate } => {
                transport
                    .send_host_message(&HostMessage::SetSampleRate { rate })
                    .await?;
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

    pub fn set_parameter_rt(&self, param_id: u32, value: f32) -> bool {
        self.command_queue
            .push(BridgeCommand::SetParameter { param_id, value })
            .is_ok()
    }

    pub fn set_sample_rate_rt(&self, rate: f64) -> bool {
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

    pub fn read_output_channel_into(&self, channel: usize, output: &mut [f32]) -> Result<usize> {
        self.audio_buffer.read_channel_into(channel, output)
    }

    pub fn write_input_channel_f64(&self, channel: usize, data: &[f64]) -> Result<()> {
        self.audio_buffer.write_channel_f64(channel, data)
    }

    pub fn read_output_channel_into_f64(
        &self,
        channel: usize,
        output: &mut [f64],
    ) -> Result<usize> {
        self.audio_buffer.read_channel_into_f64(channel, output)
    }

    /// Process audio (RT-safe, lock-free).
    ///
    /// Call `write_input_channel` before this, and `read_output_channel_into` after.
    pub fn process(
        &self,
        num_samples: usize,
        midi_events: crate::protocol::MidiEventVec,
        param_changes: crate::protocol::ParameterChanges,
        note_expression: crate::protocol::NoteExpressionChanges,
        transport: crate::protocol::TransportInfo,
    ) -> bool {
        let buffer_id = self.buffer_id_counter.fetch_add(1, Ordering::Relaxed);

        if self
            .command_queue
            .push(BridgeCommand::Process {
                buffer_id,
                num_samples,
                midi_events,
                param_changes,
                note_expression,
                transport,
            })
            .is_err()
        {
            return false;
        }

        if let Some(response) = self.response_queue.pop() {
            matches!(response, BridgeResponse::AudioProcessed)
        } else {
            false
        }
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
