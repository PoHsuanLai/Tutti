//! Plugin client for multi-process plugin hosting.
//!
//! Provides RT-safe communication with plugin server processes via lock-free queues.

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

// =============================================================================
// Plugin Client
// =============================================================================

/// Plugin client for multi-process plugin hosting.
///
/// Communicates with plugin server via lock-free queues. Implements both
/// `AudioUnit` (f32) and `AudioUnit<F64>` for native f64 processing.
///
/// Cloning is cheap - each instance has independent buffers but shares the bridge.
#[derive(Clone)]
pub struct PluginClient {
    bridge: Option<LockFreeBridge>,
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
        let metadata =
            Self::load_plugin_on_server(&mut transport, &config, &plugin_path, sample_rate).await?;

        let negotiated_format = Self::negotiate_format(config.preferred_format, &metadata);
        let (bridge, bridge_thread) =
            Self::setup_bridge(&config, &metadata, negotiated_format, transport)?;

        let client = Self::create_client(bridge, &metadata, &config, negotiated_format);
        let handle = PluginClientHandle {
            process: Some(process),
            bridge_thread: Some(bridge_thread),
            config,
        };

        Ok((client, handle))
    }

    /// Plugin latency in samples.
    pub fn latency_samples(&self) -> usize {
        self.bridge
            .as_ref()
            .and_then(|b| b.metadata().map(|m| m.latency_samples))
            .unwrap_or(0)
    }

    /// Plugin metadata (I/O channels, latency, format support).
    pub fn metadata(&self) -> Option<PluginMetadata> {
        self.bridge.as_ref().and_then(|b| b.metadata())
    }

    /// Set plugin parameter by native ID (RT-safe).
    pub fn set_parameter(&self, param_id: u32, value: f32) {
        if let Some(bridge) = &self.bridge {
            let _ = bridge.set_parameter_rt(param_id, value);
        }
    }

    /// Negotiated sample format (Float32 or Float64).
    pub fn sample_format(&self) -> SampleFormat {
        self.negotiated_format
    }
}

impl PluginClient {
    fn spawn_bridge_process(config: &BridgeConfig) -> Result<Child> {
        let bridge_path = std::env::current_exe()
            .map(|mut p| {
                p.pop();
                p.push("plugin-server");
                p
            })
            .map_err(BridgeError::Io)?;

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
    ) -> Result<Box<PluginMetadata>> {
        transport
            .send_host_message(&HostMessage::LoadPlugin {
                path: plugin_path.to_path_buf(),
                sample_rate,
                block_size: config.max_buffer_size,
                preferred_format: config.preferred_format,
            })
            .await?;

        match transport.recv_bridge_message().await? {
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

    fn negotiate_format(preferred: SampleFormat, metadata: &PluginMetadata) -> SampleFormat {
        if preferred == SampleFormat::Float64 && metadata.supports_f64 {
            SampleFormat::Float64
        } else {
            SampleFormat::Float32
        }
    }

    fn setup_bridge(
        config: &BridgeConfig,
        metadata: &PluginMetadata,
        format: SampleFormat,
        transport: MessageTransport,
    ) -> Result<(LockFreeBridge, BridgeThreadHandle)> {
        let num_channels = metadata.audio_io.inputs.max(metadata.audio_io.outputs);
        let audio_buffer = Arc::new(
            SharedAudioBuffer::create_with_format(
                format!("dawai_plugin_{}", std::process::id()),
                num_channels,
                config.max_buffer_size,
                format,
            )
            .map_err(|e| BridgeError::Io(std::io::Error::other(e)))?,
        );

        LockFreeBridge::new(transport, audio_buffer)
    }

    fn create_client(
        bridge: LockFreeBridge,
        metadata: &PluginMetadata,
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
            bridge: Some(bridge),
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
    }

    /// Fill output with silence.
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

    /// Generic process implementation.
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

        // Write inputs to bridge
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

        if !bridge.process_rt_with_midi(size, &self.midi_drain_buffer) {
            Self::fill_silence(size, self.outputs, set_output);
            return;
        }

        // Read outputs from bridge
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

        // Write inputs
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

        if !self
            .bridge
            .as_ref()
            .unwrap()
            .process_rt_with_midi(size, &self.midi_drain_buffer)
        {
            self.tick_f32.fill_output_silence(size);
            return;
        }

        // Read outputs
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

        // Write inputs
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

        if !self
            .bridge
            .as_ref()
            .unwrap()
            .process_rt_with_midi(size, &self.midi_drain_buffer)
        {
            self.tick_f64.fill_output_silence(size);
            return;
        }

        // Read outputs
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
        self as *const _ as u64
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
        self as *const _ as u64
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

impl tutti_core::MidiAudioUnit for PluginClient {
    fn queue_midi(&mut self, events: &[MidiEvent]) {
        let prod = unsafe { &mut *self.midi_producer.get() };
        let cons = unsafe { &mut *self.midi_consumer.get() };

        while cons.try_pop().is_some() {}

        for event in events {
            let _ = prod.try_push(*event);
        }
    }

    fn has_midi_output(&self) -> bool {
        false
    }

    fn clear_midi(&mut self) {
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
    }
}
