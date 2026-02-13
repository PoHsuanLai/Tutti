//! Lock-free bridge for RT-safe plugin communication.

use crate::error::Result;
use crate::protocol::{BridgeMessage, HostMessage, IpcMidiEvent, ParameterInfo};
use crate::shared_memory::SharedAudioBuffer;
use crate::transport::MessageTransport;
use crossbeam::queue::ArrayQueue;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const COMMAND_QUEUE_SIZE: usize = 128;
const RESPONSE_QUEUE_SIZE: usize = 128;
const CONTROL_RESPONSE_QUEUE_SIZE: usize = 32;

#[derive(Debug)]
struct ProcessCommandData {
    buffer_id: u32,
    num_samples: usize,
    midi_events: crate::protocol::MidiEventVec,
    param_changes: crate::protocol::ParameterChanges,
    note_expression: crate::protocol::NoteExpressionChanges,
    transport: crate::protocol::TransportInfo,
}

#[derive(Debug)]
enum BridgeCommand {
    // RT commands (audio thread)
    Process(Box<ProcessCommandData>),
    SetParameter { param_id: u32, value: f32 },
    SetSampleRate { rate: f64 },
    Reset,
    Shutdown,
    // Control commands (main thread, non-RT)
    OpenEditor { parent_handle: u64 },
    CloseEditor,
    EditorIdle,
    SaveState,
    LoadState { data: Vec<u8> },
    GetParameterList,
    GetParameter { param_id: u32 },
}

#[derive(Debug, Clone)]
enum BridgeResponse {
    AudioProcessed,
    Error,
}

/// Response type for non-RT control operations.
#[derive(Debug)]
pub(crate) enum ControlResponse {
    EditorOpened { width: u32, height: u32 },
    EditorClosed,
    StateSaved { data: Vec<u8> },
    StateLoaded,
    ParameterList { parameters: Vec<ParameterInfo> },
    ParameterValue { value: Option<f32> },
    Error,
}

/// Lock-free bridge for RT-safe plugin communication. Clone is cheap.
#[derive(Clone)]
pub struct LockFreeBridge {
    command_queue: Arc<ArrayQueue<BridgeCommand>>,
    response_queue: Arc<ArrayQueue<BridgeResponse>>,
    control_response_queue: Arc<ArrayQueue<ControlResponse>>,
    audio_buffer: Arc<SharedAudioBuffer>,
    buffer_id_counter: Arc<AtomicU32>,
    running: Arc<AtomicBool>,
    crashed: Arc<AtomicBool>,
    /// RT-safe recycling: bridge thread returns used Box here, audio thread reuses it.
    recycle_queue: Arc<ArrayQueue<Box<ProcessCommandData>>>,
}

/// Handle to bridge thread. Shuts down on drop.
pub struct BridgeThreadHandle {
    bridge: LockFreeBridge,
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl LockFreeBridge {
    pub fn new(
        transport: MessageTransport,
        audio_buffer: Arc<SharedAudioBuffer>,
    ) -> Result<(Self, BridgeThreadHandle)> {
        let command_queue = Arc::new(ArrayQueue::new(COMMAND_QUEUE_SIZE));
        let response_queue = Arc::new(ArrayQueue::new(RESPONSE_QUEUE_SIZE));
        let control_response_queue = Arc::new(ArrayQueue::new(CONTROL_RESPONSE_QUEUE_SIZE));
        let recycle_queue = Arc::new(ArrayQueue::new(2));
        let running = Arc::new(AtomicBool::new(true));
        let crashed = Arc::new(AtomicBool::new(false));

        let bridge = Self {
            command_queue: Arc::clone(&command_queue),
            response_queue: Arc::clone(&response_queue),
            control_response_queue: Arc::clone(&control_response_queue),
            audio_buffer,
            buffer_id_counter: Arc::new(AtomicU32::new(0)),
            running: Arc::clone(&running),
            crashed: Arc::clone(&crashed),
            recycle_queue: Arc::clone(&recycle_queue),
        };

        let thread = Self::spawn_thread(
            command_queue,
            response_queue,
            control_response_queue,
            recycle_queue,
            running,
            crashed,
            transport,
        );
        let handle = BridgeThreadHandle {
            bridge: bridge.clone(),
            thread_handle: Some(thread),
        };

        Ok((bridge, handle))
    }

    fn spawn_thread(
        command_queue: Arc<ArrayQueue<BridgeCommand>>,
        response_queue: Arc<ArrayQueue<BridgeResponse>>,
        control_response_queue: Arc<ArrayQueue<ControlResponse>>,
        recycle_queue: Arc<ArrayQueue<Box<ProcessCommandData>>>,
        running: Arc<AtomicBool>,
        crashed: Arc<AtomicBool>,
        mut transport: MessageTransport,
    ) -> thread::JoinHandle<()> {
        thread::Builder::new()
            .name("plugin-bridge".to_string())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                runtime.block_on(Self::run(
                    command_queue,
                    response_queue,
                    control_response_queue,
                    recycle_queue,
                    running,
                    crashed,
                    &mut transport,
                ));
            })
            .expect("failed to spawn bridge thread")
    }

    /// Create a bridge from a raw std UnixStream. The tokio conversion happens
    /// inside the bridge thread's own runtime, avoiding cross-runtime issues.
    #[cfg(all(test, unix))]
    pub(crate) fn new_from_std_stream(
        stream: std::os::unix::net::UnixStream,
        audio_buffer: Arc<SharedAudioBuffer>,
    ) -> crate::error::Result<(Self, BridgeThreadHandle)> {
        let command_queue = Arc::new(ArrayQueue::new(COMMAND_QUEUE_SIZE));
        let response_queue = Arc::new(ArrayQueue::new(RESPONSE_QUEUE_SIZE));
        let control_response_queue = Arc::new(ArrayQueue::new(CONTROL_RESPONSE_QUEUE_SIZE));
        let recycle_queue = Arc::new(ArrayQueue::new(2));
        let running = Arc::new(AtomicBool::new(true));
        let crashed = Arc::new(AtomicBool::new(false));

        let bridge = Self {
            command_queue: Arc::clone(&command_queue),
            response_queue: Arc::clone(&response_queue),
            control_response_queue: Arc::clone(&control_response_queue),
            audio_buffer,
            buffer_id_counter: Arc::new(AtomicU32::new(0)),
            running: Arc::clone(&running),
            crashed: Arc::clone(&crashed),
            recycle_queue: Arc::clone(&recycle_queue),
        };

        let thread = {
            let cmd_q = command_queue;
            let resp_q = response_queue;
            let ctrl_q = control_response_queue;
            let recycle_q = recycle_queue;
            let run = running;
            let crash = crashed;
            thread::Builder::new()
                .name("plugin-bridge".to_string())
                .spawn(move || {
                    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    else {
                        return;
                    };
                    runtime.block_on(async {
                        // Convert std stream to tokio inside THIS runtime's reactor
                        let Ok(mut transport) = MessageTransport::from_std_stream(stream) else {
                            return;
                        };
                        Self::run(cmd_q, resp_q, ctrl_q, recycle_q, run, crash, &mut transport)
                            .await;
                    });
                })
                .expect("failed to spawn bridge thread")
        };

        let handle = BridgeThreadHandle {
            bridge: bridge.clone(),
            thread_handle: Some(thread),
        };

        Ok((bridge, handle))
    }

    async fn run(
        commands: Arc<ArrayQueue<BridgeCommand>>,
        responses: Arc<ArrayQueue<BridgeResponse>>,
        control_responses: Arc<ArrayQueue<ControlResponse>>,
        recycle: Arc<ArrayQueue<Box<ProcessCommandData>>>,
        running: Arc<AtomicBool>,
        crashed: Arc<AtomicBool>,
        transport: &mut MessageTransport,
    ) {
        while running.load(Ordering::Relaxed) {
            if let Some(cmd) = commands.pop() {
                if Self::handle(cmd, transport, &responses, &control_responses, &recycle)
                    .await
                    .is_err()
                {
                    // Transport error = server crashed or disconnected
                    crashed.store(true, Ordering::Release);
                    let _ = responses.push(BridgeResponse::Error);
                    let _ = control_responses.push(ControlResponse::Error);
                    // Drain remaining commands with error responses
                    while let Some(_cmd) = commands.pop() {
                        let _ = responses.push(BridgeResponse::Error);
                    }
                    break;
                }
            } else {
                tokio::time::sleep(Duration::from_micros(100)).await;
            }
        }
    }

    async fn handle(
        cmd: BridgeCommand,
        transport: &mut MessageTransport,
        responses: &Arc<ArrayQueue<BridgeResponse>>,
        control_responses: &Arc<ArrayQueue<ControlResponse>>,
        recycle: &Arc<ArrayQueue<Box<ProcessCommandData>>>,
    ) -> Result<()> {
        match cmd {
            BridgeCommand::Process(mut data) => {
                let msg = HostMessage::ProcessAudioFull(Box::new(
                    crate::protocol::ProcessAudioFullData {
                        buffer_id: data.buffer_id,
                        num_samples: data.num_samples,
                        midi_events: data.midi_events.iter().map(IpcMidiEvent::from).collect(),
                        param_changes: core::mem::take(&mut data.param_changes),
                        note_expression: core::mem::take(&mut data.note_expression),
                        transport: core::mem::take(&mut data.transport),
                    },
                ));

                // Recycle the Box for RT-safe reuse by the audio thread.
                let _ = recycle.push(data);

                transport.send_host_message(&msg).await?;

                match transport.recv_with_timeout(Duration::from_secs(30)).await? {
                    BridgeMessage::AudioProcessedFull { .. }
                    | BridgeMessage::AudioProcessedMidi { .. }
                    | BridgeMessage::AudioProcessed { .. } => {
                        let _ = responses.push(BridgeResponse::AudioProcessed);
                    }
                    BridgeMessage::Error { .. } => {
                        let _ = responses.push(BridgeResponse::Error);
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
            // Control commands — send and wait for response, route to control queue
            BridgeCommand::OpenEditor { parent_handle } => {
                transport
                    .send_host_message(&HostMessage::OpenEditor { parent_handle })
                    .await?;
                match transport.recv_with_timeout(Duration::from_secs(10)).await? {
                    BridgeMessage::EditorOpened { width, height } => {
                        let _ =
                            control_responses.push(ControlResponse::EditorOpened { width, height });
                    }
                    BridgeMessage::Error { .. } => {
                        let _ = control_responses.push(ControlResponse::Error);
                    }
                    _ => {
                        let _ = control_responses.push(ControlResponse::Error);
                    }
                }
            }
            BridgeCommand::CloseEditor => {
                transport
                    .send_host_message(&HostMessage::CloseEditor)
                    .await?;
                match transport.recv_with_timeout(Duration::from_secs(5)).await? {
                    BridgeMessage::EditorClosed => {
                        let _ = control_responses.push(ControlResponse::EditorClosed);
                    }
                    _ => {
                        let _ = control_responses.push(ControlResponse::EditorClosed);
                    }
                }
            }
            BridgeCommand::EditorIdle => {
                transport
                    .send_host_message(&HostMessage::EditorIdle)
                    .await?;
                // EditorIdle is fire-and-forget, no response expected
            }
            BridgeCommand::SaveState => {
                transport.send_host_message(&HostMessage::SaveState).await?;
                match transport.recv_with_timeout(Duration::from_secs(10)).await? {
                    BridgeMessage::StateData { data } => {
                        let _ = control_responses.push(ControlResponse::StateSaved { data });
                    }
                    BridgeMessage::Error { .. } => {
                        let _ = control_responses.push(ControlResponse::Error);
                    }
                    _ => {
                        let _ = control_responses.push(ControlResponse::Error);
                    }
                }
            }
            BridgeCommand::LoadState { data } => {
                transport
                    .send_host_message(&HostMessage::LoadState { data })
                    .await?;
                // LoadState doesn't have a defined response in the protocol,
                // but we signal completion
                let _ = control_responses.push(ControlResponse::StateLoaded);
            }
            BridgeCommand::GetParameterList => {
                transport
                    .send_host_message(&HostMessage::GetParameterList)
                    .await?;
                match transport.recv_with_timeout(Duration::from_secs(5)).await? {
                    BridgeMessage::ParameterList { parameters } => {
                        let _ =
                            control_responses.push(ControlResponse::ParameterList { parameters });
                    }
                    BridgeMessage::Error { .. } => {
                        let _ = control_responses.push(ControlResponse::Error);
                    }
                    _ => {
                        let _ = control_responses.push(ControlResponse::Error);
                    }
                }
            }
            BridgeCommand::GetParameter { param_id } => {
                transport
                    .send_host_message(&HostMessage::GetParameter { param_id })
                    .await?;
                match transport.recv_with_timeout(Duration::from_secs(5)).await? {
                    BridgeMessage::ParameterValue { value } => {
                        let _ = control_responses.push(ControlResponse::ParameterValue { value });
                    }
                    BridgeMessage::Error { .. } => {
                        let _ =
                            control_responses.push(ControlResponse::ParameterValue { value: None });
                    }
                    _ => {
                        let _ =
                            control_responses.push(ControlResponse::ParameterValue { value: None });
                    }
                }
            }
        }
        Ok(())
    }

    pub fn is_crashed(&self) -> bool {
        self.crashed.load(Ordering::Acquire)
    }

    pub fn set_parameter_rt(&self, param_id: u32, value: f32) -> bool {
        if self.crashed.load(Ordering::Acquire) {
            return false;
        }
        self.command_queue
            .push(BridgeCommand::SetParameter { param_id, value })
            .is_ok()
    }

    pub fn set_sample_rate_rt(&self, rate: f64) -> bool {
        if self.crashed.load(Ordering::Acquire) {
            return false;
        }
        self.command_queue
            .push(BridgeCommand::SetSampleRate { rate })
            .is_ok()
    }

    pub fn reset_rt(&self) -> bool {
        if self.crashed.load(Ordering::Acquire) {
            return false;
        }
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

    /// Process audio. RT-safe, lock-free.
    pub fn process(
        &self,
        num_samples: usize,
        midi_events: crate::protocol::MidiEventVec,
        param_changes: crate::protocol::ParameterChanges,
        note_expression: crate::protocol::NoteExpressionChanges,
        transport: crate::protocol::TransportInfo,
    ) -> bool {
        if self.crashed.load(Ordering::Acquire) {
            return false;
        }
        let buffer_id = self.buffer_id_counter.fetch_add(1, Ordering::Relaxed);

        // RT-safe: reuse a recycled Box if available, avoiding heap allocation.
        let mut data = self.recycle_queue.pop().unwrap_or_else(|| {
            Box::new(ProcessCommandData {
                buffer_id: 0,
                num_samples: 0,
                midi_events: crate::protocol::MidiEventVec::new(),
                param_changes: crate::protocol::ParameterChanges::new(),
                note_expression: crate::protocol::NoteExpressionChanges::new(),
                transport: crate::protocol::TransportInfo::default(),
            })
        });
        data.buffer_id = buffer_id;
        data.num_samples = num_samples;
        data.midi_events = midi_events;
        data.param_changes = param_changes;
        data.note_expression = note_expression;
        data.transport = transport;

        if self
            .command_queue
            .push(BridgeCommand::Process(data))
            .is_err()
        {
            return false;
        }

        matches!(
            self.response_queue.pop(),
            Some(BridgeResponse::AudioProcessed)
        )
    }

    /// Wait for a control response with timeout.
    fn wait_control_response(&self, timeout: Duration) -> Option<ControlResponse> {
        let start = Instant::now();
        loop {
            if let Some(resp) = self.control_response_queue.pop() {
                return Some(resp);
            }
            if start.elapsed() >= timeout {
                return None;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Open the plugin editor GUI. Returns (width, height) on success.
    ///
    /// Non-RT. Must be called from the main thread.
    pub fn open_editor(&self, parent_handle: u64) -> Option<(u32, u32)> {
        if self.crashed.load(Ordering::Acquire) {
            return None;
        }
        self.command_queue
            .push(BridgeCommand::OpenEditor { parent_handle })
            .ok()?;
        match self.wait_control_response(Duration::from_secs(10))? {
            ControlResponse::EditorOpened { width, height } => Some((width, height)),
            _ => None,
        }
    }

    /// Close the plugin editor GUI.
    ///
    /// Non-RT. Must be called from the main thread.
    pub fn close_editor(&self) -> bool {
        if self.crashed.load(Ordering::Acquire) {
            return false;
        }
        if self.command_queue.push(BridgeCommand::CloseEditor).is_err() {
            return false;
        }
        matches!(
            self.wait_control_response(Duration::from_secs(5)),
            Some(ControlResponse::EditorClosed)
        )
    }

    /// Tick the plugin editor idle loop.
    ///
    /// Non-RT. Fire-and-forget.
    pub fn editor_idle(&self) {
        if self.crashed.load(Ordering::Acquire) {
            return;
        }
        let _ = self.command_queue.push(BridgeCommand::EditorIdle);
    }

    /// Save the plugin state. Returns the state bytes on success.
    ///
    /// Non-RT. Must be called from the main thread.
    pub fn save_state(&self) -> Option<Vec<u8>> {
        if self.crashed.load(Ordering::Acquire) {
            return None;
        }
        self.command_queue.push(BridgeCommand::SaveState).ok()?;
        match self.wait_control_response(Duration::from_secs(10))? {
            ControlResponse::StateSaved { data } => Some(data),
            _ => None,
        }
    }

    /// Load plugin state from bytes.
    ///
    /// Non-RT. Must be called from the main thread.
    pub fn load_state(&self, data: &[u8]) -> bool {
        if self.crashed.load(Ordering::Acquire) {
            return false;
        }
        if self
            .command_queue
            .push(BridgeCommand::LoadState {
                data: data.to_vec(),
            })
            .is_err()
        {
            return false;
        }
        matches!(
            self.wait_control_response(Duration::from_secs(10)),
            Some(ControlResponse::StateLoaded)
        )
    }

    /// Get the full parameter list from the plugin.
    ///
    /// Non-RT. Must be called from the main thread.
    pub fn get_parameter_list(&self) -> Option<Vec<ParameterInfo>> {
        if self.crashed.load(Ordering::Acquire) {
            return None;
        }
        self.command_queue
            .push(BridgeCommand::GetParameterList)
            .ok()?;
        match self.wait_control_response(Duration::from_secs(5))? {
            ControlResponse::ParameterList { parameters } => Some(parameters),
            _ => None,
        }
    }

    /// Get a single parameter value.
    ///
    /// Non-RT. Must be called from the main thread.
    pub fn get_parameter(&self, param_id: u32) -> Option<f32> {
        if self.crashed.load(Ordering::Acquire) {
            return None;
        }
        self.command_queue
            .push(BridgeCommand::GetParameter { param_id })
            .ok()?;
        match self.wait_control_response(Duration::from_secs(5))? {
            ControlResponse::ParameterValue { value } => value,
            _ => None,
        }
    }
}

impl crate::bridge::PluginBridge for LockFreeBridge {
    fn process(
        &self,
        num_samples: usize,
        midi_events: crate::protocol::MidiEventVec,
        param_changes: crate::protocol::ParameterChanges,
        note_expression: crate::protocol::NoteExpressionChanges,
        transport: crate::protocol::TransportInfo,
    ) -> bool {
        self.process(
            num_samples,
            midi_events,
            param_changes,
            note_expression,
            transport,
        )
    }

    fn set_parameter_rt(&self, param_id: u32, value: f32) -> bool {
        self.set_parameter_rt(param_id, value)
    }

    fn set_sample_rate_rt(&self, rate: f64) -> bool {
        self.set_sample_rate_rt(rate)
    }

    fn reset_rt(&self) -> bool {
        self.reset_rt()
    }

    fn write_input_channel(&self, channel: usize, data: &[f32]) -> Result<()> {
        self.write_input_channel(channel, data)
    }

    fn read_output_channel_into(&self, channel: usize, output: &mut [f32]) -> Result<usize> {
        self.read_output_channel_into(channel, output)
    }

    fn write_input_channel_f64(&self, channel: usize, data: &[f64]) -> Result<()> {
        self.write_input_channel_f64(channel, data)
    }

    fn read_output_channel_into_f64(&self, channel: usize, output: &mut [f64]) -> Result<usize> {
        self.read_output_channel_into_f64(channel, output)
    }

    fn is_crashed(&self) -> bool {
        self.is_crashed()
    }

    fn open_editor(&self, parent_handle: u64) -> Option<(u32, u32)> {
        self.open_editor(parent_handle)
    }

    fn close_editor(&self) -> bool {
        self.close_editor()
    }

    fn editor_idle(&self) {
        self.editor_idle()
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        self.save_state()
    }

    fn load_state(&self, data: &[u8]) -> bool {
        self.load_state(data)
    }

    fn get_parameter_list(&self) -> Option<Vec<ParameterInfo>> {
        self.get_parameter_list()
    }

    fn get_parameter(&self, param_id: u32) -> Option<f32> {
        self.get_parameter(param_id)
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
