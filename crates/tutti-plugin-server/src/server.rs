//! Bridge server - runs in isolated process
//!
//! The server loads and runs plugins in a sandboxed environment.

use crate::instance::{PluginInstance, ProcessContext};
use crate::transport::{MessageTransport, TransportListener};
use std::path::PathBuf;

use tutti_plugin::{BridgeConfig, BridgeError, LoadStage, PluginMetadata, Result, SampleFormat};
use tutti_plugin::protocol::{BridgeMessage, HostMessage, IpcMidiEvent};
use tutti_plugin::shared_memory::SharedAudioBuffer;

#[cfg(feature = "vst2")]
use crate::vst2_loader::Vst2Instance;

#[cfg(feature = "vst3")]
use crate::vst3_loader::Vst3Instance;

#[cfg(feature = "clap")]
use crate::clap_loader::ClapInstance;

/// VST bridge server (runs in isolated process)
pub struct PluginServer {
    config: BridgeConfig,
    transport: Option<MessageTransport>,

    plugin: Option<LoadedPlugin>,

    shared_buffer: Option<SharedAudioBuffer>,

    editor_open: bool,

    /// Negotiated sample format for processing
    negotiated_format: SampleFormat,

    /// Current sample rate (f64 for precision, matches VST3/CLAP)
    sample_rate: f64,

    // Pre-allocated audio buffers (RT-safe: resize only on config change)
    input_buffers_f32: Vec<Vec<f32>>,
    output_buffers_f32: Vec<Vec<f32>>,
    input_buffers_f64: Vec<Vec<f64>>,
    output_buffers_f64: Vec<Vec<f64>>,

    current_num_channels: usize,
    current_buffer_size: usize,

    /// Pre-allocated MIDI output buffer (RT-safe: clear() reuses allocation)
    midi_output_buffer: tutti_plugin::protocol::MidiEventVec,
}

#[cfg(not(any(feature = "vst2", feature = "vst3", feature = "clap")))]
compile_error!(
    "tutti-plugin requires at least one plugin format. Enable with: --features vst2,vst3,clap"
);

enum LoadedPlugin {
    #[cfg(feature = "vst2")]
    Vst2(Vst2Instance),

    #[cfg(feature = "vst3")]
    Vst3(Vst3Instance),

    #[cfg(feature = "clap")]
    Clap(ClapInstance),
}

impl LoadedPlugin {
    fn as_instance_mut(&mut self) -> &mut dyn PluginInstance {
        match self {
            #[cfg(feature = "vst2")]
            LoadedPlugin::Vst2(p) => p,
            #[cfg(feature = "vst3")]
            LoadedPlugin::Vst3(p) => p,
            #[cfg(feature = "clap")]
            LoadedPlugin::Clap(p) => p,
        }
    }

    fn as_instance(&self) -> &dyn PluginInstance {
        match self {
            #[cfg(feature = "vst2")]
            LoadedPlugin::Vst2(p) => p,
            #[cfg(feature = "vst3")]
            LoadedPlugin::Vst3(p) => p,
            #[cfg(feature = "clap")]
            LoadedPlugin::Clap(p) => p,
        }
    }
}

impl PluginServer {
    pub async fn new(config: BridgeConfig) -> Result<Self> {
        Ok(Self {
            config,
            transport: None,
            plugin: None,
            shared_buffer: None,
            editor_open: false,
            negotiated_format: SampleFormat::Float32,
            sample_rate: 44100.0, // Default, updated when plugin is loaded
            input_buffers_f32: Vec::new(),
            output_buffers_f32: Vec::new(),
            input_buffers_f64: Vec::new(),
            output_buffers_f64: Vec::new(),
            current_num_channels: 0,
            current_buffer_size: 0,
            midi_output_buffer: smallvec::SmallVec::new(),
        })
    }

    /// Ensure audio buffers are sized correctly (only reallocates if size changed).
    fn ensure_buffers_sized(&mut self, num_channels: usize, buffer_size: usize) {
        if self.current_num_channels != num_channels || self.current_buffer_size != buffer_size {
            self.input_buffers_f32.clear();
            self.output_buffers_f32.clear();
            for _ in 0..num_channels {
                self.input_buffers_f32.push(vec![0.0f32; buffer_size]);
                self.output_buffers_f32.push(vec![0.0f32; buffer_size]);
            }

            self.input_buffers_f64.clear();
            self.output_buffers_f64.clear();
            for _ in 0..num_channels {
                self.input_buffers_f64.push(vec![0.0f64; buffer_size]);
                self.output_buffers_f64.push(vec![0.0f64; buffer_size]);
            }

            self.current_num_channels = num_channels;
            self.current_buffer_size = buffer_size;
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        let listener = TransportListener::bind(&self.config.socket_path).await?;
        let mut transport = listener.accept().await?;
        transport.send_bridge_message(&BridgeMessage::Ready).await?;
        self.transport = Some(transport);

        loop {
            let msg = self
                .transport
                .as_mut()
                .expect("BUG: transport should be Some (set on line 131)")
                .recv_host_message()
                .await?;

            let response = self.handle_message(msg).await?;

            if let Some(response) = response {
                self.transport
                    .as_mut()
                    .expect("BUG: transport should be Some (set on line 131)")
                    .send_bridge_message(&response)
                    .await?;
            }

            self.poll_parameter_changes().await?;

            if self.transport.is_none() {
                break;
            }
        }

        Ok(())
    }

    async fn handle_message(&mut self, msg: HostMessage) -> Result<Option<BridgeMessage>> {
        match msg {
            HostMessage::LoadPlugin {
                path,
                sample_rate,
                block_size,
                preferred_format,
                shm_name,
            } => {
                let metadata = self.load_plugin(path, sample_rate, block_size, preferred_format, &shm_name)?;

                Ok(Some(BridgeMessage::PluginLoaded {
                    metadata: Box::new(metadata),
                }))
            }

            HostMessage::UnloadPlugin => {
                self.plugin = None;
                self.shared_buffer = None;
                Ok(None)
            }

            HostMessage::ProcessAudio {
                buffer_id,
                num_samples,
            } => {
                // Forward to ProcessAudioMidi with empty MIDI
                return self.handle_process_audio(buffer_id, num_samples, &[]).await;
            }

            HostMessage::ProcessAudioMidi(data) => {
                let midi: tutti_plugin::protocol::MidiEventVec = data
                    .midi_events
                    .iter()
                    .filter_map(|e| e.to_midi_event())
                    .collect();
                return self
                    .handle_process_audio(data.buffer_id, data.num_samples, &midi)
                    .await;
            }

            HostMessage::ProcessAudioFull(data) => {
                let midi: tutti_plugin::protocol::MidiEventVec = data
                    .midi_events
                    .iter()
                    .filter_map(|e| e.to_midi_event())
                    .collect();
                return self
                    .handle_process_audio_full(
                        data.buffer_id,
                        data.num_samples,
                        &midi,
                        &data.param_changes,
                        &data.note_expression,
                        &data.transport,
                    )
                    .await;
            }

            HostMessage::SetParameter { param_id, value } => {
                if let Some(ref mut plugin) = self.plugin {
                    // Use unified trait method - all loaders use param_id directly
                    plugin
                        .as_instance_mut()
                        .set_parameter(param_id, value as f64);
                }

                Ok(None)
            }

            HostMessage::GetParameter { param_id } => {
                let value = self
                    .plugin
                    .as_mut()
                    .map(|plugin| plugin.as_instance_mut().get_parameter(param_id) as f32);

                Ok(Some(BridgeMessage::ParameterValue { value }))
            }

            HostMessage::GetParameterList => {
                let parameters = if let Some(ref mut plugin) = self.plugin {
                    plugin.as_instance_mut().get_parameter_list()
                } else {
                    Vec::new()
                };

                Ok(Some(BridgeMessage::ParameterList { parameters }))
            }

            HostMessage::GetParameterInfo { param_id } => {
                let info = if let Some(ref mut plugin) = self.plugin {
                    plugin.as_instance_mut().get_parameter_info(param_id)
                } else {
                    None
                };

                Ok(Some(BridgeMessage::ParameterInfoResponse { info }))
            }

            HostMessage::OpenEditor { parent_handle } => {
                if let Some(ref mut plugin) = self.plugin {
                    // Convert parent_handle u64 to raw pointer
                    let parent_ptr = parent_handle as *mut std::ffi::c_void;
                    // Safety: parent_handle was provided by the host and is expected to be a valid window handle
                    let result = unsafe { plugin.as_instance_mut().open_editor(parent_ptr) };

                    match result {
                        Ok((width, height)) => {
                            self.editor_open = true;
                            Ok(Some(BridgeMessage::EditorOpened { width, height }))
                        }
                        Err(e) => Ok(Some(BridgeMessage::Error {
                            message: format!("Failed to open editor: {}", e),
                        })),
                    }
                } else {
                    Ok(Some(BridgeMessage::Error {
                        message: "No plugin loaded".to_string(),
                    }))
                }
            }

            HostMessage::CloseEditor => {
                if let Some(ref mut plugin) = self.plugin {
                    plugin.as_instance_mut().close_editor();
                    self.editor_open = false;
                }

                Ok(None)
            }

            HostMessage::EditorIdle => {
                if self.editor_open {
                    if let Some(ref mut plugin) = self.plugin {
                        plugin.as_instance_mut().editor_idle();
                    }
                }
                Ok(None)
            }

            HostMessage::SetSampleRate { rate } => {
                if let Some(ref mut plugin) = self.plugin {
                    plugin.as_instance_mut().set_sample_rate(rate);
                }

                Ok(None)
            }

            HostMessage::Reset => Ok(None),

            HostMessage::SaveState => {
                if let Some(ref mut plugin) = self.plugin {
                    match plugin.as_instance_mut().get_state() {
                        Ok(data) => Ok(Some(BridgeMessage::StateData { data })),
                        Err(e) => Ok(Some(BridgeMessage::Error {
                            message: format!("Failed to save state: {}", e),
                        })),
                    }
                } else {
                    Ok(Some(BridgeMessage::StateData { data: vec![] }))
                }
            }

            HostMessage::LoadState { data } => {
                if let Some(ref mut plugin) = self.plugin {
                    match plugin.as_instance_mut().set_state(&data) {
                        Ok(()) => Ok(None),
                        Err(e) => Ok(Some(BridgeMessage::Error {
                            message: format!("Failed to load state: {}", e),
                        })),
                    }
                } else {
                    Ok(None)
                }
            }

            HostMessage::Shutdown => {
                self.transport = None;
                Ok(None)
            }
        }
    }

    async fn handle_process_audio(
        &mut self,
        _buffer_id: u32,
        num_samples: usize,
        midi_events: &[tutti_plugin::protocol::MidiEvent],
    ) -> Result<Option<BridgeMessage>> {
        if self.plugin.is_none() {
            return Ok(Some(BridgeMessage::Error {
                message: "No plugin loaded".to_string(),
            }));
        }

        let start = std::time::Instant::now();

        self.midi_output_buffer.clear();

        if let (Some(_), Some(ref plugin)) = (&self.shared_buffer, &self.plugin) {
            let metadata = plugin.as_instance().metadata();
            let num_channels = metadata.audio_io.inputs.max(metadata.audio_io.outputs);
            self.ensure_buffers_sized(num_channels, num_samples);
        }

        if let (Some(ref mut shared_buffer), Some(ref mut plugin)) =
            (&mut self.shared_buffer, &mut self.plugin)
        {
            let metadata = plugin.as_instance().metadata();
            let num_channels = metadata.audio_io.inputs.max(metadata.audio_io.outputs);

            let ctx = ProcessContext::new().midi(midi_events);

            match self.negotiated_format {
                SampleFormat::Float64 => {
                    for ch in 0..num_channels {
                        if let Ok(data) = shared_buffer.read_channel_f64(ch) {
                            self.input_buffers_f64[ch][..num_samples]
                                .copy_from_slice(&data[..num_samples]);
                            self.output_buffers_f64[ch][..num_samples].fill(0.0);
                        }
                    }

                    let input_slices: Vec<&[f64]> = self.input_buffers_f64[..num_channels]
                        .iter()
                        .map(|v| &v[..num_samples])
                        .collect();
                    let mut output_slices: Vec<&mut [f64]> = self.output_buffers_f64
                        [..num_channels]
                        .iter_mut()
                        .map(|v| &mut v[..num_samples])
                        .collect();

                    let sample_rate = self.sample_rate;
                    let mut audio_buffer = tutti_plugin::protocol::AudioBuffer64 {
                        inputs: &input_slices,
                        outputs: &mut output_slices,
                        num_samples,
                        sample_rate,
                    };

                    let output = plugin
                        .as_instance_mut()
                        .process_f64(&mut audio_buffer, &ctx);
                    self.midi_output_buffer = output.midi_events;

                    for ch in 0..num_channels {
                        let _ = shared_buffer
                            .write_channel_f64(ch, &self.output_buffers_f64[ch][..num_samples]);
                    }
                }
                SampleFormat::Float32 => {
                    for ch in 0..num_channels {
                        if let Ok(data) = shared_buffer.read_channel(ch) {
                            self.input_buffers_f32[ch][..num_samples]
                                .copy_from_slice(&data[..num_samples]);
                            self.output_buffers_f32[ch][..num_samples].fill(0.0);
                        }
                    }

                    let input_slices: Vec<&[f32]> = self.input_buffers_f32[..num_channels]
                        .iter()
                        .map(|v| &v[..num_samples])
                        .collect();
                    let mut output_slices: Vec<&mut [f32]> = self.output_buffers_f32
                        [..num_channels]
                        .iter_mut()
                        .map(|v| &mut v[..num_samples])
                        .collect();

                    let sample_rate = self.sample_rate;
                    let mut audio_buffer = tutti_plugin::protocol::AudioBuffer {
                        inputs: &input_slices,
                        outputs: &mut output_slices,
                        num_samples,
                        sample_rate,
                    };

                    let output = plugin
                        .as_instance_mut()
                        .process_f32(&mut audio_buffer, &ctx);
                    self.midi_output_buffer = output.midi_events;

                    for ch in 0..num_channels {
                        let _ = shared_buffer
                            .write_channel(ch, &self.output_buffers_f32[ch][..num_samples]);
                    }
                }
            }
        }

        let latency_us = start.elapsed().as_micros() as u64;

        if !self.midi_output_buffer.is_empty() {
            Ok(Some(BridgeMessage::AudioProcessedMidi(Box::new(
                tutti_plugin::protocol::AudioProcessedMidiData {
                    latency_us,
                    midi_output: self
                        .midi_output_buffer
                        .iter()
                        .map(IpcMidiEvent::from)
                        .collect(),
                },
            ))))
        } else {
            Ok(Some(BridgeMessage::AudioProcessed { latency_us }))
        }
    }

    async fn handle_process_audio_full(
        &mut self,
        _buffer_id: u32,
        num_samples: usize,
        midi_events: &[tutti_plugin::protocol::MidiEvent],
        param_changes: &tutti_plugin::protocol::ParameterChanges,
        note_expression: &tutti_plugin::protocol::NoteExpressionChanges,
        transport: &tutti_plugin::protocol::TransportInfo,
    ) -> Result<Option<BridgeMessage>> {
        if self.plugin.is_none() {
            return Ok(Some(BridgeMessage::Error {
                message: "No plugin loaded".to_string(),
            }));
        }

        let start = std::time::Instant::now();

        self.midi_output_buffer.clear();
        let mut param_output = tutti_plugin::protocol::ParameterChanges::new();
        let mut note_expression_output = tutti_plugin::protocol::NoteExpressionChanges::new();

        // Pre-size buffers before borrowing shared_buffer and plugin
        if let Some(ref plugin) = self.plugin {
            let metadata = plugin.as_instance().metadata();
            let num_channels = metadata.audio_io.inputs.max(metadata.audio_io.outputs);
            self.ensure_buffers_sized(num_channels, num_samples);
        }

        if let (Some(ref mut shared_buffer), Some(ref mut plugin)) =
            (&mut self.shared_buffer, &mut self.plugin)
        {
            let metadata = plugin.as_instance().metadata();
            let num_channels = metadata.audio_io.inputs.max(metadata.audio_io.outputs);

            let ctx = ProcessContext::new()
                .midi(midi_events)
                .params(param_changes)
                .note_expression(note_expression)
                .transport(transport);

            match self.negotiated_format {
                SampleFormat::Float64 => {
                    for ch in 0..num_channels {
                        if let Ok(data) = shared_buffer.read_channel_f64(ch) {
                            self.input_buffers_f64[ch][..num_samples]
                                .copy_from_slice(&data[..num_samples]);
                            self.output_buffers_f64[ch][..num_samples].fill(0.0);
                        }
                    }

                    let input_slices: Vec<&[f64]> = self.input_buffers_f64[..num_channels]
                        .iter()
                        .map(|v| &v[..num_samples])
                        .collect();
                    let mut output_slices: Vec<&mut [f64]> = self.output_buffers_f64
                        [..num_channels]
                        .iter_mut()
                        .map(|v| &mut v[..num_samples])
                        .collect();

                    let sample_rate = self.sample_rate;
                    let mut audio_buffer = tutti_plugin::protocol::AudioBuffer64 {
                        inputs: &input_slices,
                        outputs: &mut output_slices,
                        num_samples,
                        sample_rate,
                    };

                    let output = plugin
                        .as_instance_mut()
                        .process_f64(&mut audio_buffer, &ctx);
                    self.midi_output_buffer = output.midi_events;
                    param_output = output.param_changes;
                    note_expression_output = output.note_expression;

                    for ch in 0..num_channels {
                        let _ = shared_buffer
                            .write_channel_f64(ch, &self.output_buffers_f64[ch][..num_samples]);
                    }
                }
                SampleFormat::Float32 => {
                    for ch in 0..num_channels {
                        if let Ok(data) = shared_buffer.read_channel(ch) {
                            self.input_buffers_f32[ch][..num_samples]
                                .copy_from_slice(&data[..num_samples]);
                            self.output_buffers_f32[ch][..num_samples].fill(0.0);
                        }
                    }

                    let input_slices: Vec<&[f32]> = self.input_buffers_f32[..num_channels]
                        .iter()
                        .map(|v| &v[..num_samples])
                        .collect();
                    let mut output_slices: Vec<&mut [f32]> = self.output_buffers_f32
                        [..num_channels]
                        .iter_mut()
                        .map(|v| &mut v[..num_samples])
                        .collect();

                    let sample_rate = self.sample_rate;
                    let mut audio_buffer = tutti_plugin::protocol::AudioBuffer {
                        inputs: &input_slices,
                        outputs: &mut output_slices,
                        num_samples,
                        sample_rate,
                    };

                    let output = plugin
                        .as_instance_mut()
                        .process_f32(&mut audio_buffer, &ctx);
                    self.midi_output_buffer = output.midi_events;
                    param_output = output.param_changes;
                    note_expression_output = output.note_expression;

                    for ch in 0..num_channels {
                        let _ = shared_buffer
                            .write_channel(ch, &self.output_buffers_f32[ch][..num_samples]);
                    }
                }
            }
        }

        let latency_us = start.elapsed().as_micros() as u64;

        Ok(Some(BridgeMessage::AudioProcessedFull(Box::new(
            tutti_plugin::protocol::AudioProcessedFullData {
                latency_us,
                midi_output: self
                    .midi_output_buffer
                    .iter()
                    .map(IpcMidiEvent::from)
                    .collect(),
                param_output,
                note_expression_output,
            },
        ))))
    }

    async fn poll_parameter_changes(&mut self) -> Result<()> {
        #[cfg(feature = "vst2")]
        if let Some(LoadedPlugin::Vst2(vst2)) = &self.plugin {
            let changes = vst2.poll_parameter_changes();
            for (index, value) in changes {
                if let Some(ref mut transport) = self.transport {
                    transport
                        .send_bridge_message(&BridgeMessage::ParameterChanged { index, value })
                        .await?;
                }
            }
        }

        Ok(())
    }

    fn load_plugin(
        &mut self,
        path: PathBuf,
        sample_rate: f64,
        block_size: usize,
        preferred_format: SampleFormat,
        shm_name: &str,
    ) -> Result<PluginMetadata> {
        self.sample_rate = sample_rate;

        if !path.exists() {
            return Err(BridgeError::LoadFailed {
                path: path.to_path_buf(),
                stage: LoadStage::Scanning,
                reason: "Plugin not found".to_string(),
            });
        }

        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let (plugin, metadata): (LoadedPlugin, PluginMetadata) = match extension
            .to_lowercase()
            .as_str()
        {
            #[cfg(feature = "vst3")]
            "vst3" => {
                let mut vst = Vst3Instance::load(&path, sample_rate, block_size)?;
                if preferred_format == SampleFormat::Float64 && vst.can_process_f64() {
                    let _ = vst.set_sample_format(SampleFormat::Float64);
                }
                let metadata = vst.metadata().clone();
                (LoadedPlugin::Vst3(vst), metadata)
            }

            #[cfg(feature = "vst2")]
            "vst" | "dll" | "so" => {
                let vst = Vst2Instance::load(&path, sample_rate, block_size)?;
                let metadata = vst.metadata().clone();
                (LoadedPlugin::Vst2(vst), metadata)
            }

            #[cfg(feature = "clap")]
            "clap" => {
                let clap = ClapInstance::load(&path, sample_rate, block_size)?;
                let metadata = clap.metadata().clone();
                (LoadedPlugin::Clap(clap), metadata)
            }

            _ => {
                return Err(BridgeError::LoadFailed {
                        path: path.to_path_buf(),
                        stage: LoadStage::Opening,
                        reason: format!(
                            "Unsupported plugin format: {}. Supported: .vst3, .vst/.dll/.so (VST2), .clap",
                            extension
                        ),
                    });
            }
        };

        let negotiated_format =
            if preferred_format == SampleFormat::Float64 && metadata.supports_f64 {
                SampleFormat::Float64
            } else {
                SampleFormat::Float32
            };
        self.negotiated_format = negotiated_format;

        // Use the shared memory name provided by the client
        let buffer_name = if shm_name.is_empty() {
            format!("dawai_plugin_{}", std::process::id())
        } else {
            shm_name.to_string()
        };
        let max_samples = 8192;
        let shared_buffer = SharedAudioBuffer::open_with_format(
            buffer_name,
            metadata.audio_io.inputs.max(metadata.audio_io.outputs),
            max_samples,
            negotiated_format,
        )?;

        self.shared_buffer = Some(shared_buffer);
        self.plugin = Some(plugin);

        Ok(metadata)
    }
}

impl Drop for PluginServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.config.socket_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tutti_plugin::protocol::{
        BridgeMessage, HostMessage, IpcMidiEventVec, NoteExpressionChanges, ParameterChanges,
        TransportInfo,
    };

    /// Create a PluginServer for testing with a unique socket path per call site.
    async fn test_server(name: &str) -> PluginServer {
        let config = BridgeConfig {
            socket_path: std::env::temp_dir()
                .join(format!("test_server_{}_{}.sock", name, std::process::id())),
            ..Default::default()
        };
        PluginServer::new(config).await.unwrap()
    }

    // -----------------------------------------------------------------------
    // Server creation
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_new_server_defaults() {
        let server = test_server("defaults").await;
        assert_eq!(server.negotiated_format, SampleFormat::Float32);
        assert_eq!(server.sample_rate, 44100.0);
        assert!(server.plugin.is_none());
        assert!(server.transport.is_none());
    }

    // -----------------------------------------------------------------------
    // Message handling without plugin loaded
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_get_parameter_no_plugin() {
        let mut server = test_server("get_param").await;
        let result = server
            .handle_message(HostMessage::GetParameter { param_id: 0 })
            .await
            .unwrap();
        match result {
            Some(BridgeMessage::ParameterValue { value }) => {
                assert!(value.is_none());
            }
            other => panic!("Expected ParameterValue {{ value: None }}, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_get_parameter_list_no_plugin() {
        let mut server = test_server("get_param_list").await;
        let result = server
            .handle_message(HostMessage::GetParameterList)
            .await
            .unwrap();
        match result {
            Some(BridgeMessage::ParameterList { parameters }) => {
                assert!(parameters.is_empty());
            }
            other => panic!("Expected ParameterList with empty vec, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_get_parameter_info_no_plugin() {
        let mut server = test_server("get_param_info").await;
        let result = server
            .handle_message(HostMessage::GetParameterInfo { param_id: 0 })
            .await
            .unwrap();
        match result {
            Some(BridgeMessage::ParameterInfoResponse { info }) => {
                assert!(info.is_none());
            }
            other => panic!(
                "Expected ParameterInfoResponse {{ info: None }}, got {:?}",
                other
            ),
        }
    }

    #[tokio::test]
    async fn test_process_audio_no_plugin() {
        let mut server = test_server("proc_audio").await;
        let result = server
            .handle_message(HostMessage::ProcessAudio {
                buffer_id: 0,
                num_samples: 256,
            })
            .await
            .unwrap();
        match result {
            Some(BridgeMessage::Error { message }) => {
                assert!(
                    message.contains("No plugin loaded"),
                    "Expected 'No plugin loaded', got: {}",
                    message
                );
            }
            other => panic!("Expected Error message, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_process_audio_midi_no_plugin() {
        let mut server = test_server("proc_audio_midi").await;
        let result = server
            .handle_message(HostMessage::ProcessAudioMidi(Box::new(
                tutti_plugin::protocol::ProcessAudioMidiData {
                    buffer_id: 0,
                    num_samples: 256,
                    midi_events: IpcMidiEventVec::new(),
                },
            )))
            .await
            .unwrap();
        match result {
            Some(BridgeMessage::Error { message }) => {
                assert!(
                    message.contains("No plugin loaded"),
                    "Expected 'No plugin loaded', got: {}",
                    message
                );
            }
            other => panic!("Expected Error message, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_process_audio_full_no_plugin() {
        let mut server = test_server("proc_audio_full").await;
        let result = server
            .handle_message(HostMessage::ProcessAudioFull(Box::new(
                tutti_plugin::protocol::ProcessAudioFullData {
                    buffer_id: 0,
                    num_samples: 256,
                    midi_events: IpcMidiEventVec::new(),
                    param_changes: ParameterChanges::new(),
                    note_expression: NoteExpressionChanges::new(),
                    transport: TransportInfo::default(),
                },
            )))
            .await
            .unwrap();
        match result {
            Some(BridgeMessage::Error { message }) => {
                assert!(
                    message.contains("No plugin loaded"),
                    "Expected 'No plugin loaded', got: {}",
                    message
                );
            }
            other => panic!("Expected Error message, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_set_parameter_no_plugin() {
        let mut server = test_server("set_param").await;
        let result = server
            .handle_message(HostMessage::SetParameter {
                param_id: 0,
                value: 0.5,
            })
            .await
            .unwrap();
        assert!(result.is_none(), "SetParameter with no plugin should return None");
    }

    #[tokio::test]
    async fn test_set_sample_rate_no_plugin() {
        let mut server = test_server("set_sr").await;
        let result = server
            .handle_message(HostMessage::SetSampleRate { rate: 96000.0 })
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "SetSampleRate with no plugin should return None"
        );
    }

    #[tokio::test]
    async fn test_save_state_no_plugin() {
        let mut server = test_server("save_state").await;
        let result = server
            .handle_message(HostMessage::SaveState)
            .await
            .unwrap();
        match result {
            Some(BridgeMessage::StateData { data }) => {
                assert!(data.is_empty());
            }
            other => panic!("Expected StateData with empty data, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_load_state_no_plugin() {
        let mut server = test_server("load_state").await;
        let result = server
            .handle_message(HostMessage::LoadState {
                data: vec![1, 2, 3],
            })
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "LoadState with no plugin should return None"
        );
    }

    #[tokio::test]
    async fn test_open_editor_no_plugin() {
        let mut server = test_server("open_editor").await;
        let result = server
            .handle_message(HostMessage::OpenEditor { parent_handle: 0 })
            .await
            .unwrap();
        match result {
            Some(BridgeMessage::Error { message }) => {
                assert!(
                    message.contains("No plugin loaded"),
                    "Expected 'No plugin loaded', got: {}",
                    message
                );
            }
            other => panic!("Expected Error message, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_close_editor_no_plugin() {
        let mut server = test_server("close_editor").await;
        let result = server
            .handle_message(HostMessage::CloseEditor)
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "CloseEditor with no plugin should return None"
        );
    }

    #[tokio::test]
    async fn test_unload_plugin() {
        let mut server = test_server("unload").await;
        let result = server
            .handle_message(HostMessage::UnloadPlugin)
            .await
            .unwrap();
        assert!(result.is_none(), "UnloadPlugin should return None");
        assert!(server.plugin.is_none(), "Plugin should be None after unload");
        assert!(
            server.shared_buffer.is_none(),
            "SharedBuffer should be None after unload"
        );
    }

    #[tokio::test]
    async fn test_reset_no_plugin() {
        let mut server = test_server("reset").await;
        let result = server
            .handle_message(HostMessage::Reset)
            .await
            .unwrap();
        assert!(result.is_none(), "Reset should return None");
    }

    #[tokio::test]
    async fn test_shutdown_clears_transport() {
        let mut server = test_server("shutdown").await;
        let result = server
            .handle_message(HostMessage::Shutdown)
            .await
            .unwrap();
        assert!(result.is_none(), "Shutdown should return None");
        assert!(
            server.transport.is_none(),
            "Transport should be None after shutdown"
        );
    }

    // -----------------------------------------------------------------------
    // Buffer sizing
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_ensure_buffers_sized() {
        let mut server = test_server("buf_sized").await;
        server.ensure_buffers_sized(2, 512);

        assert_eq!(server.input_buffers_f32.len(), 2);
        assert_eq!(server.output_buffers_f32.len(), 2);
        assert_eq!(server.input_buffers_f64.len(), 2);
        assert_eq!(server.output_buffers_f64.len(), 2);

        for ch in 0..2 {
            assert_eq!(server.input_buffers_f32[ch].len(), 512);
            assert_eq!(server.output_buffers_f32[ch].len(), 512);
            assert_eq!(server.input_buffers_f64[ch].len(), 512);
            assert_eq!(server.output_buffers_f64[ch].len(), 512);
        }

        assert_eq!(server.current_num_channels, 2);
        assert_eq!(server.current_buffer_size, 512);
    }

    #[tokio::test]
    async fn test_ensure_buffers_no_realloc() {
        let mut server = test_server("buf_no_realloc").await;
        server.ensure_buffers_sized(2, 512);

        // Record pointers to the inner Vecs.
        let ptr_in_0 = server.input_buffers_f32[0].as_ptr();
        let ptr_out_0 = server.output_buffers_f32[0].as_ptr();

        // Call again with the same dimensions -- should not reallocate.
        server.ensure_buffers_sized(2, 512);

        assert_eq!(server.current_num_channels, 2);
        assert_eq!(server.current_buffer_size, 512);
        // Pointers should be the same because the early return skips reallocation.
        assert_eq!(
            server.input_buffers_f32[0].as_ptr(),
            ptr_in_0,
            "Input buffer should not reallocate when size is unchanged"
        );
        assert_eq!(
            server.output_buffers_f32[0].as_ptr(),
            ptr_out_0,
            "Output buffer should not reallocate when size is unchanged"
        );
    }

    #[tokio::test]
    async fn test_ensure_buffers_resize() {
        let mut server = test_server("buf_resize").await;
        server.ensure_buffers_sized(2, 256);
        assert_eq!(server.input_buffers_f32.len(), 2);
        assert_eq!(server.input_buffers_f32[0].len(), 256);
        assert_eq!(server.current_num_channels, 2);
        assert_eq!(server.current_buffer_size, 256);

        // Resize to larger dimensions.
        server.ensure_buffers_sized(4, 512);
        assert_eq!(server.input_buffers_f32.len(), 4);
        assert_eq!(server.output_buffers_f32.len(), 4);
        assert_eq!(server.input_buffers_f64.len(), 4);
        assert_eq!(server.output_buffers_f64.len(), 4);

        for ch in 0..4 {
            assert_eq!(server.input_buffers_f32[ch].len(), 512);
            assert_eq!(server.output_buffers_f32[ch].len(), 512);
            assert_eq!(server.input_buffers_f64[ch].len(), 512);
            assert_eq!(server.output_buffers_f64[ch].len(), 512);
        }

        assert_eq!(server.current_num_channels, 4);
        assert_eq!(server.current_buffer_size, 512);
    }

    // -----------------------------------------------------------------------
    // Plugin loading error paths
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_load_plugin_not_found() {
        let mut server = test_server("load_not_found").await;
        let result = server.load_plugin(
            PathBuf::from("/nonexistent/path/plugin.vst3"),
            44100.0,
            512,
            SampleFormat::Float32,
            "",
        );
        match result {
            Err(BridgeError::LoadFailed {
                path,
                stage,
                reason,
            }) => {
                assert_eq!(path, PathBuf::from("/nonexistent/path/plugin.vst3"));
                assert_eq!(stage, LoadStage::Scanning);
                assert!(
                    reason.contains("Plugin not found"),
                    "Expected 'Plugin not found', got: {}",
                    reason
                );
            }
            other => panic!("Expected LoadFailed error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_load_plugin_unsupported_format() {
        let tmp = std::env::temp_dir().join(format!(
            "fake_plugin_{}.xyz",
            std::process::id()
        ));
        std::fs::write(&tmp, b"fake").unwrap();

        let mut server = test_server("load_unsupported").await;
        let result = server.load_plugin(tmp.clone(), 44100.0, 512, SampleFormat::Float32, "");

        let _ = std::fs::remove_file(&tmp);

        match result {
            Err(BridgeError::LoadFailed {
                path,
                stage,
                reason,
            }) => {
                assert_eq!(path, tmp);
                assert_eq!(stage, LoadStage::Opening);
                assert!(
                    reason.contains("Unsupported plugin format"),
                    "Expected 'Unsupported plugin format', got: {}",
                    reason
                );
            }
            other => panic!("Expected LoadFailed error, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Integration tests (require TAL-NoiseMaker CLAP plugin installed)
    // -----------------------------------------------------------------------

    #[cfg(feature = "clap")]
    const CLAP_PLUGIN: &str = "/Library/Audio/Plug-Ins/CLAP/TAL-NoiseMaker.clap";

    /// Pre-create shared memory and load the CLAP plugin into a server.
    ///
    /// Returns (server, metadata, shared_memory_guard). The guard keeps the
    /// creator-side mmap alive, though the server also holds its own mapping.
    #[cfg(feature = "clap")]
    async fn load_clap_into_server(
        name: &str,
        preferred_format: SampleFormat,
    ) -> (PluginServer, PluginMetadata, SharedAudioBuffer) {
        let config = BridgeConfig {
            socket_path: std::env::temp_dir()
                .join(format!("integ_{}_{}.sock", name, std::process::id())),
            ..Default::default()
        };
        let mut server = PluginServer::new(config).await.unwrap();

        // Pre-create shared memory (load_plugin calls open_with_format)
        let buffer_name = format!("dawai_vst_buffer_{}", std::process::id());
        let shm = SharedAudioBuffer::create_with_format(
            buffer_name.clone(),
            2,
            8192,
            preferred_format,
        )
        .unwrap();

        let metadata = server
            .load_plugin(
                PathBuf::from(CLAP_PLUGIN),
                44100.0,
                512,
                preferred_format,
                &buffer_name,
            )
            .unwrap();

        (server, metadata, shm)
    }

    #[tokio::test]
    #[cfg(feature = "clap")]
    async fn test_load_clap_plugin() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let (_server, metadata, _shm) =
            load_clap_into_server("load_clap", SampleFormat::Float32).await;
        assert!(
            !metadata.name.is_empty(),
            "Plugin name should be non-empty, got: {:?}",
            metadata.name
        );
    }

    #[tokio::test]
    #[cfg(feature = "clap")]
    async fn test_load_clap_plugin_f64() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let (server, metadata, _shm) =
            load_clap_into_server("load_clap_f64", SampleFormat::Float64).await;
        assert!(
            !metadata.name.is_empty(),
            "Plugin name should be non-empty"
        );
        // If the plugin supports f64, the negotiated format should be Float64.
        if metadata.supports_f64 {
            assert_eq!(
                server.negotiated_format,
                SampleFormat::Float64,
                "Negotiated format should be Float64 when plugin supports it"
            );
        } else {
            // Plugin doesn't support f64 -- server falls back to f32.
            assert_eq!(
                server.negotiated_format,
                SampleFormat::Float32,
                "Negotiated format should fall back to Float32"
            );
        }
    }

    #[tokio::test]
    #[cfg(feature = "clap")]
    async fn test_get_parameter_list_with_plugin() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let (mut server, _metadata, _shm) =
            load_clap_into_server("get_param_list_clap", SampleFormat::Float32).await;

        let result = server
            .handle_message(HostMessage::GetParameterList)
            .await
            .unwrap();

        match result {
            Some(BridgeMessage::ParameterList { parameters }) => {
                assert!(
                    !parameters.is_empty(),
                    "TAL-NoiseMaker should expose parameters"
                );
            }
            other => panic!("Expected ParameterList, got {:?}", other),
        }
    }

    #[tokio::test]
    #[cfg(feature = "clap")]
    async fn test_get_parameter_with_plugin() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let (mut server, _metadata, _shm) =
            load_clap_into_server("get_param_clap", SampleFormat::Float32).await;

        // First, get the parameter list to find a valid param_id.
        let list_result = server
            .handle_message(HostMessage::GetParameterList)
            .await
            .unwrap();

        let param_id = match list_result {
            Some(BridgeMessage::ParameterList { ref parameters }) => {
                assert!(!parameters.is_empty(), "Need at least one parameter");
                parameters[0].id
            }
            other => panic!("Expected ParameterList, got {:?}", other),
        };

        // Now get the parameter value.
        let result = server
            .handle_message(HostMessage::GetParameter { param_id })
            .await
            .unwrap();

        match result {
            Some(BridgeMessage::ParameterValue { value }) => {
                assert!(
                    value.is_some(),
                    "Parameter value should be Some for a loaded plugin"
                );
            }
            other => panic!("Expected ParameterValue with Some(v), got {:?}", other),
        }
    }

    #[tokio::test]
    #[cfg(feature = "clap")]
    async fn test_get_parameter_info_with_plugin() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let (mut server, _metadata, _shm) =
            load_clap_into_server("get_param_info_clap", SampleFormat::Float32).await;

        // Get parameter list to find a valid id.
        let list_result = server
            .handle_message(HostMessage::GetParameterList)
            .await
            .unwrap();

        let param_id = match list_result {
            Some(BridgeMessage::ParameterList { ref parameters }) => {
                assert!(!parameters.is_empty(), "Need at least one parameter");
                parameters[0].id
            }
            other => panic!("Expected ParameterList, got {:?}", other),
        };

        let result = server
            .handle_message(HostMessage::GetParameterInfo { param_id })
            .await
            .unwrap();

        match result {
            Some(BridgeMessage::ParameterInfoResponse { info }) => {
                assert!(
                    info.is_some(),
                    "ParameterInfo should be Some for a valid param_id"
                );
                let info = info.unwrap();
                assert_eq!(info.id, param_id);
            }
            other => panic!("Expected ParameterInfoResponse, got {:?}", other),
        }
    }

    #[tokio::test]
    #[cfg(feature = "clap")]
    async fn test_set_parameter_with_plugin() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let (mut server, _metadata, _shm) =
            load_clap_into_server("set_param_clap", SampleFormat::Float32).await;

        // Query valid param_id from the plugin (CLAP uses opaque IDs, not indices)
        let list_result = server
            .handle_message(HostMessage::GetParameterList)
            .await
            .unwrap();

        let param_id = match list_result {
            Some(BridgeMessage::ParameterList { ref parameters }) => {
                assert!(!parameters.is_empty(), "Need at least one parameter");
                parameters[0].id
            }
            other => panic!("Expected ParameterList, got {:?}", other),
        };

        let result = server
            .handle_message(HostMessage::SetParameter {
                param_id,
                value: 0.5,
            })
            .await
            .unwrap();

        assert!(
            result.is_none(),
            "SetParameter should return None (no crash)"
        );
    }

    #[tokio::test]
    #[cfg(feature = "clap")]
    async fn test_save_load_state() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let (mut server, _metadata, _shm) =
            load_clap_into_server("save_load_state_clap", SampleFormat::Float32).await;

        // Save state.
        let save_result = server
            .handle_message(HostMessage::SaveState)
            .await
            .unwrap();

        let state_data = match save_result {
            Some(BridgeMessage::StateData { data }) => {
                assert!(
                    !data.is_empty(),
                    "State data should be non-empty for a loaded plugin"
                );
                data
            }
            other => panic!("Expected StateData, got {:?}", other),
        };

        // Load state back.
        let load_result = server
            .handle_message(HostMessage::LoadState { data: state_data })
            .await
            .unwrap();

        assert!(
            load_result.is_none(),
            "LoadState should return None on success"
        );
    }

    #[tokio::test]
    #[cfg(feature = "clap")]
    async fn test_set_sample_rate_with_plugin() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let (mut server, _metadata, _shm) =
            load_clap_into_server("set_sr_clap", SampleFormat::Float32).await;

        let result = server
            .handle_message(HostMessage::SetSampleRate { rate: 96000.0 })
            .await
            .unwrap();

        assert!(
            result.is_none(),
            "SetSampleRate should return None"
        );
    }

    #[tokio::test]
    #[cfg(feature = "clap")]
    async fn test_unload_loaded_plugin() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let (mut server, _metadata, _shm) =
            load_clap_into_server("unload_clap", SampleFormat::Float32).await;

        // Verify plugin is loaded.
        assert!(server.plugin.is_some(), "Plugin should be loaded");
        assert!(server.shared_buffer.is_some(), "Shared buffer should exist");

        let result = server
            .handle_message(HostMessage::UnloadPlugin)
            .await
            .unwrap();

        assert!(result.is_none(), "UnloadPlugin should return None");
        assert!(
            server.plugin.is_none(),
            "Plugin should be None after unload"
        );
        assert!(
            server.shared_buffer.is_none(),
            "Shared buffer should be None after unload"
        );
    }

    #[tokio::test]
    #[cfg(feature = "clap")]
    async fn test_process_audio_with_plugin() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let (mut server, _metadata, _shm) =
            load_clap_into_server("proc_audio_clap", SampleFormat::Float32).await;

        let result = server
            .handle_message(HostMessage::ProcessAudio {
                buffer_id: 0,
                num_samples: 512,
            })
            .await
            .unwrap();

        match result {
            Some(BridgeMessage::AudioProcessed { .. })
            | Some(BridgeMessage::AudioProcessedMidi(..)) => {
                // Either variant is acceptable (plugin may or may not output MIDI).
            }
            other => panic!(
                "Expected AudioProcessed or AudioProcessedMidi, got {:?}",
                other
            ),
        }
    }

    #[tokio::test]
    #[cfg(feature = "clap")]
    async fn test_process_audio_full_with_plugin() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let (mut server, _metadata, _shm) =
            load_clap_into_server("proc_audio_full_clap", SampleFormat::Float32).await;

        let result = server
            .handle_message(HostMessage::ProcessAudioFull(Box::new(
                tutti_plugin::protocol::ProcessAudioFullData {
                    buffer_id: 0,
                    num_samples: 512,
                    midi_events: IpcMidiEventVec::new(),
                    param_changes: ParameterChanges::new(),
                    note_expression: NoteExpressionChanges::new(),
                    transport: TransportInfo::default(),
                },
            )))
            .await
            .unwrap();

        match result {
            Some(BridgeMessage::AudioProcessedFull(..)) => {
                // Success -- AudioProcessedFull is always returned by handle_process_audio_full.
            }
            other => panic!("Expected AudioProcessedFull, got {:?}", other),
        }
    }

    #[tokio::test]
    #[cfg(feature = "clap")]
    async fn test_editor_check_with_plugin() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let (mut server, _metadata, _shm) =
            load_clap_into_server("editor_check_clap", SampleFormat::Float32).await;

        // Access the plugin instance directly to check has_editor().
        let plugin = server
            .plugin
            .as_mut()
            .expect("Plugin should be loaded");

        let _has_editor: bool = plugin.as_instance_mut().has_editor();
        // We only check it returns a bool without crashing. TAL-NoiseMaker
        // has an editor, but we do not open it in a headless test environment.
    }
}
