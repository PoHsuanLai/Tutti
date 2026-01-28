//! Bridge server - runs in isolated process
//!
//! The server loads and runs plugins in a sandboxed environment.

use crate::error::{BridgeError, Result};
use crate::protocol::{BridgeConfig, BridgeMessage, HostMessage, PluginMetadata};
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
        })
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
            let msg = self.transport.as_mut().unwrap().recv_host_message().await?;

            let response = self.handle_message(msg).await?;

            if let Some(response) = response {
                self.transport
                    .as_mut()
                    .unwrap()
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
            HostMessage::LoadPlugin { path, sample_rate, preferred_format } => {

                let metadata = self.load_plugin(path, sample_rate, preferred_format)?;

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
                return self
                    .handle_process_audio(buffer_id, num_samples, &midi_events)
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
                return self
                    .handle_process_audio_full(
                        buffer_id,
                        num_samples,
                        &midi_events,
                        &param_changes,
                        &note_expression,
                        &transport,
                    )
                    .await;
            }

            HostMessage::SetParameter { id, value } => {

                if let Some(ref mut plugin) = self.plugin {
                    // Parse parameter ID as index
                    if let Ok(index) = id.parse::<u32>() {
                        match plugin {
                            #[cfg(feature = "vst2")]
                            LoadedPlugin::Vst2(p) => p.set_parameter(index as i32, value),
                            #[cfg(feature = "vst3")]
                            LoadedPlugin::Vst3(p) => p.set_parameter(index, value as f64),
                            #[cfg(feature = "clap")]
                            LoadedPlugin::Clap(p) => p.set_parameter(index, value as f64),
                        }
                    }
                }

                Ok(None)
            }

            HostMessage::GetParameter { id } => {

                if let Some(ref mut plugin) = self.plugin {
                    // Parse parameter ID as index
                    if let Ok(index) = id.parse::<u32>() {
                        let value = match plugin {
                            #[cfg(feature = "vst2")]
                            LoadedPlugin::Vst2(p) => Some(p.get_parameter(index as i32)),
                            #[cfg(feature = "vst3")]
                            LoadedPlugin::Vst3(p) => Some(p.get_parameter(index) as f32),
                            #[cfg(feature = "clap")]
                            LoadedPlugin::Clap(p) => Some(p.get_parameter(index) as f32),
                        };
                        return Ok(Some(BridgeMessage::ParameterValue { value }));
                    }
                }

                Ok(Some(BridgeMessage::ParameterValue { value: None }))
            }

            HostMessage::OpenEditor { parent_handle } => {

                if let Some(ref mut plugin) = self.plugin {
                    // Convert parent_handle u64 to raw pointer
                    let parent_ptr = parent_handle as *mut std::ffi::c_void;

                    let result: Result<(u32, u32)> = match plugin {
                        #[cfg(feature = "vst2")]
                        LoadedPlugin::Vst2(p) => p.open_editor(parent_ptr),
                        #[cfg(feature = "vst3")]
                        LoadedPlugin::Vst3(p) => unsafe { p.open_editor(parent_ptr) },
                        #[cfg(feature = "clap")]
                        LoadedPlugin::Clap(p) => p.open_editor(parent_ptr),
                    };

                    match result {
                        Ok((width, height)) => {
                            self.editor_open = true;
                            Ok(Some(BridgeMessage::EditorOpened { width, height }))
                        }
                        Err(e) => {
                            Ok(Some(BridgeMessage::Error {
                                message: format!("Failed to open editor: {}", e),
                            }))
                        }
                    }
                } else {
                    Ok(Some(BridgeMessage::Error {
                        message: "No plugin loaded".to_string(),
                    }))
                }
            }

            HostMessage::CloseEditor => {

                if let Some(ref mut plugin) = self.plugin {
                    match plugin {
                        #[cfg(feature = "vst2")]
                        LoadedPlugin::Vst2(p) => p.close_editor(),
                        #[cfg(feature = "vst3")]
                        LoadedPlugin::Vst3(p) => p.close_editor(),
                        #[cfg(feature = "clap")]
                        LoadedPlugin::Clap(p) => p.close_editor(),
                    }
                    self.editor_open = false;
                }

                Ok(None)
            }

            HostMessage::EditorIdle => {
                // Call editor idle for VST2 (VST3 uses own event loop)
                if self.editor_open {
                    if let Some(ref mut plugin) = self.plugin {
                        match plugin {
                            #[cfg(feature = "vst2")]
                            LoadedPlugin::Vst2(p) => p.editor_idle(),
                            #[cfg(feature = "vst3")]
                            LoadedPlugin::Vst3(p) => p.editor_idle(),
                            #[cfg(feature = "clap")]
                            LoadedPlugin::Clap(p) => p.editor_idle(),
                        }
                    }
                }
                Ok(None)
            }

            HostMessage::SetSampleRate { rate } => {

                if let Some(ref mut plugin) = self.plugin {
                    match plugin {
                        #[cfg(feature = "vst2")]
                        LoadedPlugin::Vst2(p) => p.set_sample_rate(rate),
                        #[cfg(feature = "vst3")]
                        LoadedPlugin::Vst3(p) => p.set_sample_rate(rate),
                        #[cfg(feature = "clap")]
                        LoadedPlugin::Clap(p) => p.set_sample_rate(rate),
                    }
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
                    let state_result: Result<Vec<u8>> = match plugin {
                        #[cfg(feature = "vst2")]
                        LoadedPlugin::Vst2(p) => p.get_state(),
                        #[cfg(feature = "vst3")]
                        LoadedPlugin::Vst3(p) => p.get_state(),
                        #[cfg(feature = "clap")]
                        LoadedPlugin::Clap(p) => p.get_state(),
                    };

                    match state_result {
                        Ok(data) => {
                            Ok(Some(BridgeMessage::StateData { data }))
                        }
                        Err(e) => {
                            Ok(Some(BridgeMessage::Error {
                                message: format!("Failed to save state: {}", e),
                            }))
                        }
                    }
                } else {
                    Ok(Some(BridgeMessage::StateData { data: vec![] }))
                }
            }

            HostMessage::LoadState { data } => {

                if let Some(ref mut plugin) = self.plugin {
                    let result: Result<()> = match plugin {
                        #[cfg(feature = "vst2")]
                        LoadedPlugin::Vst2(p) => p.set_state(&data),
                        #[cfg(feature = "vst3")]
                        LoadedPlugin::Vst3(p) => p.set_state(&data),
                        #[cfg(feature = "clap")]
                        LoadedPlugin::Clap(p) => p.set_state(&data),
                    };

                    match result {
                        Ok(()) => {
                            Ok(None)
                        }
                        Err(e) => {
                            Ok(Some(BridgeMessage::Error {
                                message: format!("Failed to load state: {}", e),
                            }))
                        }
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

        // Collect MIDI output (will be populated by plugins that support it)
        let mut midi_output = Vec::new();

        // Process audio through plugin
        if let (Some(ref mut shared_buffer), Some(ref mut plugin)) =
            (&mut self.shared_buffer, &mut self.plugin)
        {
            let num_channels = match plugin {
                #[cfg(feature = "vst2")]
                LoadedPlugin::Vst2(p) => p
                    .metadata()
                    .audio_io
                    .inputs
                    .max(p.metadata().audio_io.outputs),
                #[cfg(feature = "vst3")]
                LoadedPlugin::Vst3(p) => p
                    .metadata()
                    .audio_io
                    .inputs
                    .max(p.metadata().audio_io.outputs),
                #[cfg(feature = "clap")]
                LoadedPlugin::Clap(p) => p
                    .metadata()
                    .audio_io
                    .inputs
                    .max(p.metadata().audio_io.outputs),
            };

            match self.negotiated_format {
                crate::protocol::SampleFormat::Float64 => {
                    // f64 path: read/write f64 from shared memory
                    let mut input_vecs: Vec<Vec<f64>> = Vec::new();
                    let mut output_vecs: Vec<Vec<f64>> = Vec::new();

                    for ch in 0..num_channels {
                        if let Ok(data) = shared_buffer.read_channel_f64(ch) {
                            input_vecs.push(data[..num_samples].to_vec());
                            output_vecs.push(vec![0.0f64; num_samples]);
                        }
                    }

                    let input_slices: Vec<&[f64]> = input_vecs.iter().map(|v| v.as_slice()).collect();
                    let mut output_slices: Vec<&mut [f64]> =
                        output_vecs.iter_mut().map(|v| v.as_mut_slice()).collect();

                    let sample_rate = match plugin {
                        #[cfg(feature = "vst2")]
                        LoadedPlugin::Vst2(p) => p.metadata().audio_io.inputs as f32,
                        #[cfg(feature = "vst3")]
                        LoadedPlugin::Vst3(p) => p.metadata().audio_io.inputs as f32,
                        #[cfg(feature = "clap")]
                        LoadedPlugin::Clap(p) => p.metadata().audio_io.inputs as f32,
                    };

                    let mut audio_buffer = crate::protocol::AudioBuffer64 {
                        inputs: &input_slices,
                        outputs: &mut output_slices,
                        num_samples,
                        sample_rate,
                    };

                    midi_output = match plugin {
                        #[cfg(feature = "vst2")]
                        LoadedPlugin::Vst2(p) => p.process_with_midi_f64(&mut audio_buffer, midi_events),
                        #[cfg(feature = "vst3")]
                        LoadedPlugin::Vst3(p) => {
                            p.process_f64(&mut audio_buffer);
                            Vec::new()
                        }
                        #[cfg(feature = "clap")]
                        LoadedPlugin::Clap(p) => {
                            p.process_f64(&mut audio_buffer);
                            Vec::new()
                        }
                    };

                    for (ch, output) in output_vecs.iter().enumerate() {
                        if let Err(_e) = shared_buffer.write_channel_f64(ch, output) {
                        }
                    }
                }
                crate::protocol::SampleFormat::Float32 => {
                    // f32 path: existing code
                    let mut input_vecs: Vec<Vec<f32>> = Vec::new();
                    let mut output_vecs: Vec<Vec<f32>> = Vec::new();

                    for ch in 0..num_channels {
                        if let Ok(data) = shared_buffer.read_channel(ch) {
                            input_vecs.push(data[..num_samples].to_vec());
                            output_vecs.push(vec![0.0; num_samples]);
                        }
                    }

                    let input_slices: Vec<&[f32]> = input_vecs.iter().map(|v| v.as_slice()).collect();
                    let mut output_slices: Vec<&mut [f32]> =
                        output_vecs.iter_mut().map(|v| v.as_mut_slice()).collect();

                    let sample_rate = match plugin {
                        #[cfg(feature = "vst2")]
                        LoadedPlugin::Vst2(p) => p.metadata().audio_io.inputs as f32,
                        #[cfg(feature = "vst3")]
                        LoadedPlugin::Vst3(p) => p.metadata().audio_io.inputs as f32,
                        #[cfg(feature = "clap")]
                        LoadedPlugin::Clap(p) => p.metadata().audio_io.inputs as f32,
                    };

                    let mut audio_buffer = crate::protocol::AudioBuffer {
                        inputs: &input_slices,
                        outputs: &mut output_slices,
                        num_samples,
                        sample_rate,
                    };

                    midi_output = match plugin {
                        #[cfg(feature = "vst2")]
                        LoadedPlugin::Vst2(p) => p.process_with_midi(&mut audio_buffer, midi_events),
                        #[cfg(feature = "vst3")]
                        LoadedPlugin::Vst3(p) => p.process_with_midi(&mut audio_buffer, midi_events),
                        #[cfg(feature = "clap")]
                        LoadedPlugin::Clap(p) => p.process_with_midi(&mut audio_buffer, midi_events),
                    };

                    for (ch, output) in output_vecs.iter().enumerate() {
                        if let Err(_e) = shared_buffer.write_channel(ch, output) {
                        }
                    }
                }
            }
        }

        let latency_us = start.elapsed().as_micros() as u64;

        // Return appropriate response based on whether we have MIDI output
        if !midi_output.is_empty() {
            Ok(Some(BridgeMessage::AudioProcessedMidi {
                latency_us,
                midi_output,
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

        let mut midi_output = Vec::new();
        let mut param_output = crate::protocol::ParameterChanges::new();
        let mut note_expression_output = crate::protocol::NoteExpressionChanges::new();

        // Process audio through plugin
        if let (Some(ref mut shared_buffer), Some(ref mut plugin)) =
            (&mut self.shared_buffer, &mut self.plugin)
        {
            let num_channels = match plugin {
                #[cfg(feature = "vst2")]
                LoadedPlugin::Vst2(p) => p
                    .metadata()
                    .audio_io
                    .inputs
                    .max(p.metadata().audio_io.outputs),
                #[cfg(feature = "vst3")]
                LoadedPlugin::Vst3(p) => p
                    .metadata()
                    .audio_io
                    .inputs
                    .max(p.metadata().audio_io.outputs),
                #[cfg(feature = "clap")]
                LoadedPlugin::Clap(p) => p
                    .metadata()
                    .audio_io
                    .inputs
                    .max(p.metadata().audio_io.outputs),
            };

            match self.negotiated_format {
                crate::protocol::SampleFormat::Float64 => {
                    // f64 path
                    let mut input_vecs: Vec<Vec<f64>> = Vec::new();
                    let mut output_vecs: Vec<Vec<f64>> = Vec::new();

                    for ch in 0..num_channels {
                        if let Ok(data) = shared_buffer.read_channel_f64(ch) {
                            input_vecs.push(data[..num_samples].to_vec());
                            output_vecs.push(vec![0.0f64; num_samples]);
                        }
                    }

                    let input_slices: Vec<&[f64]> = input_vecs.iter().map(|v| v.as_slice()).collect();
                    let mut output_slices: Vec<&mut [f64]> =
                        output_vecs.iter_mut().map(|v| v.as_mut_slice()).collect();

                    let sample_rate = match plugin {
                        #[cfg(feature = "vst2")]
                        LoadedPlugin::Vst2(p) => p.metadata().audio_io.inputs as f32,
                        #[cfg(feature = "vst3")]
                        LoadedPlugin::Vst3(p) => p.metadata().audio_io.inputs as f32,
                        #[cfg(feature = "clap")]
                        LoadedPlugin::Clap(p) => p.metadata().audio_io.inputs as f32,
                    };

                    let mut audio_buffer = crate::protocol::AudioBuffer64 {
                        inputs: &input_slices,
                        outputs: &mut output_slices,
                        num_samples,
                        sample_rate,
                    };

                    match plugin {
                        #[cfg(feature = "vst2")]
                        LoadedPlugin::Vst2(p) => {
                            midi_output =
                                p.process_with_transport_f64(&mut audio_buffer, midi_events, transport);
                        }
                        #[cfg(feature = "vst3")]
                        LoadedPlugin::Vst3(p) => {
                            let (midi_out, param_out, note_expr_out) = p.process_with_automation_f64(
                                &mut audio_buffer,
                                midi_events,
                                param_changes,
                                note_expression,
                                transport,
                            );
                            midi_output = midi_out;
                            param_output = param_out;
                            note_expression_output = note_expr_out;
                        }
                        #[cfg(feature = "clap")]
                        LoadedPlugin::Clap(p) => {
                            let (midi_out, param_out, note_expr_out) = p.process_with_automation_f64(
                                &mut audio_buffer,
                                midi_events,
                                param_changes,
                                note_expression,
                                transport,
                            );
                            midi_output = midi_out;
                            param_output = param_out;
                            note_expression_output = note_expr_out;
                        }
                    };

                    for (ch, output) in output_vecs.iter().enumerate() {
                        if let Err(_e) = shared_buffer.write_channel_f64(ch, output) {
                        }
                    }
                }
                crate::protocol::SampleFormat::Float32 => {
                    // f32 path: existing code
                    let mut input_vecs: Vec<Vec<f32>> = Vec::new();
                    let mut output_vecs: Vec<Vec<f32>> = Vec::new();

                    for ch in 0..num_channels {
                        if let Ok(data) = shared_buffer.read_channel(ch) {
                            input_vecs.push(data[..num_samples].to_vec());
                            output_vecs.push(vec![0.0; num_samples]);
                        }
                    }

                    let input_slices: Vec<&[f32]> = input_vecs.iter().map(|v| v.as_slice()).collect();
                    let mut output_slices: Vec<&mut [f32]> =
                        output_vecs.iter_mut().map(|v| v.as_mut_slice()).collect();

                    let sample_rate = match plugin {
                        #[cfg(feature = "vst2")]
                        LoadedPlugin::Vst2(p) => p.metadata().audio_io.inputs as f32,
                        #[cfg(feature = "vst3")]
                        LoadedPlugin::Vst3(p) => p.metadata().audio_io.inputs as f32,
                        #[cfg(feature = "clap")]
                        LoadedPlugin::Clap(p) => p.metadata().audio_io.inputs as f32,
                    };

                    let mut audio_buffer = crate::protocol::AudioBuffer {
                        inputs: &input_slices,
                        outputs: &mut output_slices,
                        num_samples,
                        sample_rate,
                    };

                    match plugin {
                        #[cfg(feature = "vst2")]
                        LoadedPlugin::Vst2(p) => {
                            midi_output =
                                p.process_with_transport(&mut audio_buffer, midi_events, transport);
                        }
                        #[cfg(feature = "vst3")]
                        LoadedPlugin::Vst3(p) => {
                            let (midi_out, param_out, note_expr_out) = p.process_with_automation(
                                &mut audio_buffer,
                                midi_events,
                                param_changes,
                                note_expression,
                                transport,
                            );
                            midi_output = midi_out;
                            param_output = param_out;
                            note_expression_output = note_expr_out;
                        }
                        #[cfg(feature = "clap")]
                        LoadedPlugin::Clap(p) => {
                            let (midi_out, param_out, note_expr_out) = p.process_with_automation(
                                &mut audio_buffer,
                                midi_events,
                                param_changes,
                                note_expression,
                                transport,
                            );
                            midi_output = midi_out;
                            param_output = param_out;
                            note_expression_output = note_expr_out;
                        }
                    };

                    for (ch, output) in output_vecs.iter().enumerate() {
                        if let Err(_e) = shared_buffer.write_channel(ch, output) {
                        }
                    }
                }
            }
        }

        let latency_us = start.elapsed().as_micros() as u64;

        // Return full automation response
        Ok(Some(BridgeMessage::AudioProcessedFull {
            latency_us,
            midi_output,
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
        sample_rate: f32,
        preferred_format: crate::protocol::SampleFormat,
    ) -> Result<PluginMetadata> {
        // Validate plugin exists
        if !path.exists() {
            return Err(BridgeError::LoadFailed(format!(
                "Plugin not found: {:?}",
                path
            )));
        }

        // Determine plugin format from extension
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let (plugin, metadata): (LoadedPlugin, PluginMetadata) =
            match extension.to_lowercase().as_str() {
                #[cfg(feature = "vst3")]
                "vst3" => {
                    // Load as VST3
                    let mut vst = Vst3Instance::load(&path, sample_rate)?;
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
                    let vst = Vst2Instance::load(&path, sample_rate)?;
                    let metadata = vst.metadata().clone();
                    (LoadedPlugin::Vst2(vst), metadata)
                }

                #[cfg(feature = "clap")]
                "clap" => {
                    // Load as CLAP
                    let clap = ClapInstance::load(&path, sample_rate)?;
                    let metadata = clap.metadata().clone();
                    (LoadedPlugin::Clap(clap), metadata)
                }

                _ => {
                    return Err(BridgeError::LoadFailed(format!(
                    "Unsupported plugin format: {}. Supported: .vst3, .vst/.dll/.so (VST2), .clap",
                    extension
                )));
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
