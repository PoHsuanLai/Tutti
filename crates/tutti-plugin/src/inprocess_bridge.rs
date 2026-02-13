//! In-process plugin bridge — loads plugins in the same process.
//!
//! Unlike `LockFreeBridge` (which communicates with a child process via IPC),
//! this bridge hosts the plugin in-process. Audio/param/state operations go through
//! an ArrayQueue to a dedicated thread; GUI methods (`open_editor`, `close_editor`,
//! `editor_idle`) are called directly on the caller's thread (required by CLAP/VST3).

use crate::bridge::PluginBridge;
use crate::error::{BridgeError, Result};
use crate::instance::{PluginInstance, ProcessContext, ProcessOutput};
use crate::protocol::{
    MidiEventVec, NoteExpressionChanges, ParameterChanges, ParameterInfo, TransportInfo,
};
use crossbeam::queue::ArrayQueue;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

type SharedPlugin = Arc<Mutex<Box<dyn PluginInstance>>>;

const COMMAND_QUEUE_SIZE: usize = 128;
const RESPONSE_QUEUE_SIZE: usize = 128;

/// In-process audio buffer using plain Vec allocations.
///
/// Each channel has a separate f32 and f64 buffer. The audio thread writes input
/// and reads output; the bridge thread reads input and writes output. Access is
/// serialized by the command queue (no concurrent access to the same data).
pub(crate) struct InProcessAudioBuffer {
    f32_channels: Vec<parking_lot::Mutex<Vec<f32>>>,
    f64_channels: Vec<parking_lot::Mutex<Vec<f64>>>,
    num_channels: usize,
    max_samples: usize,
}

impl InProcessAudioBuffer {
    fn new(num_channels: usize, max_samples: usize) -> Self {
        let f32_channels = (0..num_channels)
            .map(|_| parking_lot::Mutex::new(vec![0.0f32; max_samples]))
            .collect();
        let f64_channels = (0..num_channels)
            .map(|_| parking_lot::Mutex::new(vec![0.0f64; max_samples]))
            .collect();
        Self {
            f32_channels,
            f64_channels,
            num_channels,
            max_samples,
        }
    }

    fn write_f32(&self, channel: usize, data: &[f32]) -> Result<()> {
        if channel >= self.num_channels {
            return Err(BridgeError::ProtocolError(format!(
                "channel {} >= {}",
                channel, self.num_channels
            )));
        }
        let mut buf = self.f32_channels[channel].lock();
        let len = data.len().min(buf.len());
        buf[..len].copy_from_slice(&data[..len]);
        Ok(())
    }

    fn read_f32(&self, channel: usize, output: &mut [f32]) -> Result<usize> {
        if channel >= self.num_channels {
            return Err(BridgeError::ProtocolError(format!(
                "channel {} >= {}",
                channel, self.num_channels
            )));
        }
        let buf = self.f32_channels[channel].lock();
        let samples = buf.len().min(output.len());
        output[..samples].copy_from_slice(&buf[..samples]);
        Ok(samples)
    }

    fn write_f64(&self, channel: usize, data: &[f64]) -> Result<()> {
        if channel >= self.num_channels {
            return Err(BridgeError::ProtocolError(format!(
                "channel {} >= {}",
                channel, self.num_channels
            )));
        }
        let mut buf = self.f64_channels[channel].lock();
        let len = data.len().min(buf.len());
        buf[..len].copy_from_slice(&data[..len]);
        Ok(())
    }

    fn read_f64(&self, channel: usize, output: &mut [f64]) -> Result<usize> {
        if channel >= self.num_channels {
            return Err(BridgeError::ProtocolError(format!(
                "channel {} >= {}",
                channel, self.num_channels
            )));
        }
        let buf = self.f64_channels[channel].lock();
        let samples = buf.len().min(output.len());
        output[..samples].copy_from_slice(&buf[..samples]);
        Ok(samples)
    }

    /// Build input/output slices for f32 processing, call the plugin, and write back.
    fn process_f32(
        &self,
        plugin: &SharedPlugin,
        num_samples: usize,
        ctx: &ProcessContext,
    ) -> ProcessOutput {
        let n = num_samples.min(self.max_samples);
        // Lock all channels for the duration of processing.
        let mut guards: Vec<_> = self.f32_channels.iter().map(|ch| ch.lock()).collect();

        // Split into input refs and output mut refs.
        // For simplicity, use the same buffers for both input and output
        // (plugin processes in-place on outputs, reads from inputs).
        let input_vecs: Vec<Vec<f32>> = guards.iter().map(|g| g[..n].to_vec()).collect();
        let input_slices: Vec<&[f32]> = input_vecs.iter().map(|v| v.as_slice()).collect();

        let mut output_vecs: Vec<Vec<f32>> = guards.iter().map(|g| g[..n].to_vec()).collect();
        let mut output_slices: Vec<&mut [f32]> =
            output_vecs.iter_mut().map(|v| v.as_mut_slice()).collect();

        let mut audio_buffer = crate::protocol::AudioBuffer {
            inputs: &input_slices,
            outputs: &mut output_slices,
            num_samples: n,
            sample_rate: 0.0, // sample rate is set separately
        };

        let result = plugin.lock().process_f32(&mut audio_buffer, ctx);

        // Write processed output back to shared buffer
        for (ch, guard) in guards.iter_mut().enumerate() {
            guard[..n].copy_from_slice(&output_vecs[ch][..n]);
        }

        result
    }

    /// Build input/output slices for f64 processing, call the plugin, and write back.
    fn process_f64(
        &self,
        plugin: &SharedPlugin,
        num_samples: usize,
        ctx: &ProcessContext,
    ) -> ProcessOutput {
        let n = num_samples.min(self.max_samples);
        let mut guards: Vec<_> = self.f64_channels.iter().map(|ch| ch.lock()).collect();

        let input_vecs: Vec<Vec<f64>> = guards.iter().map(|g| g[..n].to_vec()).collect();
        let input_slices: Vec<&[f64]> = input_vecs.iter().map(|v| v.as_slice()).collect();

        let mut output_vecs: Vec<Vec<f64>> = guards.iter().map(|g| g[..n].to_vec()).collect();
        let mut output_slices: Vec<&mut [f64]> =
            output_vecs.iter_mut().map(|v| v.as_mut_slice()).collect();

        let mut audio_buffer = crate::protocol::AudioBuffer64 {
            inputs: &input_slices,
            outputs: &mut output_slices,
            num_samples: n,
            sample_rate: 0.0,
        };

        let result = plugin.lock().process_f64(&mut audio_buffer, ctx);

        for (ch, guard) in guards.iter_mut().enumerate() {
            guard[..n].copy_from_slice(&output_vecs[ch][..n]);
        }

        result
    }
}

struct ProcessCommandData {
    num_samples: usize,
    midi_events: MidiEventVec,
    param_changes: ParameterChanges,
    note_expression: NoteExpressionChanges,
    transport: TransportInfo,
}

/// Commands sent to the bridge thread (audio-thread operations only).
enum BridgeCommand {
    Process(Box<ProcessCommandData>),
    SetParameter { param_id: u32, value: f32 },
    SetSampleRate { rate: f64 },
    Reset,
    Shutdown,
}

#[derive(Debug, Clone)]
enum BridgeResponse {
    AudioProcessed,
    #[allow(dead_code)]
    Error,
}

/// In-process bridge that hosts a plugin on a dedicated thread.
///
/// Audio/param/state operations go through an ArrayQueue to the bridge thread.
/// GUI methods (`open_editor`, `close_editor`, `editor_idle`) are called directly
/// on the caller's thread via a shared mutex, as required by CLAP/VST3.
#[derive(Clone)]
pub struct InProcessBridge {
    plugin: SharedPlugin,
    command_queue: Arc<ArrayQueue<BridgeCommand>>,
    response_queue: Arc<ArrayQueue<BridgeResponse>>,
    audio_buffer: Arc<InProcessAudioBuffer>,
    running: Arc<AtomicBool>,
    crashed: Arc<AtomicBool>,
    /// RT-safe recycling: bridge thread returns used Box here, audio thread reuses it.
    recycle_queue: Arc<ArrayQueue<Box<ProcessCommandData>>>,
}

/// Handle to in-process bridge thread. Shuts down on drop.
pub struct InProcessThreadHandle {
    bridge: InProcessBridge,
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl InProcessBridge {
    /// Create a new in-process bridge hosting the given plugin instance.
    pub fn new(
        plugin: Box<dyn PluginInstance>,
        num_channels: usize,
        max_buffer_size: usize,
    ) -> (Self, InProcessThreadHandle) {
        let command_queue = Arc::new(ArrayQueue::new(COMMAND_QUEUE_SIZE));
        let response_queue = Arc::new(ArrayQueue::new(RESPONSE_QUEUE_SIZE));
        let recycle_queue = Arc::new(ArrayQueue::new(2));
        let running = Arc::new(AtomicBool::new(true));
        let crashed = Arc::new(AtomicBool::new(false));
        let audio_buffer = Arc::new(InProcessAudioBuffer::new(num_channels, max_buffer_size));
        let use_f64 = plugin.supports_f64();
        let plugin: SharedPlugin = Arc::new(Mutex::new(plugin));

        let bridge = Self {
            plugin: Arc::clone(&plugin),
            command_queue: Arc::clone(&command_queue),
            response_queue: Arc::clone(&response_queue),
            audio_buffer: Arc::clone(&audio_buffer),
            running: Arc::clone(&running),
            crashed: Arc::clone(&crashed),
            recycle_queue: Arc::clone(&recycle_queue),
        };

        let thread = {
            let cmd_q = Arc::clone(&command_queue);
            let resp_q = Arc::clone(&response_queue);
            let recycle_q = Arc::clone(&recycle_queue);
            let run = Arc::clone(&running);
            let buf = Arc::clone(&audio_buffer);
            let plugin = Arc::clone(&plugin);

            thread::Builder::new()
                .name("plugin-inprocess".to_string())
                .spawn(move || {
                    Self::run_loop(&cmd_q, &resp_q, &recycle_q, &run, &buf, &plugin, use_f64);
                })
                .expect("failed to spawn in-process plugin thread")
        };

        let handle = InProcessThreadHandle {
            bridge: bridge.clone(),
            thread_handle: Some(thread),
        };

        (bridge, handle)
    }

    /// Bridge thread loop — handles audio processing and RT parameter changes.
    /// All other operations (GUI, state, param queries) go direct via the shared mutex.
    fn run_loop(
        commands: &ArrayQueue<BridgeCommand>,
        responses: &ArrayQueue<BridgeResponse>,
        recycle: &ArrayQueue<Box<ProcessCommandData>>,
        running: &AtomicBool,
        audio_buffer: &InProcessAudioBuffer,
        plugin: &SharedPlugin,
        use_f64: bool,
    ) {
        while running.load(Ordering::Relaxed) {
            if let Some(cmd) = commands.pop() {
                match cmd {
                    BridgeCommand::Process(data) => {
                        let ctx = ProcessContext::new()
                            .midi(&data.midi_events)
                            .params(&data.param_changes)
                            .note_expression(&data.note_expression)
                            .transport(&data.transport);

                        if use_f64 {
                            audio_buffer.process_f64(plugin, data.num_samples, &ctx);
                        } else {
                            audio_buffer.process_f32(plugin, data.num_samples, &ctx);
                        }

                        // Recycle the Box for RT-safe reuse by the audio thread.
                        let _ = recycle.push(data);

                        let _ = responses.push(BridgeResponse::AudioProcessed);
                    }
                    BridgeCommand::SetParameter { param_id, value } => {
                        plugin.lock().set_parameter(param_id, value as f64);
                    }
                    BridgeCommand::SetSampleRate { rate } => {
                        plugin.lock().set_sample_rate(rate);
                    }
                    BridgeCommand::Reset => {}
                    BridgeCommand::Shutdown => {
                        break;
                    }
                }
            } else {
                thread::sleep(Duration::from_micros(100));
            }
        }

        // Stop processing on the audio/bridge thread (CLAP requirement:
        // stop_processing must be called on the same thread as process).
        plugin.lock().stop_processing();
    }
}

impl PluginBridge for InProcessBridge {
    fn process(
        &self,
        num_samples: usize,
        midi_events: MidiEventVec,
        param_changes: ParameterChanges,
        note_expression: NoteExpressionChanges,
        transport: TransportInfo,
    ) -> bool {
        if self.crashed.load(Ordering::Acquire) {
            return false;
        }
        // RT-safe: reuse a recycled Box if available, avoiding heap allocation.
        let mut data = self.recycle_queue.pop().unwrap_or_else(|| {
            Box::new(ProcessCommandData {
                num_samples: 0,
                midi_events: MidiEventVec::new(),
                param_changes: ParameterChanges::new(),
                note_expression: NoteExpressionChanges::new(),
                transport: TransportInfo::default(),
            })
        });
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

        // RT-safe busy-spin: no syscalls, no sleep, stays on-CPU.
        // In-process plugins typically complete in <1ms, so this spins briefly.
        // Bounded to ~5ms worst case (50_000 iterations × ~100ns per spin_loop hint).
        for _ in 0..50_000 {
            if let Some(resp) = self.response_queue.pop() {
                return matches!(resp, BridgeResponse::AudioProcessed);
            }
            core::hint::spin_loop();
        }
        false
    }

    fn set_parameter_rt(&self, param_id: u32, value: f32) -> bool {
        if self.crashed.load(Ordering::Acquire) {
            return false;
        }
        self.command_queue
            .push(BridgeCommand::SetParameter { param_id, value })
            .is_ok()
    }

    fn set_sample_rate_rt(&self, rate: f64) -> bool {
        if self.crashed.load(Ordering::Acquire) {
            return false;
        }
        self.command_queue
            .push(BridgeCommand::SetSampleRate { rate })
            .is_ok()
    }

    fn reset_rt(&self) -> bool {
        if self.crashed.load(Ordering::Acquire) {
            return false;
        }
        self.command_queue.push(BridgeCommand::Reset).is_ok()
    }

    fn write_input_channel(&self, channel: usize, data: &[f32]) -> Result<()> {
        self.audio_buffer.write_f32(channel, data)
    }

    fn read_output_channel_into(&self, channel: usize, output: &mut [f32]) -> Result<usize> {
        self.audio_buffer.read_f32(channel, output)
    }

    fn write_input_channel_f64(&self, channel: usize, data: &[f64]) -> Result<()> {
        self.audio_buffer.write_f64(channel, data)
    }

    fn read_output_channel_into_f64(&self, channel: usize, output: &mut [f64]) -> Result<usize> {
        self.audio_buffer.read_f64(channel, output)
    }

    fn is_crashed(&self) -> bool {
        self.crashed.load(Ordering::Acquire)
    }

    fn open_editor(&self, parent_handle: u64) -> Option<(u32, u32)> {
        if self.crashed.load(Ordering::Acquire) {
            return None;
        }
        // Called directly on the caller's thread (must be main thread for CLAP/VST3).
        let parent_ptr = parent_handle as *mut std::ffi::c_void;
        let mut plugin = self.plugin.lock();
        unsafe { plugin.open_editor(parent_ptr) }.ok()
    }

    fn close_editor(&self) -> bool {
        if self.crashed.load(Ordering::Acquire) {
            return false;
        }
        // Called directly on the caller's thread (must be main thread for CLAP/VST3).
        self.plugin.lock().close_editor();
        true
    }

    fn editor_idle(&self) {
        if self.crashed.load(Ordering::Acquire) {
            return;
        }
        // Called directly on the caller's thread (must be main thread for CLAP/VST3).
        self.plugin.lock().editor_idle();
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        if self.crashed.load(Ordering::Acquire) {
            return None;
        }
        self.plugin.lock().get_state().ok()
    }

    fn load_state(&self, data: &[u8]) -> bool {
        if self.crashed.load(Ordering::Acquire) {
            return false;
        }
        self.plugin.lock().set_state(data).is_ok()
    }

    fn get_parameter_list(&self) -> Option<Vec<ParameterInfo>> {
        if self.crashed.load(Ordering::Acquire) {
            return None;
        }
        Some(self.plugin.lock().get_parameter_list())
    }

    fn get_parameter(&self, param_id: u32) -> Option<f32> {
        if self.crashed.load(Ordering::Acquire) {
            return None;
        }
        Some(self.plugin.lock().get_parameter(param_id) as f32)
    }
}

impl InProcessThreadHandle {
    pub fn shutdown(&mut self) {
        self.bridge.running.store(false, Ordering::Relaxed);
        let _ = self.bridge.command_queue.push(BridgeCommand::Shutdown);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for InProcessThreadHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::{PluginInstance, ProcessContext, ProcessOutput};
    use crate::protocol::{AudioBuffer, AudioBuffer64, ParameterInfo};
    use crate::PluginMetadata;

    /// Mock plugin instance for testing.
    struct MockPlugin {
        metadata: PluginMetadata,
        params: std::collections::HashMap<u32, f64>,
        state: Vec<u8>,
    }

    impl MockPlugin {
        fn new() -> Self {
            let mut params = std::collections::HashMap::new();
            params.insert(0, 0.5);
            params.insert(1, 0.75);
            Self {
                metadata: PluginMetadata::new("mock.plugin", "Mock Plugin")
                    .audio_io(2, 2)
                    .editor(true, Some((800, 600))),
                params,
                state: vec![1, 2, 3],
            }
        }
    }

    impl PluginInstance for MockPlugin {
        fn metadata(&self) -> &PluginMetadata {
            &self.metadata
        }

        fn supports_f64(&self) -> bool {
            false
        }

        fn process_f32<'a>(
            &mut self,
            buffer: &'a mut AudioBuffer<'a>,
            _ctx: &ProcessContext,
        ) -> ProcessOutput {
            // Simple effect: multiply all outputs by 0.5
            let num_samples = buffer.num_samples;
            for ch in 0..buffer.outputs.len() {
                for i in 0..num_samples {
                    let val = if ch < buffer.inputs.len() && i < buffer.inputs[ch].len() {
                        buffer.inputs[ch][i]
                    } else {
                        0.0
                    };
                    buffer.outputs[ch][i] = val * 0.5;
                }
            }
            ProcessOutput::default()
        }

        fn process_f64<'a>(
            &mut self,
            _buffer: &'a mut AudioBuffer64<'a>,
            _ctx: &ProcessContext,
        ) -> ProcessOutput {
            ProcessOutput::default()
        }

        fn set_sample_rate(&mut self, _rate: f64) {}

        fn get_parameter_count(&self) -> usize {
            self.params.len()
        }

        fn get_parameter(&self, id: u32) -> f64 {
            *self.params.get(&id).unwrap_or(&0.0)
        }

        fn set_parameter(&mut self, id: u32, value: f64) {
            self.params.insert(id, value);
        }

        fn get_parameter_list(&mut self) -> Vec<ParameterInfo> {
            vec![
                ParameterInfo::new(0, "Volume".to_string()),
                ParameterInfo::new(1, "Pan".to_string()),
            ]
        }

        fn get_parameter_info(&mut self, id: u32) -> Option<ParameterInfo> {
            match id {
                0 => Some(ParameterInfo::new(0, "Volume".to_string())),
                1 => Some(ParameterInfo::new(1, "Pan".to_string())),
                _ => None,
            }
        }

        fn has_editor(&mut self) -> bool {
            true
        }

        unsafe fn open_editor(&mut self, _parent: *mut std::ffi::c_void) -> Result<(u32, u32)> {
            Ok((800, 600))
        }

        fn close_editor(&mut self) {}

        fn editor_idle(&mut self) {}

        fn get_state(&mut self) -> Result<Vec<u8>> {
            Ok(self.state.clone())
        }

        fn set_state(&mut self, data: &[u8]) -> Result<()> {
            self.state = data.to_vec();
            Ok(())
        }
    }

    /// Mock plugin that uses f64 processing.
    struct MockPluginF64;

    impl PluginInstance for MockPluginF64 {
        fn metadata(&self) -> &PluginMetadata {
            static META: std::sync::OnceLock<PluginMetadata> = std::sync::OnceLock::new();
            META.get_or_init(|| PluginMetadata::new("mock.f64", "Mock F64"))
        }

        fn supports_f64(&self) -> bool {
            true
        }

        fn process_f32<'a>(
            &mut self,
            _buffer: &'a mut AudioBuffer<'a>,
            _ctx: &ProcessContext,
        ) -> ProcessOutput {
            ProcessOutput::default()
        }

        fn process_f64<'a>(
            &mut self,
            buffer: &'a mut AudioBuffer64<'a>,
            _ctx: &ProcessContext,
        ) -> ProcessOutput {
            // Multiply all outputs by 0.25
            let num_samples = buffer.num_samples;
            for ch in 0..buffer.outputs.len() {
                for i in 0..num_samples {
                    let val = if ch < buffer.inputs.len() && i < buffer.inputs[ch].len() {
                        buffer.inputs[ch][i]
                    } else {
                        0.0
                    };
                    buffer.outputs[ch][i] = val * 0.25;
                }
            }
            ProcessOutput::default()
        }

        fn set_sample_rate(&mut self, _rate: f64) {}
        fn get_parameter_count(&self) -> usize {
            0
        }
        fn get_parameter(&self, _id: u32) -> f64 {
            0.0
        }
        fn set_parameter(&mut self, _id: u32, _value: f64) {}
        fn get_parameter_list(&mut self) -> Vec<ParameterInfo> {
            vec![]
        }
        fn get_parameter_info(&mut self, _id: u32) -> Option<ParameterInfo> {
            None
        }
        fn has_editor(&mut self) -> bool {
            false
        }
        unsafe fn open_editor(&mut self, _parent: *mut std::ffi::c_void) -> Result<(u32, u32)> {
            Err(crate::error::BridgeError::EditorError("no editor".into()))
        }
        fn close_editor(&mut self) {}
        fn editor_idle(&mut self) {}
        fn get_state(&mut self) -> Result<Vec<u8>> {
            Ok(vec![])
        }
        fn set_state(&mut self, _data: &[u8]) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_inprocess_bridge_creation() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);
        assert!(!bridge.is_crashed());
    }

    #[test]
    fn test_inprocess_set_parameter() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);

        assert!(bridge.set_parameter_rt(0, 0.9));
        // Give bridge thread time to process
        thread::sleep(Duration::from_millis(10));
        // Verify via get_parameter
        let value = bridge.get_parameter(0);
        assert!(value.is_some());
        assert!((value.unwrap() - 0.9).abs() < 0.01);
    }

    #[test]
    fn test_inprocess_get_parameter_list() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);

        let params = bridge.get_parameter_list();
        assert!(params.is_some());
        let params = params.unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "Volume");
        assert_eq!(params[1].name, "Pan");
    }

    #[test]
    fn test_inprocess_save_load_state() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);

        let state = bridge.save_state();
        assert!(state.is_some());
        assert_eq!(state.unwrap(), vec![1, 2, 3]);

        assert!(bridge.load_state(&[4, 5, 6]));
        let state2 = bridge.save_state();
        assert_eq!(state2.unwrap(), vec![4, 5, 6]);
    }

    #[test]
    fn test_inprocess_open_close_editor() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);

        let result = bridge.open_editor(0x12345678);
        assert_eq!(result, Some((800, 600)));

        assert!(bridge.close_editor());
    }

    #[test]
    fn test_inprocess_editor_idle() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);

        bridge.editor_idle();
        bridge.editor_idle();
    }

    #[test]
    fn test_inprocess_audio_buffer_f32() {
        let buf = InProcessAudioBuffer::new(2, 64);

        let input = vec![0.5f32; 64];
        buf.write_f32(0, &input).unwrap();

        let mut output = vec![0.0f32; 64];
        let n = buf.read_f32(0, &mut output).unwrap();
        assert_eq!(n, 64);
        assert!((output[0] - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_inprocess_audio_buffer_f64() {
        let buf = InProcessAudioBuffer::new(2, 64);

        let input = vec![0.75f64; 64];
        buf.write_f64(0, &input).unwrap();

        let mut output = vec![0.0f64; 64];
        let n = buf.read_f64(0, &mut output).unwrap();
        assert_eq!(n, 64);
        assert!((output[0] - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn test_inprocess_audio_buffer_out_of_bounds() {
        let buf = InProcessAudioBuffer::new(2, 64);
        assert!(buf.write_f32(5, &[1.0]).is_err());
        assert!(buf.read_f32(5, &mut [0.0]).is_err());
    }

    #[test]
    fn test_inprocess_process_audio() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);

        // Write input data
        let input = vec![1.0f32; 64];
        bridge.write_input_channel(0, &input).unwrap();
        bridge.write_input_channel(1, &input).unwrap();

        // Process
        let ok = bridge.process(
            64,
            smallvec::SmallVec::new(),
            ParameterChanges::new(),
            NoteExpressionChanges::new(),
            TransportInfo::default(),
        );
        assert!(ok);

        // Read output — MockPlugin multiplies by 0.5
        let mut output = vec![0.0f32; 64];
        let n = bridge.read_output_channel_into(0, &mut output).unwrap();
        assert!(n >= 64);
        assert!(
            (output[0] - 0.5).abs() < f32::EPSILON,
            "Expected 0.5, got {}",
            output[0]
        );
    }

    #[test]
    fn test_inprocess_set_sample_rate() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);

        assert!(bridge.set_sample_rate_rt(96000.0));
    }

    #[test]
    fn test_inprocess_reset() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);

        assert!(bridge.reset_rt());
    }

    #[test]
    fn test_inprocess_shutdown_on_drop() {
        let plugin = Box::new(MockPlugin::new());
        let (_bridge, handle) = InProcessBridge::new(plugin, 2, 512);
        drop(handle);
    }

    // =========================================================================
    // Editor lifecycle via PluginHandle
    // =========================================================================

    fn make_handle(bridge: InProcessBridge) -> crate::handle::PluginHandle {
        use crate::PluginMetadata;
        let metadata = PluginMetadata::new("mock.plugin", "Mock Plugin")
            .audio_io(2, 2)
            .editor(true, Some((800, 600)));
        crate::handle::PluginHandle::from_bridge_and_metadata(
            Arc::new(bridge) as Arc<dyn PluginBridge>,
            metadata,
        )
    }

    #[test]
    fn test_handle_editor_open_close_lifecycle() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);
        let ph = make_handle(bridge);

        assert!(ph.has_editor());
        let size = ph.open_editor(0xDEAD);
        assert_eq!(size, Some((800, 600)));

        // close_editor returns &Self for chaining
        let ret = ph.close_editor();
        assert_eq!(ret.name(), "Mock Plugin");
    }

    #[test]
    fn test_handle_editor_idle_chaining() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);
        let ph = make_handle(bridge);

        ph.open_editor(0x1234);
        // editor_idle returns &Self, allowing chaining
        ph.editor_idle().editor_idle().editor_idle();
        ph.close_editor();
    }

    #[test]
    fn test_handle_editor_reopen_after_close() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);
        let ph = make_handle(bridge);

        // First cycle
        assert_eq!(ph.open_editor(0x1), Some((800, 600)));
        ph.close_editor();

        // Second cycle — should work the same
        assert_eq!(ph.open_editor(0x2), Some((800, 600)));
        ph.editor_idle();
        ph.close_editor();
    }

    #[test]
    fn test_handle_editor_idle_without_open() {
        // Calling idle/close without open should not panic
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);
        let ph = make_handle(bridge);

        ph.editor_idle();
        ph.close_editor();
    }

    // =========================================================================
    // PluginHandle fluent API chaining
    // =========================================================================

    #[test]
    fn test_handle_fluent_parameter_chaining() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);
        let ph = make_handle(bridge);

        // All set_parameter calls return &Self
        ph.set_parameter(0, 0.3)
            .set_parameter(1, 0.9)
            .set_parameter(0, 0.7);

        thread::sleep(Duration::from_millis(20));

        // Verify final values were applied
        let v0 = ph.get_parameter(0);
        assert!(v0.is_some());
        assert!((v0.unwrap() - 0.7).abs() < 0.01);

        let v1 = ph.get_parameter(1);
        assert!(v1.is_some());
        assert!((v1.unwrap() - 0.9).abs() < 0.01);
    }

    #[test]
    fn test_handle_fluent_state_chaining() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);
        let ph = make_handle(bridge);

        // load_state returns &Self for chaining
        ph.load_state(&[10, 20, 30]).load_state(&[40, 50]);

        let state = ph.save_state();
        assert_eq!(state, Some(vec![40, 50]));
    }

    #[test]
    fn test_handle_fluent_mixed_chaining() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);
        let ph = make_handle(bridge);

        // Mix parameter, state, and editor operations
        ph.set_parameter(0, 0.5)
            .set_parameter(1, 0.2)
            .load_state(&[7, 8, 9])
            .close_editor()
            .editor_idle();

        let state = ph.save_state();
        assert_eq!(state, Some(vec![7, 8, 9]));
    }

    // =========================================================================
    // Crash recovery
    // =========================================================================

    #[test]
    fn test_crash_flag_blocks_editor_operations() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);

        // Simulate crash
        bridge.crashed.store(true, Ordering::Release);

        // All editor operations should fail gracefully
        assert_eq!(bridge.open_editor(0x1234), None);
        assert!(!bridge.close_editor());
        // editor_idle returns () — just verify it doesn't panic
        bridge.editor_idle();
    }

    #[test]
    fn test_crash_flag_blocks_state_operations() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);

        bridge.crashed.store(true, Ordering::Release);

        assert!(bridge.save_state().is_none());
        assert!(!bridge.load_state(&[1, 2, 3]));
    }

    #[test]
    fn test_crash_flag_blocks_parameter_operations() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);

        bridge.crashed.store(true, Ordering::Release);

        assert!(bridge.get_parameter_list().is_none());
        assert!(bridge.get_parameter(0).is_none());
        assert!(!bridge.set_parameter_rt(0, 0.5));
    }

    #[test]
    fn test_crash_flag_blocks_audio_operations() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);

        bridge.crashed.store(true, Ordering::Release);

        assert!(!bridge.set_sample_rate_rt(48000.0));
        assert!(!bridge.reset_rt());
        assert!(!bridge.process(
            64,
            smallvec::SmallVec::new(),
            ParameterChanges::new(),
            NoteExpressionChanges::new(),
            TransportInfo::default(),
        ));
    }

    #[test]
    fn test_handle_crash_flag_propagates() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);
        let ph = make_handle(bridge.clone());

        assert!(!ph.is_crashed());

        bridge.crashed.store(true, Ordering::Release);

        assert!(ph.is_crashed());
        assert!(ph.open_editor(0x1).is_none());
        assert!(ph.save_state().is_none());
        assert!(ph.get_parameter(0).is_none());
        assert!(ph.parameters().is_none());
    }

    // =========================================================================
    // Parameter change detection (snapshot comparison)
    // =========================================================================

    #[test]
    fn test_parameter_change_detection() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);
        let ph = make_handle(bridge);

        // Take initial snapshot
        let params = ph.parameters().unwrap();
        let snapshot: Vec<(u32, f32)> = params
            .iter()
            .map(|p| (p.id, ph.get_parameter(p.id).unwrap_or(0.0)))
            .collect();

        // Change one parameter
        ph.set_parameter(0, 0.1);
        thread::sleep(Duration::from_millis(20));

        // Detect changes by comparing snapshots
        let threshold = 0.001f32;
        let mut changed = Vec::new();
        for (id, old_val) in &snapshot {
            if let Some(new_val) = ph.get_parameter(*id) {
                if (new_val - old_val).abs() > threshold {
                    changed.push((*id, *old_val, new_val));
                }
            }
        }

        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].0, 0); // param id
        assert!((changed[0].1 - 0.5).abs() < 0.01); // old value
        assert!((changed[0].2 - 0.1).abs() < 0.01); // new value
    }

    // =========================================================================
    // GUI direct dispatch (not queued)
    // =========================================================================

    #[test]
    fn test_gui_methods_work_when_command_queue_full() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);

        // Fill the command queue to capacity
        for _ in 0..COMMAND_QUEUE_SIZE {
            let _ = bridge.command_queue.push(BridgeCommand::SetParameter {
                param_id: 0,
                value: 0.5,
            });
        }

        // Verify queue is actually full
        assert!(bridge.command_queue.push(BridgeCommand::Reset).is_err());

        // GUI methods bypass the queue — they should still work
        let size = bridge.open_editor(0xBEEF);
        assert_eq!(size, Some((800, 600)));

        bridge.editor_idle();
        assert!(bridge.close_editor());

        // State/param queries also bypass the queue
        let state = bridge.save_state();
        assert!(state.is_some());

        let params = bridge.get_parameter_list();
        assert!(params.is_some());
    }

    // =========================================================================
    // Concurrent audio + GUI
    // =========================================================================

    #[test]
    fn test_concurrent_audio_and_gui() {
        let plugin = Box::new(MockPlugin::new());
        let (bridge, _thread_handle) = InProcessBridge::new(plugin, 2, 512);
        let bridge2 = bridge.clone();

        // Spawn a thread that does audio processing
        let audio_thread = thread::spawn(move || {
            let input = vec![1.0f32; 64];
            for _ in 0..5 {
                bridge2.write_input_channel(0, &input).unwrap();
                bridge2.write_input_channel(1, &input).unwrap();
                let ok = bridge2.process(
                    64,
                    smallvec::SmallVec::new(),
                    ParameterChanges::new(),
                    NoteExpressionChanges::new(),
                    TransportInfo::default(),
                );
                assert!(ok);
            }
        });

        // Meanwhile, do GUI operations on the "main" thread
        for _ in 0..5 {
            let _ = bridge.open_editor(0x1);
            bridge.editor_idle();
            bridge.close_editor();
            thread::sleep(Duration::from_millis(2));
        }

        audio_thread.join().unwrap();
    }

    // =========================================================================
    // f64 processing path
    // =========================================================================

    #[test]
    fn test_inprocess_process_audio_f64() {
        let plugin: Box<dyn PluginInstance> = Box::new(MockPluginF64);
        let (bridge, _handle) = InProcessBridge::new(plugin, 2, 512);

        // Write f64 input data
        let input = vec![1.0f64; 64];
        bridge.write_input_channel_f64(0, &input).unwrap();
        bridge.write_input_channel_f64(1, &input).unwrap();

        // Process
        let ok = bridge.process(
            64,
            smallvec::SmallVec::new(),
            ParameterChanges::new(),
            NoteExpressionChanges::new(),
            TransportInfo::default(),
        );
        assert!(ok);

        // Read f64 output — MockPluginF64 multiplies by 0.25
        let mut output = vec![0.0f64; 64];
        let n = bridge.read_output_channel_into_f64(0, &mut output).unwrap();
        assert!(n >= 64);
        assert!(
            (output[0] - 0.25).abs() < f64::EPSILON,
            "Expected 0.25, got {}",
            output[0]
        );
    }
}
