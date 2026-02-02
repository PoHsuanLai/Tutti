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
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Arc;
use tutti_core::{AudioUnit, BufferMut, BufferRef, Sample, SignalFrame, F64};
use tutti_midi_io::MidiEvent;

/// Batch size for tick() accumulation (matches fundsp MAX_BUFFER_SIZE).
const TICK_BATCH_SIZE: usize = 64;

/// Pre-allocated scratch buffer for RT-safe audio conversion.
#[derive(Clone)]
struct ScratchBuffer<T: Copy> {
    input: Vec<T>,
    output: Vec<T>,
}

impl<T: Copy + Default> ScratchBuffer<T> {
    fn new(max_samples: usize) -> Self {
        Self {
            input: vec![T::default(); max_samples],
            output: vec![T::default(); max_samples],
        }
    }
}

/// Per-channel tick accumulation buffer. Batches single-sample tick() calls.
#[derive(Clone)]
struct TickBuffer<T: Copy> {
    input: Vec<Vec<T>>,
    output: Vec<Vec<T>>,
    write_pos: usize,
    read_pos: usize,
    filled: usize,
}

impl<T: Copy + Default> TickBuffer<T> {
    fn new(num_input_channels: usize, num_output_channels: usize) -> Self {
        Self {
            input: (0..num_input_channels)
                .map(|_| vec![T::default(); TICK_BATCH_SIZE])
                .collect(),
            output: (0..num_output_channels)
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
}

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
    scratch_f32: ScratchBuffer<f32>,
    scratch_f64: ScratchBuffer<f64>,
    tick_f32: TickBuffer<f32>,
    tick_f64: TickBuffer<f64>,
    midi_producer: Arc<UnsafeCell<ringbuf::HeapProd<MidiEvent>>>,
    midi_consumer: Arc<UnsafeCell<ringbuf::HeapCons<MidiEvent>>>,
    /// Pre-allocated buffer for RT-safe MIDI event draining (avoids per-call allocation)
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
        // Find bridge binary
        let bridge_path = std::env::current_exe()
            .map(|mut p| {
                p.pop();
                p.push("plugin-server");
                p
            })
            .map_err(BridgeError::Io)?;

        // Spawn bridge process
        let process = Command::new(bridge_path)
            .arg(&config.socket_path)
            .spawn()
            .map_err(BridgeError::Io)?;

        // Wait for bridge to start
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Connect to bridge
        let mut transport = MessageTransport::connect(&config.socket_path).await?;

        // Create shared memory for audio first (before loading plugin)
        let max_samples = config.max_buffer_size;

        // Load plugin with preferred format from config
        let preferred_format = config.preferred_format;
        let plugin_path_for_error = plugin_path.clone();
        transport
            .send_host_message(&HostMessage::LoadPlugin {
                path: plugin_path,
                sample_rate: sample_rate as f32,
                preferred_format,
            })
            .await?;

        let response = transport.recv_bridge_message().await?;
        let metadata = match response {
            BridgeMessage::PluginLoaded { metadata } => metadata,
            BridgeMessage::Error { message } => {
                return Err(BridgeError::LoadFailed {
                    path: plugin_path_for_error,
                    stage: LoadStage::Opening,
                    reason: message,
                });
            }
            _ => {
                return Err(BridgeError::ProtocolError(format!(
                    "Unexpected response: {:?}",
                    response
                )));
            }
        };

        // Determine negotiated format: use f64 if we preferred it AND plugin supports it
        let negotiated_format =
            if preferred_format == SampleFormat::Float64 && metadata.supports_f64 {
                SampleFormat::Float64
            } else {
                SampleFormat::Float32
            };

        // Create shared memory for audio with the negotiated format
        let num_channels = metadata.audio_io.inputs.max(metadata.audio_io.outputs);
        let audio_buffer = Arc::new(
            SharedAudioBuffer::create_with_format(
                format!("dawai_plugin_{}", std::process::id()),
                num_channels,
                max_samples,
                negotiated_format,
            )
            .map_err(|e| BridgeError::Io(std::io::Error::other(e)))?,
        );

        // Set up lock-free bridge
        let (bridge, bridge_thread) = LockFreeBridge::new(transport, audio_buffer)?;

        // Create lock-free SPSC ring buffer for MIDI events (512 event capacity)
        let (midi_prod, midi_cons) = ringbuf::HeapRb::<MidiEvent>::new(512).split();
        #[allow(clippy::arc_with_non_send_sync)]
        let midi_producer = Arc::new(UnsafeCell::new(midi_prod));
        #[allow(clippy::arc_with_non_send_sync)]
        let midi_consumer = Arc::new(UnsafeCell::new(midi_cons));

        let inputs = metadata.audio_io.inputs;
        let outputs = metadata.audio_io.outputs;

        // Create client and handle
        let client = Self {
            bridge: Some(bridge),
            inputs,
            outputs,
            negotiated_format,
            scratch_f32: ScratchBuffer::new(max_samples),
            scratch_f64: ScratchBuffer::new(max_samples),
            tick_f32: TickBuffer::new(inputs, outputs),
            tick_f64: TickBuffer::new(inputs, outputs),
            midi_producer,
            midi_consumer,
            midi_drain_buffer: smallvec::SmallVec::new(),
        };

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

    /// Set plugin parameter (RT-safe).
    pub fn set_parameter(&self, index: i32, value: f32) {
        if let Some(bridge) = &self.bridge {
            let _ = bridge.set_parameter_rt(index, value);
        }
    }

    /// Negotiated sample format (Float32 or Float64).
    pub fn sample_format(&self) -> SampleFormat {
        self.negotiated_format
    }

    fn output_silence(size: usize, num_outputs: usize, output: &mut BufferMut) {
        for i in 0..size {
            for ch in 0..num_outputs {
                output.set_f32(ch, i, 0.0);
            }
        }
    }

    fn output_silence_generic<S: Sample>(
        size: usize,
        num_outputs: usize,
        output: &mut BufferMut<'_, S>,
    ) {
        for i in 0..size {
            for ch in 0..num_outputs {
                output.set_scalar(ch, i, S::scalar_zero());
            }
        }
    }

    fn drain_midi_events(&mut self) {
        let cons = unsafe { &mut *self.midi_consumer.get() };
        self.midi_drain_buffer.clear();
        while let Some(event) = cons.try_pop() {
            self.midi_drain_buffer.push(event);
        }
    }

    fn flush_tick_f32(&mut self) {
        let size = self.tick_f32.write_pos;
        if size == 0 {
            return;
        }

        // Drain MIDI events before borrowing bridge (avoids borrow conflict)
        self.drain_midi_events();

        let bridge = match &self.bridge {
            Some(b) => b,
            None => {
                // No bridge — fill output with silence
                for ch in 0..self.outputs {
                    self.tick_f32.output[ch][..size].fill(0.0);
                }
                self.tick_f32.filled = size;
                self.tick_f32.write_pos = 0;
                self.tick_f32.read_pos = 0;
                return;
            }
        };

        // Write accumulated input to bridge (format depends on negotiation)
        let write_ok = match self.negotiated_format {
            SampleFormat::Float64 => {
                // Convert f32 input → f64 scratch, then write
                let mut ok = true;
                for ch in 0..self.inputs {
                    for i in 0..size {
                        self.scratch_f64.input[i] = self.tick_f32.input[ch][i] as f64;
                    }
                    if bridge
                        .write_input_channel_f64(ch, &self.scratch_f64.input[..size])
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
                    if bridge
                        .write_input_channel(ch, &self.tick_f32.input[ch][..size])
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
            for ch in 0..self.outputs {
                self.tick_f32.output[ch][..size].fill(0.0);
            }
            self.tick_f32.filled = size;
            self.tick_f32.write_pos = 0;
            self.tick_f32.read_pos = 0;
            return;
        }

        let success = bridge.process_rt_with_midi(size, &self.midi_drain_buffer);

        if success {
            match self.negotiated_format {
                SampleFormat::Float64 => {
                    // Read f64 output, convert to f32
                    for ch in 0..self.outputs {
                        if let Ok(n) = bridge
                            .read_output_channel_into_f64(ch, &mut self.scratch_f64.output[..size])
                        {
                            for i in 0..n {
                                self.tick_f32.output[ch][i] = self.scratch_f64.output[i] as f32;
                            }
                            for i in n..size {
                                self.tick_f32.output[ch][i] = 0.0;
                            }
                        } else {
                            self.tick_f32.output[ch][..size].fill(0.0);
                        }
                    }
                }
                SampleFormat::Float32 => {
                    for ch in 0..self.outputs {
                        if let Ok(n) = bridge
                            .read_output_channel_into(ch, &mut self.scratch_f32.output[..size])
                        {
                            self.tick_f32.output[ch][..n]
                                .copy_from_slice(&self.scratch_f32.output[..n]);
                            for i in n..size {
                                self.tick_f32.output[ch][i] = 0.0;
                            }
                        } else {
                            self.tick_f32.output[ch][..size].fill(0.0);
                        }
                    }
                }
            }
        } else {
            // No response yet — output silence for this batch
            for ch in 0..self.outputs {
                self.tick_f32.output[ch][..size].fill(0.0);
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

        // Drain MIDI events before borrowing bridge (avoids borrow conflict)
        self.drain_midi_events();

        let bridge = match &self.bridge {
            Some(b) => b,
            None => {
                for ch in 0..self.outputs {
                    self.tick_f64.output[ch][..size].fill(0.0);
                }
                self.tick_f64.filled = size;
                self.tick_f64.write_pos = 0;
                self.tick_f64.read_pos = 0;
                return;
            }
        };

        // Write accumulated input to bridge
        let write_ok = match self.negotiated_format {
            SampleFormat::Float64 => {
                let mut ok = true;
                for ch in 0..self.inputs {
                    if bridge
                        .write_input_channel_f64(ch, &self.tick_f64.input[ch][..size])
                        .is_err()
                    {
                        ok = false;
                        break;
                    }
                }
                ok
            }
            SampleFormat::Float32 => {
                // Convert f64 input → f32 scratch, then write
                let mut ok = true;
                for ch in 0..self.inputs {
                    for i in 0..size {
                        self.scratch_f32.input[i] = self.tick_f64.input[ch][i] as f32;
                    }
                    if bridge
                        .write_input_channel(ch, &self.scratch_f32.input[..size])
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
            for ch in 0..self.outputs {
                self.tick_f64.output[ch][..size].fill(0.0);
            }
            self.tick_f64.filled = size;
            self.tick_f64.write_pos = 0;
            self.tick_f64.read_pos = 0;
            return;
        }

        let success = bridge.process_rt_with_midi(size, &self.midi_drain_buffer);

        if success {
            match self.negotiated_format {
                SampleFormat::Float64 => {
                    for ch in 0..self.outputs {
                        if let Ok(n) = bridge
                            .read_output_channel_into_f64(ch, &mut self.scratch_f64.output[..size])
                        {
                            self.tick_f64.output[ch][..n]
                                .copy_from_slice(&self.scratch_f64.output[..n]);
                            for i in n..size {
                                self.tick_f64.output[ch][i] = 0.0;
                            }
                        } else {
                            self.tick_f64.output[ch][..size].fill(0.0);
                        }
                    }
                }
                SampleFormat::Float32 => {
                    // Read f32 output, convert to f64
                    for ch in 0..self.outputs {
                        if let Ok(n) = bridge
                            .read_output_channel_into(ch, &mut self.scratch_f32.output[..size])
                        {
                            for i in 0..n {
                                self.tick_f64.output[ch][i] = self.scratch_f32.output[i] as f64;
                            }
                            for i in n..size {
                                self.tick_f64.output[ch][i] = 0.0;
                            }
                        } else {
                            self.tick_f64.output[ch][..size].fill(0.0);
                        }
                    }
                }
            }
        } else {
            for ch in 0..self.outputs {
                self.tick_f64.output[ch][..size].fill(0.0);
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
            let _ = bridge.set_sample_rate_rt(sample_rate as f32);
        }
    }

    fn tick(&mut self, input: &[f32], output: &mut [f32]) {
        // Store input sample into accumulation buffer
        for (ch, &sample) in input.iter().enumerate().take(self.inputs) {
            self.tick_f32.input[ch][self.tick_f32.write_pos] = sample;
        }
        self.tick_f32.write_pos += 1;

        // If batch full, flush through bridge
        if self.tick_f32.write_pos >= TICK_BATCH_SIZE {
            self.flush_tick_f32();
        }

        // Read output sample from pre-filled buffer
        // (first batch outputs silence until flush completes)
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
        // Drain MIDI events before borrowing bridge (avoids borrow conflict)
        self.drain_midi_events();

        let bridge = match &self.bridge {
            Some(b) => b,
            None => {
                Self::output_silence(size, self.outputs, output);
                return;
            }
        };

        match self.negotiated_format {
            SampleFormat::Float64 => {
                // f64 bridge: convert f32 input → f64, process, convert f64 output → f32
                for ch in 0..self.inputs {
                    for i in 0..size {
                        self.scratch_f64.input[i] = input.at_f32(ch, i) as f64;
                    }
                    if bridge
                        .write_input_channel_f64(ch, &self.scratch_f64.input[..size])
                        .is_err()
                    {
                        Self::output_silence(size, self.outputs, output);
                        return;
                    }
                }

                let success = bridge.process_rt_with_midi(size, &self.midi_drain_buffer);

                if success {
                    for ch in 0..self.outputs {
                        if let Ok(n) = bridge
                            .read_output_channel_into_f64(ch, &mut self.scratch_f64.output[..size])
                        {
                            for i in 0..n {
                                output.set_f32(ch, i, self.scratch_f64.output[i] as f32);
                            }
                            for i in n..size {
                                output.set_f32(ch, i, 0.0);
                            }
                        } else {
                            for i in 0..size {
                                output.set_f32(ch, i, 0.0);
                            }
                        }
                    }
                } else {
                    Self::output_silence(size, self.outputs, output);
                }
            }
            SampleFormat::Float32 => {
                for ch in 0..self.inputs {
                    for i in 0..size {
                        self.scratch_f32.input[i] = input.at_f32(ch, i);
                    }
                    if bridge
                        .write_input_channel(ch, &self.scratch_f32.input[..size])
                        .is_err()
                    {
                        Self::output_silence(size, self.outputs, output);
                        return;
                    }
                }

                let success = bridge.process_rt_with_midi(size, &self.midi_drain_buffer);

                if success {
                    for ch in 0..self.outputs {
                        if let Ok(n) = bridge
                            .read_output_channel_into(ch, &mut self.scratch_f32.output[..size])
                        {
                            for i in 0..n {
                                output.set_f32(ch, i, self.scratch_f32.output[i]);
                            }
                            for i in n..size {
                                output.set_f32(ch, i, 0.0);
                            }
                        } else {
                            for i in 0..size {
                                output.set_f32(ch, i, 0.0);
                            }
                        }
                    }
                } else {
                    Self::output_silence(size, self.outputs, output);
                }
            }
        }
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
            let _ = bridge.set_sample_rate_rt(sample_rate as f32);
        }
    }

    fn tick(&mut self, input: &[f64], output: &mut [f64]) {
        // Store input sample into accumulation buffer
        for (ch, &sample) in input.iter().enumerate().take(self.inputs) {
            self.tick_f64.input[ch][self.tick_f64.write_pos] = sample;
        }
        self.tick_f64.write_pos += 1;

        // If batch full, flush through bridge
        if self.tick_f64.write_pos >= TICK_BATCH_SIZE {
            self.flush_tick_f64();
        }

        // Read output sample from pre-filled buffer
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
        // Drain MIDI events before borrowing bridge (avoids borrow conflict)
        self.drain_midi_events();

        let bridge = match &self.bridge {
            Some(b) => b,
            None => {
                Self::output_silence_generic::<F64>(size, self.outputs, output);
                return;
            }
        };

        match self.negotiated_format {
            SampleFormat::Float64 => {
                // Native f64 path
                for ch in 0..self.inputs {
                    for i in 0..size {
                        self.scratch_f64.input[i] = input.at_scalar(ch, i);
                    }
                    if bridge
                        .write_input_channel_f64(ch, &self.scratch_f64.input[..size])
                        .is_err()
                    {
                        Self::output_silence_generic::<F64>(size, self.outputs, output);
                        return;
                    }
                }

                let success = bridge.process_rt_with_midi(size, &self.midi_drain_buffer);

                if success {
                    for ch in 0..self.outputs {
                        if let Ok(n) = bridge
                            .read_output_channel_into_f64(ch, &mut self.scratch_f64.output[..size])
                        {
                            for i in 0..n {
                                output.set_scalar(ch, i, self.scratch_f64.output[i]);
                            }
                            for i in n..size {
                                output.set_scalar(ch, i, 0.0);
                            }
                        } else {
                            for i in 0..size {
                                output.set_scalar(ch, i, 0.0);
                            }
                        }
                    }
                } else {
                    Self::output_silence_generic::<F64>(size, self.outputs, output);
                }
            }
            SampleFormat::Float32 => {
                // f32 bridge: convert f64 input → f32, process, convert f32 output → f64
                for ch in 0..self.inputs {
                    for i in 0..size {
                        self.scratch_f32.input[i] = input.at_scalar(ch, i) as f32;
                    }
                    if bridge
                        .write_input_channel(ch, &self.scratch_f32.input[..size])
                        .is_err()
                    {
                        Self::output_silence_generic::<F64>(size, self.outputs, output);
                        return;
                    }
                }

                let success = bridge.process_rt_with_midi(size, &self.midi_drain_buffer);

                if success {
                    for ch in 0..self.outputs {
                        if let Ok(n) = bridge
                            .read_output_channel_into(ch, &mut self.scratch_f32.output[..size])
                        {
                            for i in 0..n {
                                output.set_scalar(ch, i, self.scratch_f32.output[i] as f64);
                            }
                            for i in n..size {
                                output.set_scalar(ch, i, 0.0);
                            }
                        } else {
                            for i in 0..size {
                                output.set_scalar(ch, i, 0.0);
                            }
                        }
                    }
                } else {
                    Self::output_silence_generic::<F64>(size, self.outputs, output);
                }
            }
        }
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

        // Clear old events (drain consumer)
        while cons.try_pop().is_some() {}

        // Push new events (drop if full)
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
