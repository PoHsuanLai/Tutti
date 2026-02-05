//! Bridge server - runs in isolated process
//!
//! The server loads and runs plugins in a sandboxed environment.

use crate::error::{BridgeError, LoadStage, Result};
use crate::instance::{PluginInstance, ProcessContext};
use crate::protocol::{BridgeConfig, BridgeMessage, HostMessage, IpcMidiEvent, PluginMetadata};
use crate::shared_memory::SharedAudioBuffer;
use crate::transport::{MessageTransport, TransportListener};
use std::path::PathBuf;

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

    // Plugin state (enum to support multiple formats)
    plugin: Option<LoadedPlugin>,

    shared_buffer: Option<SharedAudioBuffer>,

    // Editor state
    editor_open: bool,

    /// Negotiated sample format for processing
    negotiated_format: crate::protocol::SampleFormat,

    /// Current sample rate (f64 for precision, matches VST3/CLAP)
    sample_rate: f64,

    // Pre-allocated audio buffers (RT-safe: resize only on config change)
    /// Pre-allocated f32 input buffers (one per channel)
    input_buffers_f32: Vec<Vec<f32>>,
    /// Pre-allocated f32 output buffers (one per channel)
    output_buffers_f32: Vec<Vec<f32>>,
    /// Pre-allocated f64 input buffers (one per channel)
    input_buffers_f64: Vec<Vec<f64>>,
    /// Pre-allocated f64 output buffers (one per channel)
    output_buffers_f64: Vec<Vec<f64>>,

    /// Current buffer configuration
    current_num_channels: usize,
    current_buffer_size: usize,

    /// Pre-allocated MIDI output buffer (RT-safe: clear() reuses allocation)
    midi_output_buffer: crate::protocol::MidiEventVec,
}

// Compile-time error if no plugin formats enabled
#[cfg(not(any(feature = "vst2", feature = "vst3", feature = "clap")))]
compile_error!(
    "tutti-plugin requires at least one plugin format. Enable with: --features vst2,vst3,clap"
);

/// Loaded plugin instance (supports multiple formats)
enum LoadedPlugin {
    #[cfg(feature = "vst2")]
    Vst2(Vst2Instance),

    #[cfg(feature = "vst3")]
    Vst3(Vst3Instance),

    #[cfg(feature = "clap")]
    Clap(ClapInstance),
}

impl LoadedPlugin {
    /// Get mutable reference to plugin as trait object
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

    /// Get immutable reference to plugin as trait object
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
    /// Create new bridge server
    pub async fn new(config: BridgeConfig) -> Result<Self> {
        Ok(Self {
            config,
            transport: None,
            plugin: None,
            shared_buffer: None,
            editor_open: false,
            negotiated_format: crate::protocol::SampleFormat::Float32,
            sample_rate: 44100.0, // Default, updated when plugin is loaded
            // Initialize empty buffers (will resize on first process call)
            input_buffers_f32: Vec::new(),
            output_buffers_f32: Vec::new(),
            input_buffers_f64: Vec::new(),
            output_buffers_f64: Vec::new(),
            current_num_channels: 0,
            current_buffer_size: 0,
            midi_output_buffer: smallvec::SmallVec::new(),
        })
    }

    /// Ensure audio buffers are sized correctly
    /// Only reallocates if channel count or buffer size changed
    fn ensure_buffers_sized(&mut self, num_channels: usize, buffer_size: usize) {
        if self.current_num_channels != num_channels || self.current_buffer_size != buffer_size {
            // Resize f32 buffers
            self.input_buffers_f32.clear();
            self.output_buffers_f32.clear();
            for _ in 0..num_channels {
                self.input_buffers_f32.push(vec![0.0f32; buffer_size]);
                self.output_buffers_f32.push(vec![0.0f32; buffer_size]);
            }

            // Resize f64 buffers
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

    /// Run the bridge server
    pub async fn run(&mut self) -> Result<()> {
        // Listen for connection from host
        let listener = TransportListener::bind(&self.config.socket_path).await?;

        // Accept connection
        let mut transport = listener.accept().await?;

        // Send Ready message
        transport.send_bridge_message(&BridgeMessage::Ready).await?;

        self.transport = Some(transport);

        // Message loop
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

            // Poll for parameter changes from plugin automation
            self.poll_parameter_changes().await?;

            // Check for shutdown
            if self.transport.is_none() {
                break;
            }
        }

        Ok(())
    }

    /// Handle a single host message
    async fn handle_message(&mut self, msg: HostMessage) -> Result<Option<BridgeMessage>> {
        match msg {
            HostMessage::LoadPlugin {
                path,
                sample_rate,
                block_size,
                preferred_format,
            } => {
                let metadata = self.load_plugin(path, sample_rate, block_size, preferred_format)?;

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

            HostMessage::ProcessAudioMidi {
                buffer_id,
                num_samples,
                midi_events,
            } => {
                let midi: crate::protocol::MidiEventVec = midi_events
                    .iter()
                    .filter_map(|e| e.to_midi_event())
                    .collect();
                return self
                    .handle_process_audio(buffer_id, num_samples, &midi)
                    .await;
            }

            HostMessage::ProcessAudioFull {
                buffer_id,
                num_samples,
                midi_events,
                param_changes,
                note_expression,
                transport,
            } => {
                let midi: crate::protocol::MidiEventVec = midi_events
                    .iter()
                    .filter_map(|e| e.to_midi_event())
                    .collect();
                return self
                    .handle_process_audio_full(
                        buffer_id,
                        num_samples,
                        &midi,
                        &param_changes,
                        &note_expression,
                        &transport,
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
                // Call editor idle for VST2 (VST3 uses own event loop)
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

            HostMessage::Reset => {
                // Reset plugin state (silence audio buffers, reset internal state)
                // VST2/VST3 don't have explicit reset, so we do soft reset via suspend/resume pattern
                // This is handled by the plugin itself on next process() call

                Ok(None)
            }

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

    /// Handle audio processing with MIDI
    async fn handle_process_audio(
        &mut self,
        _buffer_id: u32,
        num_samples: usize,
        midi_events: &[crate::protocol::MidiEvent],
    ) -> Result<Option<BridgeMessage>> {
        if self.plugin.is_none() {
            return Ok(Some(BridgeMessage::Error {
                message: "No plugin loaded".to_string(),
            }));
        }

        let start = std::time::Instant::now();

        // Reuse pre-allocated MIDI output buffer (RT-safe)
        self.midi_output_buffer.clear();

        // Ensure buffers are sized before processing (avoids borrow checker issues)
        if let (Some(_), Some(ref plugin)) = (&self.shared_buffer, &self.plugin) {
            let metadata = plugin.as_instance().metadata();
            let num_channels = metadata.audio_io.inputs.max(metadata.audio_io.outputs);
            self.ensure_buffers_sized(num_channels, num_samples);
        }

        // Process audio through plugin
        if let (Some(ref mut shared_buffer), Some(ref mut plugin)) =
            (&mut self.shared_buffer, &mut self.plugin)
        {
            let metadata = plugin.as_instance().metadata();
            let num_channels = metadata.audio_io.inputs.max(metadata.audio_io.outputs);

            // Create process context with MIDI events
            let ctx = ProcessContext::new().midi(midi_events);

            match self.negotiated_format {
                crate::protocol::SampleFormat::Float64 => {
                    // f64 path - RT-safe: reuse pre-allocated buffers (already sized above)

                    // Copy input data into pre-allocated buffers
                    for ch in 0..num_channels {
                        if let Ok(data) = shared_buffer.read_channel_f64(ch) {
                            self.input_buffers_f64[ch][..num_samples]
                                .copy_from_slice(&data[..num_samples]);
                            self.output_buffers_f64[ch][..num_samples].fill(0.0);
                        }
                    }

                    // Create slice views (no allocation)
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

                    let mut audio_buffer = crate::protocol::AudioBuffer64 {
                        inputs: &input_slices,
                        outputs: &mut output_slices,
                        num_samples,
                        sample_rate,
                    };

                    let output = plugin
                        .as_instance_mut()
                        .process_f64(&mut audio_buffer, &ctx);
                    self.midi_output_buffer = output.midi_events;

                    // Write output from pre-allocated buffers
                    for ch in 0..num_channels {
                        if let Err(_e) = shared_buffer
                            .write_channel_f64(ch, &self.output_buffers_f64[ch][..num_samples])
                        {
                        }
                    }
                }
                crate::protocol::SampleFormat::Float32 => {
                    // f32 path - RT-safe: reuse pre-allocated buffers (already sized above)

                    // Copy input data into pre-allocated buffers
                    for ch in 0..num_channels {
                        if let Ok(data) = shared_buffer.read_channel(ch) {
                            self.input_buffers_f32[ch][..num_samples]
                                .copy_from_slice(&data[..num_samples]);
                            self.output_buffers_f32[ch][..num_samples].fill(0.0);
                        }
                    }

                    // Create slice views (no allocation)
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

                    let mut audio_buffer = crate::protocol::AudioBuffer {
                        inputs: &input_slices,
                        outputs: &mut output_slices,
                        num_samples,
                        sample_rate,
                    };

                    let output = plugin
                        .as_instance_mut()
                        .process_f32(&mut audio_buffer, &ctx);
                    self.midi_output_buffer = output.midi_events;

                    // Write output from pre-allocated buffers
                    for ch in 0..num_channels {
                        if let Err(_e) = shared_buffer
                            .write_channel(ch, &self.output_buffers_f32[ch][..num_samples])
                        {
                        }
                    }
                }
            }
        }

        let latency_us = start.elapsed().as_micros() as u64;

        // Return appropriate response based on whether we have MIDI output
        if !self.midi_output_buffer.is_empty() {
            Ok(Some(BridgeMessage::AudioProcessedMidi {
                latency_us,
                midi_output: self
                    .midi_output_buffer
                    .iter()
                    .map(IpcMidiEvent::from)
                    .collect(),
            }))
        } else {
            Ok(Some(BridgeMessage::AudioProcessed { latency_us }))
        }
    }

    /// Handle audio processing with full automation (MIDI + parameters + note expression + transport)
    async fn handle_process_audio_full(
        &mut self,
        _buffer_id: u32,
        num_samples: usize,
        midi_events: &[crate::protocol::MidiEvent],
        param_changes: &crate::protocol::ParameterChanges,
        note_expression: &crate::protocol::NoteExpressionChanges,
        transport: &crate::protocol::TransportInfo,
    ) -> Result<Option<BridgeMessage>> {
        if self.plugin.is_none() {
            return Ok(Some(BridgeMessage::Error {
                message: "No plugin loaded".to_string(),
            }));
        }

        let start = std::time::Instant::now();

        // Reuse pre-allocated MIDI output buffer (RT-safe)
        self.midi_output_buffer.clear();
        let mut param_output = crate::protocol::ParameterChanges::new();
        let mut note_expression_output = crate::protocol::NoteExpressionChanges::new();

        // Process audio through plugin
        if let (Some(ref mut shared_buffer), Some(ref mut plugin)) =
            (&mut self.shared_buffer, &mut self.plugin)
        {
            let metadata = plugin.as_instance().metadata();
            let num_channels = metadata.audio_io.inputs.max(metadata.audio_io.outputs);

            // Create process context with all data
            let ctx = ProcessContext::new()
                .midi(midi_events)
                .params(param_changes)
                .note_expression(note_expression)
                .transport(transport);

            match self.negotiated_format {
                crate::protocol::SampleFormat::Float64 => {
                    // f64 path - RT-safe: reuse pre-allocated buffers (already sized above)

                    // Copy input data into pre-allocated buffers
                    for ch in 0..num_channels {
                        if let Ok(data) = shared_buffer.read_channel_f64(ch) {
                            self.input_buffers_f64[ch][..num_samples]
                                .copy_from_slice(&data[..num_samples]);
                            self.output_buffers_f64[ch][..num_samples].fill(0.0);
                        }
                    }

                    // Create slice views (no allocation)
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

                    let mut audio_buffer = crate::protocol::AudioBuffer64 {
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

                    // Write output from pre-allocated buffers
                    for ch in 0..num_channels {
                        if let Err(_e) = shared_buffer
                            .write_channel_f64(ch, &self.output_buffers_f64[ch][..num_samples])
                        {
                        }
                    }
                }
                crate::protocol::SampleFormat::Float32 => {
                    // f32 path - RT-safe: reuse pre-allocated buffers (already sized above)

                    // Copy input data into pre-allocated buffers
                    for ch in 0..num_channels {
                        if let Ok(data) = shared_buffer.read_channel(ch) {
                            self.input_buffers_f32[ch][..num_samples]
                                .copy_from_slice(&data[..num_samples]);
                            self.output_buffers_f32[ch][..num_samples].fill(0.0);
                        }
                    }

                    // Create slice views (no allocation)
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

                    let mut audio_buffer = crate::protocol::AudioBuffer {
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

                    // Write output from pre-allocated buffers
                    for ch in 0..num_channels {
                        if let Err(_e) = shared_buffer
                            .write_channel(ch, &self.output_buffers_f32[ch][..num_samples])
                        {
                        }
                    }
                }
            }
        }

        let latency_us = start.elapsed().as_micros() as u64;

        // Return full automation response
        Ok(Some(BridgeMessage::AudioProcessedFull {
            latency_us,
            midi_output: self
                .midi_output_buffer
                .iter()
                .map(IpcMidiEvent::from)
                .collect(),
            param_output,
            note_expression_output,
        }))
    }

    /// Poll for parameter changes from plugin automation
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

        // VST3 doesn't need polling - it uses a different callback mechanism
        Ok(())
    }

    /// Load plugin (VST2, VST3, or stub)
    fn load_plugin(
        &mut self,
        path: PathBuf,
        sample_rate: f64,
        block_size: usize,
        preferred_format: crate::protocol::SampleFormat,
    ) -> Result<PluginMetadata> {
        // Store sample rate for use during processing
        self.sample_rate = sample_rate;
        // Validate plugin exists
        if !path.exists() {
            return Err(BridgeError::LoadFailed {
                path: path.to_path_buf(),
                stage: LoadStage::Scanning,
                reason: "Plugin not found".to_string(),
            });
        }

        // Determine plugin format from extension
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let (plugin, metadata): (LoadedPlugin, PluginMetadata) = match extension
            .to_lowercase()
            .as_str()
        {
            #[cfg(feature = "vst3")]
            "vst3" => {
                // Load as VST3
                let mut vst = Vst3Instance::load(&path, sample_rate, block_size)?;
                // Negotiate format: use f64 if preferred AND plugin supports it
                if preferred_format == crate::protocol::SampleFormat::Float64
                    && vst.can_process_f64()
                {
                    let _ = vst.set_sample_format(crate::protocol::SampleFormat::Float64);
                }
                let metadata = vst.metadata().clone();
                (LoadedPlugin::Vst3(vst), metadata)
            }

            #[cfg(feature = "vst2")]
            "vst" | "dll" | "so" => {
                // Load as VST2
                let vst = Vst2Instance::load(&path, sample_rate, block_size)?;
                let metadata = vst.metadata().clone();
                (LoadedPlugin::Vst2(vst), metadata)
            }

            #[cfg(feature = "clap")]
            "clap" => {
                // Load as CLAP
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

        // Negotiate format: use f64 only if preferred AND plugin supports it
        let negotiated_format = if preferred_format == crate::protocol::SampleFormat::Float64
            && metadata.supports_f64
        {
            crate::protocol::SampleFormat::Float64
        } else {
            crate::protocol::SampleFormat::Float32
        };
        self.negotiated_format = negotiated_format;

        // Open shared memory buffer with negotiated format
        let buffer_name = format!("dawai_vst_buffer_{}", std::process::id());
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
        // Clean up socket
        let _ = std::fs::remove_file(&self.config.socket_path);
    }
}
