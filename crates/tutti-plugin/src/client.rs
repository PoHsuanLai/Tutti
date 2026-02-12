//! Plugin client for multi-process plugin hosting.

use crate::bridge::PluginBridge;
use crate::error::{BridgeError, LoadStage, Result};
use crate::lockfree_bridge::{BridgeThreadHandle, LockFreeBridge};
use crate::protocol::{BridgeConfig, BridgeMessage, HostMessage, PluginMetadata, SampleFormat};
use crate::shared_memory::SharedAudioBuffer;
use crate::transport::MessageTransport;
use ringbuf::traits::{Consumer, Producer, Split};
use std::cell::UnsafeCell;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use tutti_core::{AudioUnit, BufferMut, BufferRef, SignalFrame, F64};
use tutti_midi_io::MidiEvent;

/// Batch size for tick() accumulation (matches fundsp MAX_BUFFER_SIZE).
const TICK_BATCH_SIZE: usize = 64;

/// Monotonic counter for stable PluginClient IDs that survive cloning.
static NEXT_CLIENT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Pre-allocated scratch buffer for RT-safe audio conversion.
#[derive(Clone)]
struct ScratchBuffer {
    f32_buf: Vec<f32>,
    f64_buf: Vec<f64>,
}

impl ScratchBuffer {
    fn new(max_samples: usize) -> Self {
        Self {
            f32_buf: vec![0.0; max_samples],
            f64_buf: vec![0.0; max_samples],
        }
    }
}

/// Per-channel tick accumulation buffer. Batches single-sample tick() calls.
#[derive(Clone)]
struct TickBuffer<T: Copy + Default> {
    input: Vec<Vec<T>>,
    output: Vec<Vec<T>>,
    write_pos: usize,
    read_pos: usize,
    filled: usize,
}

impl<T: Copy + Default> TickBuffer<T> {
    fn new(num_inputs: usize, num_outputs: usize) -> Self {
        Self {
            input: (0..num_inputs)
                .map(|_| vec![T::default(); TICK_BATCH_SIZE])
                .collect(),
            output: (0..num_outputs)
                .map(|_| vec![T::default(); TICK_BATCH_SIZE])
                .collect(),
            write_pos: 0,
            read_pos: 0,
            filled: 0,
        }
    }

    fn reset(&mut self) {
        self.write_pos = 0;
        self.read_pos = 0;
        self.filled = 0;
    }

    fn fill_output_silence(&mut self, size: usize) {
        for ch in &mut self.output {
            ch[..size].fill(T::default());
        }
        self.filled = size;
        self.write_pos = 0;
        self.read_pos = 0;
    }
}

/// Plugin client for multi-process plugin hosting.
///
/// Communicates with plugin server via lock-free queues. Implements both
/// `AudioUnit` (f32) and `AudioUnit<F64>` for native f64 processing.
///
/// Cloning is cheap - each instance has independent buffers but shares the bridge.
#[derive(Clone)]
pub struct PluginClient {
    /// Stable ID that survives Clone (fundsp clones nodes on commit).
    stable_id: u64,
    bridge: Option<Arc<dyn PluginBridge>>,
    metadata: PluginMetadata,
    inputs: usize,
    outputs: usize,
    negotiated_format: SampleFormat,
    scratch_in: ScratchBuffer,
    scratch_out: ScratchBuffer,
    tick_f32: TickBuffer<f32>,
    tick_f64: TickBuffer<f64>,
    midi_producer: Arc<UnsafeCell<ringbuf::HeapProd<MidiEvent>>>,
    midi_consumer: Arc<UnsafeCell<ringbuf::HeapCons<MidiEvent>>>,
    /// Pre-allocated buffer for RT-safe MIDI event draining
    midi_drain_buffer: crate::protocol::MidiEventVec,
    /// Pull-based MIDI registry (set by engine for graph-routed MIDI)
    midi_registry: Option<tutti_core::MidiRegistry>,
    /// Pre-allocated buffer for polling MIDI registry (RT-safe)
    midi_poll_buffer: Vec<MidiEvent>,
}

// Safety: SPSC queues - producer and consumer never accessed concurrently
unsafe impl Send for PluginClient {}
unsafe impl Sync for PluginClient {}

/// Owner handle for plugin process and bridge thread.
///
/// Cleans up gracefully on drop.
pub struct PluginClientHandle {
    process: Option<Child>,
    #[allow(dead_code)]
    bridge_thread: Option<BridgeThreadHandle>,
    #[allow(dead_code)]
    config: BridgeConfig,
}

impl PluginClient {
    /// Load a plugin and spawn bridge process.
    ///
    /// Returns client (for audio) and handle (for cleanup).
    pub async fn load(
        config: BridgeConfig,
        plugin_path: PathBuf,
        sample_rate: f64,
    ) -> Result<(Self, PluginClientHandle)> {
        let process = Self::spawn_bridge_process(&config)?;
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let mut transport = MessageTransport::connect(&config.socket_path).await?;

        // Consume the server's Ready handshake
        let ready_timeout = std::time::Duration::from_millis(config.timeout_ms);
        match transport.recv_with_timeout(ready_timeout).await? {
            BridgeMessage::Ready => {}
            other => {
                return Err(BridgeError::ProtocolError(format!(
                    "Expected Ready handshake, got: {:?}",
                    other
                )));
            }
        }

        // Create shared memory BEFORE sending LoadPlugin so the server can open it.
        // We use conservative defaults (2ch stereo, max buffer size) since we don't
        // know the plugin's channel count yet. The buffer is oversized but safe.
        let shm_name = format!("dawai_plugin_{}", std::process::id());
        let pre_channels = 2; // Will be validated after metadata arrives
        let pre_audio_buffer = SharedAudioBuffer::create_with_format(
            shm_name.clone(),
            pre_channels,
            config.max_buffer_size,
            config.preferred_format,
        )
        .map_err(|e| BridgeError::Io(std::io::Error::other(e)))?;

        let metadata =
            Self::load_plugin_on_server(&mut transport, &config, &plugin_path, sample_rate, &shm_name).await?;

        let negotiated_format = Self::negotiate_format(config.preferred_format, &metadata);

        // Re-create shared memory if channel count differs from pre-allocated
        let num_channels = metadata.audio_io.inputs.max(metadata.audio_io.outputs);
        let audio_buffer = if num_channels != pre_channels || negotiated_format != config.preferred_format {
            drop(pre_audio_buffer);
            Arc::new(
                SharedAudioBuffer::create_with_format(
                    shm_name.clone(),
                    num_channels,
                    config.max_buffer_size,
                    negotiated_format,
                )
                .map_err(|e| BridgeError::Io(std::io::Error::other(e)))?,
            )
        } else {
            Arc::new(pre_audio_buffer)
        };

        let (bridge, bridge_thread) = LockFreeBridge::new(transport, audio_buffer)?;
        let bridge_arc: Arc<dyn PluginBridge> = Arc::new(bridge);

        let client = Self::create_client(bridge_arc, *metadata, &config, negotiated_format);
        let handle = PluginClientHandle {
            process: Some(process),
            bridge_thread: Some(bridge_thread),
            config,
        };

        Ok((client, handle))
    }

    pub fn latency_samples(&self) -> usize {
        self.metadata.latency_samples
    }

    pub fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    /// RT-safe.
    pub fn set_parameter(&self, param_id: u32, value: f32) {
        if let Some(bridge) = &self.bridge {
            let _ = bridge.set_parameter_rt(param_id, value);
        }
    }

    pub fn sample_format(&self) -> SampleFormat {
        self.negotiated_format
    }

    /// Returns true if the plugin server process has crashed.
    ///
    /// When this returns true, all audio processing calls produce silence.
    /// The UI should poll this to show "Plugin crashed" and offer reload.
    pub fn is_crashed(&self) -> bool {
        self.bridge.as_ref().is_some_and(|b| b.is_crashed())
    }

    // =========================================================================
    // Non-RT control methods (main thread only)
    // =========================================================================

    /// Open the plugin editor GUI. Returns (width, height) on success.
    pub fn open_editor(&self, parent_handle: u64) -> Option<(u32, u32)> {
        self.bridge.as_ref()?.open_editor(parent_handle)
    }

    /// Close the plugin editor GUI.
    pub fn close_editor(&self) -> bool {
        self.bridge
            .as_ref()
            .map(|b| b.close_editor())
            .unwrap_or(false)
    }

    /// Tick the plugin editor idle loop.
    pub fn editor_idle(&self) {
        if let Some(bridge) = &self.bridge {
            bridge.editor_idle();
        }
    }

    /// Save the plugin state. Returns the state bytes on success.
    pub fn save_state(&self) -> Option<Vec<u8>> {
        self.bridge.as_ref()?.save_state()
    }

    /// Load plugin state from bytes.
    pub fn load_state(&self, data: &[u8]) -> bool {
        self.bridge
            .as_ref()
            .map(|b| b.load_state(data))
            .unwrap_or(false)
    }

    /// Get the full parameter list from the plugin.
    pub fn get_parameter_list(&self) -> Option<Vec<crate::protocol::ParameterInfo>> {
        self.bridge.as_ref()?.get_parameter_list()
    }

    /// Get a single parameter value.
    pub fn get_parameter_value(&self, param_id: u32) -> Option<f32> {
        self.bridge.as_ref()?.get_parameter(param_id)
    }

    /// Set the MIDI registry for pull-based MIDI delivery from the engine.
    ///
    /// When set, the plugin client polls the registry during audio processing
    /// to receive MIDI events routed via `engine.note_on()` / `engine.note_off()`.
    pub fn set_midi_registry(&mut self, registry: tutti_core::MidiRegistry) {
        self.midi_registry = Some(registry);
    }

    /// Get an Arc clone of the bridge (for PluginHandle construction).
    pub(crate) fn bridge_arc(&self) -> Option<Arc<dyn PluginBridge>> {
        self.bridge.as_ref().map(Arc::clone)
    }

    /// Create a PluginClient from an in-process bridge.
    ///
    /// Used by the in-process plugin loading path where no child process or
    /// BridgeConfig is involved.
    pub fn from_bridge(
        bridge: Arc<dyn PluginBridge>,
        metadata: PluginMetadata,
        max_buffer_size: usize,
    ) -> Self {
        let (midi_prod, midi_cons) = ringbuf::HeapRb::<MidiEvent>::new(512).split();
        #[allow(clippy::arc_with_non_send_sync)]
        let midi_producer = Arc::new(UnsafeCell::new(midi_prod));
        #[allow(clippy::arc_with_non_send_sync)]
        let midi_consumer = Arc::new(UnsafeCell::new(midi_cons));

        let inputs = metadata.audio_io.inputs;
        let outputs = metadata.audio_io.outputs;

        Self {
            stable_id: NEXT_CLIENT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            bridge: Some(bridge),
            metadata,
            inputs,
            outputs,
            negotiated_format: SampleFormat::Float32,
            scratch_in: ScratchBuffer::new(max_buffer_size),
            scratch_out: ScratchBuffer::new(max_buffer_size),
            tick_f32: TickBuffer::new(inputs, outputs),
            tick_f64: TickBuffer::new(inputs, outputs),
            midi_producer,
            midi_consumer,
            midi_drain_buffer: smallvec::SmallVec::new(),
            midi_registry: None,
            midi_poll_buffer: vec![MidiEvent::note_on_builder(0, 0).build(); 256],
        }
    }
}

impl PluginClient {
    fn spawn_bridge_process(config: &BridgeConfig) -> Result<Child> {
        let exe_dir = std::env::current_exe()
            .and_then(|p| {
                p.parent()
                    .map(|d| d.to_path_buf())
                    .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no parent"))
            })
            .map_err(BridgeError::Io)?;

        // Search: same directory, then parent (handles examples/ subdirectory)
        let candidate = exe_dir.join("plugin-server");
        let bridge_path = if candidate.exists() {
            candidate
        } else if let Some(parent) = exe_dir.parent() {
            let parent_candidate = parent.join("plugin-server");
            if parent_candidate.exists() {
                parent_candidate
            } else {
                candidate // will fail with a clear error
            }
        } else {
            candidate
        };

        Command::new(bridge_path)
            .arg(&config.socket_path)
            .spawn()
            .map_err(BridgeError::Io)
    }

    async fn load_plugin_on_server(
        transport: &mut MessageTransport,
        config: &BridgeConfig,
        plugin_path: &Path,
        sample_rate: f64,
        shm_name: &str,
    ) -> Result<Box<PluginMetadata>> {
        transport
            .send_host_message(&HostMessage::LoadPlugin {
                path: plugin_path.to_path_buf(),
                sample_rate,
                block_size: config.max_buffer_size,
                preferred_format: config.preferred_format,
                shm_name: shm_name.to_string(),
            })
            .await?;

        let timeout = std::time::Duration::from_millis(config.timeout_ms);
        let response = transport.recv_with_timeout(timeout).await?;

        match response {
            BridgeMessage::PluginLoaded { metadata } => Ok(metadata),
            BridgeMessage::Error { message } => Err(BridgeError::LoadFailed {
                path: plugin_path.to_path_buf(),
                stage: LoadStage::Opening,
                reason: message,
            }),
            other => Err(BridgeError::ProtocolError(format!(
                "Unexpected response: {:?}",
                other
            ))),
        }
    }

    pub(crate) fn negotiate_format(preferred: SampleFormat, metadata: &PluginMetadata) -> SampleFormat {
        if preferred == SampleFormat::Float64 && metadata.supports_f64 {
            SampleFormat::Float64
        } else {
            SampleFormat::Float32
        }
    }

    fn create_client(
        bridge: Arc<dyn PluginBridge>,
        metadata: PluginMetadata,
        config: &BridgeConfig,
        format: SampleFormat,
    ) -> Self {
        let (midi_prod, midi_cons) = ringbuf::HeapRb::<MidiEvent>::new(512).split();
        #[allow(clippy::arc_with_non_send_sync)]
        let midi_producer = Arc::new(UnsafeCell::new(midi_prod));
        #[allow(clippy::arc_with_non_send_sync)]
        let midi_consumer = Arc::new(UnsafeCell::new(midi_cons));

        let inputs = metadata.audio_io.inputs;
        let outputs = metadata.audio_io.outputs;

        Self {
            stable_id: NEXT_CLIENT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            bridge: Some(bridge),
            metadata,
            inputs,
            outputs,
            negotiated_format: format,
            scratch_in: ScratchBuffer::new(config.max_buffer_size),
            scratch_out: ScratchBuffer::new(config.max_buffer_size),
            tick_f32: TickBuffer::new(inputs, outputs),
            tick_f64: TickBuffer::new(inputs, outputs),
            midi_producer,
            midi_consumer,
            midi_drain_buffer: smallvec::SmallVec::new(),
            midi_registry: None,
            midi_poll_buffer: vec![MidiEvent::note_on_builder(0, 0).build(); 256],
        }
    }
}

#[cfg(test)]
impl PluginClient {
    pub(crate) fn with_no_bridge(
        metadata: PluginMetadata,
        max_buffer_size: usize,
        format: SampleFormat,
    ) -> Self {
        let inputs = metadata.audio_io.inputs;
        let outputs = metadata.audio_io.outputs;
        let (midi_prod, midi_cons) = ringbuf::HeapRb::<MidiEvent>::new(512).split();
        #[allow(clippy::arc_with_non_send_sync)]
        let midi_producer = Arc::new(UnsafeCell::new(midi_prod));
        #[allow(clippy::arc_with_non_send_sync)]
        let midi_consumer = Arc::new(UnsafeCell::new(midi_cons));

        Self {
            stable_id: NEXT_CLIENT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            bridge: None,
            metadata,
            inputs,
            outputs,
            negotiated_format: format,
            scratch_in: ScratchBuffer::new(max_buffer_size),
            scratch_out: ScratchBuffer::new(max_buffer_size),
            tick_f32: TickBuffer::new(inputs, outputs),
            tick_f64: TickBuffer::new(inputs, outputs),
            midi_producer,
            midi_consumer,
            midi_drain_buffer: smallvec::SmallVec::new(),
            midi_registry: None,
            midi_poll_buffer: vec![MidiEvent::note_on_builder(0, 0).build(); 256],
        }
    }
}

impl PluginClient {
    fn drain_midi_events(&mut self) {
        let cons = unsafe { &mut *self.midi_consumer.get() };
        self.midi_drain_buffer.clear();
        while let Some(event) = cons.try_pop() {
            self.midi_drain_buffer.push(event);
        }

        // Also poll the MIDI registry for events routed via engine.note_on()
        if let Some(ref registry) = self.midi_registry {
            let unit_id = self.stable_id;
            let count = registry.poll_into(unit_id, &mut self.midi_poll_buffer);
            for i in 0..count {
                self.midi_drain_buffer.push(self.midi_poll_buffer[i]);
            }
        }
    }

    fn fill_silence<F>(size: usize, outputs: usize, mut set_output: F)
    where
        F: FnMut(usize, usize, f64),
    {
        for i in 0..size {
            for ch in 0..outputs {
                set_output(ch, i, 0.0);
            }
        }
    }

    fn process_impl<GetIn, SetOut>(&mut self, size: usize, get_input: GetIn, mut set_output: SetOut)
    where
        GetIn: Fn(usize, usize) -> f64,
        SetOut: FnMut(usize, usize, f64),
    {
        self.drain_midi_events();

        let bridge = match self.bridge.as_ref() {
            Some(b) => b,
            None => {
                Self::fill_silence(size, self.outputs, set_output);
                return;
            }
        };

        let write_ok = match self.negotiated_format {
            SampleFormat::Float64 => {
                let mut ok = true;
                for ch in 0..self.inputs {
                    for i in 0..size {
                        self.scratch_in.f64_buf[i] = get_input(ch, i);
                    }
                    if bridge
                        .write_input_channel_f64(ch, &self.scratch_in.f64_buf[..size])
                        .is_err()
                    {
                        ok = false;
                        break;
                    }
                }
                ok
            }
            SampleFormat::Float32 => {
                let mut ok = true;
                for ch in 0..self.inputs {
                    for i in 0..size {
                        self.scratch_in.f32_buf[i] = get_input(ch, i) as f32;
                    }
                    if bridge
                        .write_input_channel(ch, &self.scratch_in.f32_buf[..size])
                        .is_err()
                    {
                        ok = false;
                        break;
                    }
                }
                ok
            }
        };

        if !write_ok {
            Self::fill_silence(size, self.outputs, set_output);
            return;
        }

        let midi_events: crate::protocol::MidiEventVec =
            self.midi_drain_buffer.iter().copied().collect();
        if !bridge.process(
            size,
            midi_events,
            crate::protocol::ParameterChanges::new(),
            crate::protocol::NoteExpressionChanges::new(),
            crate::protocol::TransportInfo::default(),
        ) {
            Self::fill_silence(size, self.outputs, set_output);
            return;
        }

        match self.negotiated_format {
            SampleFormat::Float64 => {
                for ch in 0..self.outputs {
                    let n = bridge
                        .read_output_channel_into_f64(ch, &mut self.scratch_out.f64_buf[..size])
                        .unwrap_or(0);
                    for i in 0..n {
                        set_output(ch, i, self.scratch_out.f64_buf[i]);
                    }
                    for i in n..size {
                        set_output(ch, i, 0.0);
                    }
                }
            }
            SampleFormat::Float32 => {
                for ch in 0..self.outputs {
                    let n = bridge
                        .read_output_channel_into(ch, &mut self.scratch_out.f32_buf[..size])
                        .unwrap_or(0);
                    for i in 0..n {
                        set_output(ch, i, self.scratch_out.f32_buf[i] as f64);
                    }
                    for i in n..size {
                        set_output(ch, i, 0.0);
                    }
                }

                // Debug: log peak output from client side
                {
                    static LOG_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                    let count = LOG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if count < 10 {
                        let peak: f32 = self.scratch_out.f32_buf[..size]
                            .iter()
                            .fold(0.0f32, |a, &b| a.max(b.abs()));
                        eprintln!(
                            "[client] read_output: size={}, outputs={}, peak={:.6}",
                            size, self.outputs, peak
                        );
                    }
                }
            }
        }
    }
}

impl PluginClient {
    fn flush_tick_f32(&mut self) {
        let size = self.tick_f32.write_pos;
        if size == 0 {
            return;
        }

        self.drain_midi_events();

        if self.bridge.is_none() {
            self.tick_f32.fill_output_silence(size);
            return;
        }

        let write_ok = match self.negotiated_format {
            SampleFormat::Float64 => {
                let mut ok = true;
                for ch in 0..self.inputs {
                    for i in 0..size {
                        self.scratch_in.f64_buf[i] = self.tick_f32.input[ch][i] as f64;
                    }
                    if self
                        .bridge
                        .as_ref()
                        .unwrap()
                        .write_input_channel_f64(ch, &self.scratch_in.f64_buf[..size])
                        .is_err()
                    {
                        ok = false;
                        break;
                    }
                }
                ok
            }
            SampleFormat::Float32 => {
                let mut ok = true;
                for ch in 0..self.inputs {
                    for i in 0..size {
                        self.scratch_in.f32_buf[i] = self.tick_f32.input[ch][i];
                    }
                    if self
                        .bridge
                        .as_ref()
                        .unwrap()
                        .write_input_channel(ch, &self.scratch_in.f32_buf[..size])
                        .is_err()
                    {
                        ok = false;
                        break;
                    }
                }
                ok
            }
        };

        if !write_ok {
            self.tick_f32.fill_output_silence(size);
            return;
        }

        let midi_events: crate::protocol::MidiEventVec =
            self.midi_drain_buffer.iter().copied().collect();
        if !self.bridge.as_ref().unwrap().process(
            size,
            midi_events,
            crate::protocol::ParameterChanges::new(),
            crate::protocol::NoteExpressionChanges::new(),
            crate::protocol::TransportInfo::default(),
        ) {
            self.tick_f32.fill_output_silence(size);
            return;
        }

        match self.negotiated_format {
            SampleFormat::Float64 => {
                for ch in 0..self.outputs {
                    let n = self
                        .bridge
                        .as_ref()
                        .unwrap()
                        .read_output_channel_into_f64(ch, &mut self.scratch_out.f64_buf[..size])
                        .unwrap_or(0);
                    for i in 0..n {
                        self.tick_f32.output[ch][i] = self.scratch_out.f64_buf[i] as f32;
                    }
                    for i in n..size {
                        self.tick_f32.output[ch][i] = 0.0;
                    }
                }
            }
            SampleFormat::Float32 => {
                for ch in 0..self.outputs {
                    let n = self
                        .bridge
                        .as_ref()
                        .unwrap()
                        .read_output_channel_into(ch, &mut self.scratch_out.f32_buf[..size])
                        .unwrap_or(0);
                    for i in 0..n {
                        self.tick_f32.output[ch][i] = self.scratch_out.f32_buf[i];
                    }
                    for i in n..size {
                        self.tick_f32.output[ch][i] = 0.0;
                    }
                }
            }
        }

        self.tick_f32.filled = size;
        self.tick_f32.write_pos = 0;
        self.tick_f32.read_pos = 0;
    }

    fn flush_tick_f64(&mut self) {
        let size = self.tick_f64.write_pos;
        if size == 0 {
            return;
        }

        self.drain_midi_events();

        if self.bridge.is_none() {
            self.tick_f64.fill_output_silence(size);
            return;
        }

        let write_ok = match self.negotiated_format {
            SampleFormat::Float64 => {
                let mut ok = true;
                for ch in 0..self.inputs {
                    for i in 0..size {
                        self.scratch_in.f64_buf[i] = self.tick_f64.input[ch][i];
                    }
                    if self
                        .bridge
                        .as_ref()
                        .unwrap()
                        .write_input_channel_f64(ch, &self.scratch_in.f64_buf[..size])
                        .is_err()
                    {
                        ok = false;
                        break;
                    }
                }
                ok
            }
            SampleFormat::Float32 => {
                let mut ok = true;
                for ch in 0..self.inputs {
                    for i in 0..size {
                        self.scratch_in.f32_buf[i] = self.tick_f64.input[ch][i] as f32;
                    }
                    if self
                        .bridge
                        .as_ref()
                        .unwrap()
                        .write_input_channel(ch, &self.scratch_in.f32_buf[..size])
                        .is_err()
                    {
                        ok = false;
                        break;
                    }
                }
                ok
            }
        };

        if !write_ok {
            self.tick_f64.fill_output_silence(size);
            return;
        }

        let midi_events: crate::protocol::MidiEventVec =
            self.midi_drain_buffer.iter().copied().collect();
        if !self.bridge.as_ref().unwrap().process(
            size,
            midi_events,
            crate::protocol::ParameterChanges::new(),
            crate::protocol::NoteExpressionChanges::new(),
            crate::protocol::TransportInfo::default(),
        ) {
            self.tick_f64.fill_output_silence(size);
            return;
        }

        match self.negotiated_format {
            SampleFormat::Float64 => {
                for ch in 0..self.outputs {
                    let n = self
                        .bridge
                        .as_ref()
                        .unwrap()
                        .read_output_channel_into_f64(ch, &mut self.scratch_out.f64_buf[..size])
                        .unwrap_or(0);
                    for i in 0..n {
                        self.tick_f64.output[ch][i] = self.scratch_out.f64_buf[i];
                    }
                    for i in n..size {
                        self.tick_f64.output[ch][i] = 0.0;
                    }
                }
            }
            SampleFormat::Float32 => {
                for ch in 0..self.outputs {
                    let n = self
                        .bridge
                        .as_ref()
                        .unwrap()
                        .read_output_channel_into(ch, &mut self.scratch_out.f32_buf[..size])
                        .unwrap_or(0);
                    for i in 0..n {
                        self.tick_f64.output[ch][i] = self.scratch_out.f32_buf[i] as f64;
                    }
                    for i in n..size {
                        self.tick_f64.output[ch][i] = 0.0;
                    }
                }
            }
        }

        self.tick_f64.filled = size;
        self.tick_f64.write_pos = 0;
        self.tick_f64.read_pos = 0;
    }
}

impl AudioUnit for PluginClient {
    fn inputs(&self) -> usize {
        self.inputs
    }

    fn outputs(&self) -> usize {
        self.outputs
    }

    fn reset(&mut self) {
        self.tick_f32.reset();
        self.tick_f64.reset();
        if let Some(bridge) = &self.bridge {
            let _ = bridge.reset_rt();
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.tick_f32.reset();
        self.tick_f64.reset();
        if let Some(bridge) = &self.bridge {
            let _ = bridge.set_sample_rate_rt(sample_rate);
        }
    }

    fn tick(&mut self, input: &[f32], output: &mut [f32]) {
        for (ch, &sample) in input.iter().enumerate().take(self.inputs) {
            self.tick_f32.input[ch][self.tick_f32.write_pos] = sample;
        }
        self.tick_f32.write_pos += 1;

        if self.tick_f32.write_pos >= TICK_BATCH_SIZE {
            self.flush_tick_f32();
        }

        if self.tick_f32.read_pos < self.tick_f32.filled {
            for ch in 0..output.len().min(self.outputs) {
                output[ch] = self.tick_f32.output[ch][self.tick_f32.read_pos];
            }
            for sample in output.iter_mut().skip(self.outputs) {
                *sample = 0.0;
            }
        } else {
            output.fill(0.0);
        }
        self.tick_f32.read_pos += 1;
    }

    fn process(&mut self, size: usize, input: &BufferRef, output: &mut BufferMut) {
        self.process_impl(
            size,
            |ch, i| input.at_f32(ch, i) as f64,
            |ch, i, v| output.set_f32(ch, i, v as f32),
        );
    }

    fn get_id(&self) -> u64 {
        self.stable_id
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn route(&mut self, _input: &SignalFrame, _frequency: f64) -> SignalFrame {
        SignalFrame::new(self.outputs)
    }

    fn footprint(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

impl AudioUnit<F64> for PluginClient {
    fn inputs(&self) -> usize {
        self.inputs
    }

    fn outputs(&self) -> usize {
        self.outputs
    }

    fn reset(&mut self) {
        self.tick_f32.reset();
        self.tick_f64.reset();
        if let Some(bridge) = &self.bridge {
            let _ = bridge.reset_rt();
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.tick_f32.reset();
        self.tick_f64.reset();
        if let Some(bridge) = &self.bridge {
            let _ = bridge.set_sample_rate_rt(sample_rate);
        }
    }

    fn tick(&mut self, input: &[f64], output: &mut [f64]) {
        for (ch, &sample) in input.iter().enumerate().take(self.inputs) {
            self.tick_f64.input[ch][self.tick_f64.write_pos] = sample;
        }
        self.tick_f64.write_pos += 1;

        if self.tick_f64.write_pos >= TICK_BATCH_SIZE {
            self.flush_tick_f64();
        }

        if self.tick_f64.read_pos < self.tick_f64.filled {
            for ch in 0..output.len().min(self.outputs) {
                output[ch] = self.tick_f64.output[ch][self.tick_f64.read_pos];
            }
            for sample in output.iter_mut().skip(self.outputs) {
                *sample = 0.0;
            }
        } else {
            output.fill(0.0);
        }
        self.tick_f64.read_pos += 1;
    }

    fn process(&mut self, size: usize, input: &BufferRef<F64>, output: &mut BufferMut<F64>) {
        self.process_impl(
            size,
            |ch, i| input.at_scalar(ch, i),
            |ch, i, v| output.set_scalar(ch, i, v),
        );
    }

    fn get_id(&self) -> u64 {
        self.stable_id
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn route(&mut self, _input: &SignalFrame, _frequency: f64) -> SignalFrame {
        SignalFrame::new(self.outputs)
    }

    fn footprint(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

impl PluginClient {
    /// Queue MIDI events to be sent to the plugin server.
    ///
    /// Events are buffered and sent during the next audio processing call.
    pub fn queue_midi(&mut self, events: &[MidiEvent]) {
        let prod = unsafe { &mut *self.midi_producer.get() };
        let cons = unsafe { &mut *self.midi_consumer.get() };

        // Clear any stale events
        while cons.try_pop().is_some() {}

        for event in events {
            let _ = prod.try_push(*event);
        }
    }

    pub fn clear_midi(&mut self) {
        let cons = unsafe { &mut *self.midi_consumer.get() };
        while cons.try_pop().is_some() {}
    }
}

impl Drop for PluginClientHandle {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            let _ = process.kill();
            let _ = process.wait();
        }
        // Clean up socket file (server may not have cleaned up if it crashed)
        let _ = std::fs::remove_file(&self.config.socket_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::SampleFormat;
    use tutti_core::F64;

    fn test_client(inputs: usize, outputs: usize) -> PluginClient {
        let metadata = PluginMetadata::new("test.plugin", "Test Plugin")
            .audio_io(inputs, outputs)
            .latency(128);
        PluginClient::with_no_bridge(metadata, 512, SampleFormat::Float32)
    }

    fn test_client_f64() -> PluginClient {
        let metadata = PluginMetadata::new("test.plugin", "Test Plugin")
            .audio_io(2, 2)
            .f64_support(true);
        PluginClient::with_no_bridge(metadata, 512, SampleFormat::Float64)
    }

    // --- AudioUnit f32 ---

    #[test]
    fn test_no_bridge_tick_produces_silence() {
        let mut client = test_client(2, 2);
        let input = [0.5_f32, 0.8];
        let mut output = [1.0_f32, 1.0];
        <PluginClient as AudioUnit>::tick(&mut client, &input, &mut output);
        assert_eq!(output, [0.0, 0.0]);
    }

    #[test]
    fn test_no_bridge_tick_batch_flush() {
        let mut client = test_client(1, 1);
        for _ in 0..TICK_BATCH_SIZE {
            let input = [0.5_f32];
            let mut output = [1.0_f32];
            <PluginClient as AudioUnit>::tick(&mut client, &input, &mut output);
        }
        let input = [0.5_f32];
        let mut output = [1.0_f32];
        <PluginClient as AudioUnit>::tick(&mut client, &input, &mut output);
        assert_eq!(output, [0.0]);
    }

    #[test]
    fn test_inputs_outputs() {
        let client = test_client(3, 4);
        assert_eq!(<PluginClient as AudioUnit>::inputs(&client), 3);
        assert_eq!(<PluginClient as AudioUnit>::outputs(&client), 4);
    }

    #[test]
    fn test_reset_no_panic() {
        let mut client = test_client(2, 2);
        <PluginClient as AudioUnit>::reset(&mut client);
    }

    #[test]
    fn test_set_sample_rate_no_panic() {
        let mut client = test_client(2, 2);
        <PluginClient as AudioUnit>::set_sample_rate(&mut client, 96000.0);
    }

    #[test]
    fn test_get_id_unique() {
        let client1 = test_client(2, 2);
        let client2 = test_client(2, 2);
        assert_ne!(
            <PluginClient as AudioUnit>::get_id(&client1),
            <PluginClient as AudioUnit>::get_id(&client2),
        );
    }

    #[test]
    fn test_footprint_nonzero() {
        let client = test_client(2, 2);
        assert!(<PluginClient as AudioUnit>::footprint(&client) > 0);
    }

    // --- AudioUnit f64 ---

    #[test]
    fn test_no_bridge_tick_f64_produces_silence() {
        let mut client = test_client_f64();
        let input = [0.5_f64, 0.8];
        let mut output = [1.0_f64, 1.0];
        <PluginClient as AudioUnit<F64>>::tick(&mut client, &input, &mut output);
        assert_eq!(output, [0.0, 0.0]);
    }

    #[test]
    fn test_no_bridge_tick_f64_batch_flush() {
        let mut client = test_client_f64();
        for _ in 0..TICK_BATCH_SIZE {
            let input = [0.5_f64, 0.5];
            let mut output = [1.0_f64, 1.0];
            <PluginClient as AudioUnit<F64>>::tick(&mut client, &input, &mut output);
        }
        let input = [0.5_f64, 0.5];
        let mut output = [1.0_f64, 1.0];
        <PluginClient as AudioUnit<F64>>::tick(&mut client, &input, &mut output);
        assert_eq!(output, [0.0, 0.0]);
    }

    // --- MIDI ---

    #[test]
    fn test_queue_midi_no_panic() {
        let mut client = test_client(0, 2);
        let events = vec![
            MidiEvent::note_on_builder(60, 100).channel(0).offset(0).build(),
        ];
        client.queue_midi(&events);
    }

    #[test]
    fn test_clear_midi_no_panic() {
        let mut client = test_client(0, 2);
        client.clear_midi();
    }

    #[test]
    fn test_queue_and_clear_midi() {
        let mut client = test_client(0, 2);
        let events = vec![
            MidiEvent::note_on_builder(60, 100).channel(0).offset(0).build(),
            MidiEvent::note_on_builder(64, 80).channel(0).offset(0).build(),
        ];
        client.queue_midi(&events);
        client.clear_midi();
        let mut output = [1.0_f32, 1.0];
        <PluginClient as AudioUnit>::tick(&mut client, &[], &mut output);
        assert_eq!(output, [0.0, 0.0]);
    }

    // --- Accessors ---

    #[test]
    fn test_metadata_preserved() {
        let client = test_client(2, 2);
        assert_eq!(client.metadata().id, "test.plugin");
        assert_eq!(client.metadata().name, "Test Plugin");
        assert_eq!(client.latency_samples(), 128);
        assert_eq!(client.sample_format(), SampleFormat::Float32);
    }

    #[test]
    fn test_set_parameter_no_bridge_no_panic() {
        let client = test_client(2, 2);
        client.set_parameter(0, 0.5);
        client.set_parameter(999, 1.0);
    }

    // --- Format negotiation ---

    #[test]
    fn test_negotiate_f64_when_supported() {
        let meta = PluginMetadata::new("test", "Test").f64_support(true);
        assert_eq!(
            PluginClient::negotiate_format(SampleFormat::Float64, &meta),
            SampleFormat::Float64
        );
    }

    #[test]
    fn test_negotiate_fallback_f32() {
        let meta = PluginMetadata::new("test", "Test").f64_support(false);
        assert_eq!(
            PluginClient::negotiate_format(SampleFormat::Float64, &meta),
            SampleFormat::Float32
        );
    }

    #[test]
    fn test_negotiate_f32_preferred() {
        let meta = PluginMetadata::new("test", "Test").f64_support(true);
        assert_eq!(
            PluginClient::negotiate_format(SampleFormat::Float32, &meta),
            SampleFormat::Float32
        );
    }

    // --- Crash detection ---

    #[test]
    fn test_is_crashed_no_bridge() {
        let client = test_client(2, 2);
        assert!(!client.is_crashed());
    }

    #[test]
    fn test_is_crashed_f64_no_bridge() {
        let client = test_client_f64();
        assert!(!client.is_crashed());
    }

    /// Create a LockFreeBridge connected to a UnixStream pair.
    /// Returns (bridge, bridge_thread_handle, server_side_stream).
    /// Dropping the server_side_stream simulates a server crash.
    fn test_bridge_with_mock_server() -> (
        crate::lockfree_bridge::LockFreeBridge,
        crate::lockfree_bridge::BridgeThreadHandle,
        tokio::net::UnixStream,
    ) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (client_stream, server_stream) = tokio::net::UnixStream::pair().unwrap();
            let transport = crate::transport::MessageTransport::from_stream(client_stream);

            let name = format!("test_crash_{}", std::process::id());
            let buffer = std::sync::Arc::new(
                crate::shared_memory::SharedAudioBuffer::create(name, 2, 512).unwrap(),
            );

            let (bridge, handle) =
                crate::lockfree_bridge::LockFreeBridge::new(transport, buffer).unwrap();

            (bridge, handle, server_stream)
        })
    }

    #[test]
    fn test_crash_detected_when_server_drops() {
        let (bridge, _handle, server_stream) = test_bridge_with_mock_server();

        // Initially not crashed
        assert!(!bridge.is_crashed());

        // Drop server side — simulates server process crash
        drop(server_stream);

        // Send a process command to trigger the bridge thread to hit the broken pipe
        let sent = bridge.process(
            64,
            smallvec::SmallVec::new(),
            crate::protocol::ParameterChanges::new(),
            crate::protocol::NoteExpressionChanges::new(),
            crate::protocol::TransportInfo::default(),
        );

        // process() returns false (no response from dead server)
        assert!(!sent);

        // Give the bridge thread time to detect the crash
        std::thread::sleep(std::time::Duration::from_millis(200));

        // Now the crashed flag should be set
        assert!(bridge.is_crashed());

        // Subsequent calls should fail fast without touching the queue
        assert!(!bridge.set_parameter_rt(0, 0.5));
        assert!(!bridge.set_sample_rate_rt(48000.0));
        assert!(!bridge.reset_rt());
        assert!(!bridge.process(
            64,
            smallvec::SmallVec::new(),
            crate::protocol::ParameterChanges::new(),
            crate::protocol::NoteExpressionChanges::new(),
            crate::protocol::TransportInfo::default(),
        ));
    }

    #[test]
    fn test_client_is_crashed_with_real_bridge() {
        let (bridge, _handle, server_stream) = test_bridge_with_mock_server();

        let metadata = PluginMetadata::new("test.plugin", "Test Plugin")
            .audio_io(2, 2)
            .latency(0);
        let config = crate::protocol::BridgeConfig::default();

        let bridge_arc: Arc<dyn crate::bridge::PluginBridge> = Arc::new(bridge.clone());
        let client = PluginClient::create_client(bridge_arc, metadata, &config, SampleFormat::Float32);

        assert!(!client.is_crashed());

        // Simulate crash
        drop(server_stream);

        // Trigger bridge thread to detect the crash by sending a process command
        // directly via the bridge (tick() batches and won't flush until TICK_BATCH_SIZE)
        bridge.process(
            64,
            smallvec::SmallVec::new(),
            crate::protocol::ParameterChanges::new(),
            crate::protocol::NoteExpressionChanges::new(),
            crate::protocol::TransportInfo::default(),
        );

        std::thread::sleep(std::time::Duration::from_millis(200));

        assert!(client.is_crashed());
    }

    #[test]
    fn test_crash_produces_silence() {
        let (bridge, _handle, server_stream) = test_bridge_with_mock_server();

        let metadata = PluginMetadata::new("test.plugin", "Test Plugin")
            .audio_io(2, 2)
            .latency(0);
        let config = crate::protocol::BridgeConfig::default();

        let bridge_arc: Arc<dyn crate::bridge::PluginBridge> = Arc::new(bridge);
        let mut client =
            PluginClient::create_client(bridge_arc, metadata, &config, SampleFormat::Float32);

        // Kill the server
        drop(server_stream);
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Process should produce silence, not panic
        for _ in 0..TICK_BATCH_SIZE * 2 {
            let mut out = [999.0f32; 2];
            <PluginClient as tutti_core::AudioUnit>::tick(&mut client, &[0.5, 0.5], &mut out);
            // Output should be 0.0 (silence), not the initial 999.0
            assert!(
                out[0].abs() < f32::EPSILON && out[1].abs() < f32::EPSILON,
                "Expected silence, got {:?}",
                out
            );
        }
    }

    // =========================================================================
    // PluginHandle tests (with responding mock server)
    // =========================================================================

    /// Read a HostMessage from a std UnixStream (blocking).
    fn recv_host_msg_sync(stream: &mut std::os::unix::net::UnixStream) -> HostMessage {
        use std::io::Read;
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).unwrap();
        bincode::deserialize(&buf).unwrap()
    }

    /// Send a BridgeMessage to a std UnixStream (blocking).
    fn send_bridge_msg_sync(
        stream: &mut std::os::unix::net::UnixStream,
        msg: &crate::protocol::BridgeMessage,
    ) {
        use std::io::Write;
        let data = bincode::serialize(msg).unwrap();
        let len = (data.len() as u32).to_be_bytes();
        stream.write_all(&len).unwrap();
        stream.write_all(&data).unwrap();
    }

    /// Unique shared memory name per test to avoid collisions.
    fn unique_shm_name(label: &str) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        format!(
            "test_{}_{}_{}",
            label,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }

    /// Create a bridge + mock server that responds to control commands.
    ///
    /// Uses std::os::unix::net::UnixStream for the mock server side (blocking I/O
    /// on a dedicated thread). The client side is converted to tokio inside the
    /// bridge thread's own runtime via `new_from_std_stream`, avoiding
    /// cross-runtime IO registration issues.
    fn setup_handle_with_mock_server(
        respond: impl Fn(HostMessage) -> Option<crate::protocol::BridgeMessage> + Send + 'static,
    ) -> (
        crate::handle::PluginHandle,
        crate::lockfree_bridge::BridgeThreadHandle,
        std::thread::JoinHandle<()>,
    ) {
        // Create std UnixStream pair (not tied to any tokio runtime)
        let (client_std, mut server_std) = std::os::unix::net::UnixStream::pair().unwrap();
        client_std.set_nonblocking(true).unwrap();

        // Spawn mock server on a std thread (blocking I/O)
        let server_thread = std::thread::Builder::new()
            .name("mock-plugin-server".to_string())
            .spawn(move || {
                server_std
                    .set_read_timeout(Some(std::time::Duration::from_secs(30)))
                    .unwrap();
                loop {
                    let msg = match std::panic::catch_unwind(
                        std::panic::AssertUnwindSafe(|| recv_host_msg_sync(&mut server_std)),
                    ) {
                        Ok(msg) => msg,
                        Err(_) => break, // stream closed or error
                    };
                    if let Some(response) = respond(msg) {
                        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            send_bridge_msg_sync(&mut server_std, &response)
                        }))
                        .is_err()
                        {
                            break;
                        }
                    }
                }
            })
            .unwrap();

        // Use new_from_std_stream: the bridge thread converts the std stream
        // to tokio inside its own runtime, so IO registration is correct.
        let name = unique_shm_name("handle");
        let buffer = std::sync::Arc::new(
            crate::shared_memory::SharedAudioBuffer::create(name, 2, 512).unwrap(),
        );

        let (bridge, handle) =
            crate::lockfree_bridge::LockFreeBridge::new_from_std_stream(client_std, buffer)
                .unwrap();

        // Give bridge thread a moment to start up and register IO
        std::thread::sleep(std::time::Duration::from_millis(50));

        let metadata = PluginMetadata::new("test.plugin", "Test Plugin")
            .audio_io(2, 2)
            .editor(true, Some((800, 600)));

        let bridge_arc: std::sync::Arc<dyn crate::bridge::PluginBridge> = std::sync::Arc::new(bridge);
        let plugin_handle =
            crate::handle::PluginHandle::from_bridge_and_metadata(bridge_arc, metadata);

        (plugin_handle, handle, server_thread)
    }

    #[test]
    fn test_handle_no_bridge_returns_none() {
        let client = test_client(2, 2);
        // No bridge → all control methods return None/false
        assert!(client.open_editor(0).is_none());
        assert!(!client.close_editor());
        client.editor_idle(); // no-op, no panic
        assert!(client.save_state().is_none());
        assert!(!client.load_state(&[1, 2, 3]));
        assert!(client.get_parameter_list().is_none());
        assert!(client.get_parameter_value(0).is_none());
    }

    #[test]
    fn test_handle_metadata() {
        let metadata = PluginMetadata::new("com.test.synth", "Test Synth")
            .editor(true, Some((800, 600)));

        // Create a dummy bridge for metadata-only tests
        let (client_std, _server_std) = std::os::unix::net::UnixStream::pair().unwrap();
        client_std.set_nonblocking(true).unwrap();

        let name = unique_shm_name("meta");
        let buffer = std::sync::Arc::new(
            crate::shared_memory::SharedAudioBuffer::create(name, 2, 512).unwrap(),
        );
        let (bridge, _bridge_handle) =
            crate::lockfree_bridge::LockFreeBridge::new_from_std_stream(client_std, buffer)
                .unwrap();

        let bridge_arc: std::sync::Arc<dyn crate::bridge::PluginBridge> = std::sync::Arc::new(bridge);
        let handle =
            crate::handle::PluginHandle::from_bridge_and_metadata(bridge_arc, metadata);

        assert_eq!(handle.name(), "Test Synth");
        assert!(handle.has_editor());
        assert_eq!(handle.metadata().id, "com.test.synth");
    }

    #[test]
    fn test_handle_save_state_roundtrip() {
        use crate::protocol::BridgeMessage;

        let (handle, _bridge_handle, _server_thread) = setup_handle_with_mock_server(|msg| match msg {
            HostMessage::SaveState => Some(BridgeMessage::StateData {
                data: vec![0xDE, 0xAD, 0xBE, 0xEF],
            }),
            HostMessage::LoadState { .. } => {
                // LoadState is fire-and-forget in bridge, no response needed
                None
            }
            _ => None,
        });

        let state = handle.save_state();
        assert!(state.is_some(), "save_state should return Some");
        assert_eq!(state.unwrap(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn test_handle_load_state_chainable() {
        use crate::protocol::BridgeMessage;

        let (handle, _bridge_handle, _server_thread) = setup_handle_with_mock_server(|msg| match msg {
            HostMessage::SaveState => Some(BridgeMessage::StateData {
                data: vec![1, 2, 3],
            }),
            _ => None,
        });

        // Chainable: load_state returns &Self
        let same_handle = handle.load_state(&[1, 2, 3]);
        assert_eq!(same_handle.name(), handle.name());
    }

    #[test]
    fn test_handle_get_parameter_list() {
        use crate::protocol::{BridgeMessage, ParameterInfo};

        let (handle, _bridge_handle, _server_thread) = setup_handle_with_mock_server(|msg| match msg {
            HostMessage::GetParameterList => Some(BridgeMessage::ParameterList {
                parameters: vec![
                    ParameterInfo::new(0, "Volume".to_string()),
                    ParameterInfo::new(1, "Pan".to_string()),
                    ParameterInfo::new(2, "Cutoff".to_string()),
                ],
            }),
            _ => None,
        });

        let params = handle.parameters();
        assert!(params.is_some());
        let params = params.unwrap();
        assert_eq!(params.len(), 3);
        assert_eq!(params[0].name, "Volume");
        assert_eq!(params[1].name, "Pan");
        assert_eq!(params[2].name, "Cutoff");
    }

    #[test]
    fn test_handle_get_parameter_value() {
        use crate::protocol::BridgeMessage;

        let (handle, _bridge_handle, _server_thread) = setup_handle_with_mock_server(|msg| match msg {
            HostMessage::GetParameter { param_id: 42 } => {
                Some(BridgeMessage::ParameterValue { value: Some(0.75) })
            }
            HostMessage::GetParameter { .. } => {
                Some(BridgeMessage::ParameterValue { value: None })
            }
            _ => None,
        });

        assert_eq!(handle.get_parameter(42), Some(0.75));
    }

    #[test]
    fn test_handle_set_parameter_chainable() {
        let (handle, _bridge_handle, _server_thread) = setup_handle_with_mock_server(|_| None);

        // set_parameter is fire-and-forget via RT path, no response needed
        let h = handle
            .set_parameter(0, 0.1)
            .set_parameter(1, 0.2)
            .set_parameter(2, 0.3);
        assert_eq!(h.name(), "Test Plugin");
    }

    #[test]
    fn test_handle_open_editor() {
        use crate::protocol::BridgeMessage;

        let (handle, _bridge_handle, _server_thread) = setup_handle_with_mock_server(|msg| match msg {
            HostMessage::OpenEditor { .. } => Some(BridgeMessage::EditorOpened {
                width: 1024,
                height: 768,
            }),
            _ => None,
        });

        let result = handle.open_editor(0x12345678);
        assert_eq!(result, Some((1024, 768)));
    }

    #[test]
    fn test_handle_close_editor_chainable() {
        use crate::protocol::BridgeMessage;

        let (handle, _bridge_handle, _server_thread) = setup_handle_with_mock_server(|msg| match msg {
            HostMessage::CloseEditor => Some(BridgeMessage::EditorClosed),
            _ => None,
        });

        let h = handle.close_editor();
        assert_eq!(h.name(), "Test Plugin");
    }

    #[test]
    fn test_handle_editor_idle_chainable() {
        let (handle, _bridge_handle, _server_thread) = setup_handle_with_mock_server(|_| None);

        // editor_idle is fire-and-forget
        let h = handle.editor_idle().editor_idle().editor_idle();
        assert_eq!(h.name(), "Test Plugin");
    }

    #[test]
    fn test_handle_clone() {
        let (handle, _bridge_handle, _server_thread) = setup_handle_with_mock_server(|_| None);

        let cloned = handle.clone();
        assert_eq!(cloned.name(), handle.name());
        assert_eq!(cloned.has_editor(), handle.has_editor());
        assert_eq!(cloned.metadata().id, handle.metadata().id);
    }

    #[test]
    fn test_handle_is_crashed_initially_false() {
        let (handle, _bridge_handle, _server_thread) = setup_handle_with_mock_server(|_| None);
        assert!(!handle.is_crashed());
    }
}
