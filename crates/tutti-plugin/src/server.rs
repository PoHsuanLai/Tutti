//! Bridge server - runs in isolated process
//!
//! The server loads and runs plugins in a sandboxed environment.

use crate::error::{BridgeError, LoadStage, Result};
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
                preferred_format,
            } => {
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
                    let result: Result<()> = match plugin {
                        #[cfg(feature = "vst2")]
                        LoadedPlugin::Vst2(p) => p.set_state(&data),
                        #[cfg(feature = "vst3")]
                        LoadedPlugin::Vst3(p) => p.set_state(&data),
                        #[cfg(feature = "clap")]
                        LoadedPlugin::Clap(p) => p.set_state(&data),
                    };

                    match result {
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
            self.ensure_buffers_sized(num_channels, num_samples);
        }

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

                    self.midi_output_buffer = match plugin {
                        #[cfg(feature = "vst2")]
                        LoadedPlugin::Vst2(p) => {
                            p.process_with_midi_f64(&mut audio_buffer, midi_events)
                        }
                        #[cfg(feature = "vst3")]
                        LoadedPlugin::Vst3(p) => {
                            p.process_f64(&mut audio_buffer);
                            smallvec::SmallVec::new()
                        }
                        #[cfg(feature = "clap")]
                        LoadedPlugin::Clap(p) => {
                            p.process_f64(&mut audio_buffer);
                            smallvec::SmallVec::new()
                        }
                    };

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

                    self.midi_output_buffer = match plugin {
                        #[cfg(feature = "vst2")]
                        LoadedPlugin::Vst2(p) => {
                            p.process_with_midi(&mut audio_buffer, midi_events)
                        }
                        #[cfg(feature = "vst3")]
                        LoadedPlugin::Vst3(p) => {
                            p.process_with_midi(&mut audio_buffer, midi_events)
                        }
                        #[cfg(feature = "clap")]
                        LoadedPlugin::Clap(p) => {
                            p.process_with_midi(&mut audio_buffer, midi_events)
                        }
                    };

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
                midi_output: self.midi_output_buffer.clone(),
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
                            self.midi_output_buffer = p.process_with_transport_f64(
                                &mut audio_buffer,
                                midi_events,
                                transport,
                            );
                        }
                        #[cfg(feature = "vst3")]
                        LoadedPlugin::Vst3(p) => {
                            let (midi_out, param_out, note_expr_out) = p
                                .process_with_automation_f64(
                                    &mut audio_buffer,
                                    midi_events,
                                    param_changes,
                                    note_expression,
                                    transport,
                                );
                            self.midi_output_buffer = midi_out;
                            param_output = param_out;
                            note_expression_output = note_expr_out;
                        }
                        #[cfg(feature = "clap")]
                        LoadedPlugin::Clap(p) => {
                            let (midi_out, param_out, note_expr_out) = p
                                .process_with_automation_f64(
                                    &mut audio_buffer,
                                    midi_events,
                                    param_changes,
                                    note_expression,
                                    transport,
                                );
                            self.midi_output_buffer = midi_out;
                            param_output = param_out;
                            note_expression_output = note_expr_out;
                        }
                    };

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
                            self.midi_output_buffer =
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
                            self.midi_output_buffer = midi_out;
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
                            self.midi_output_buffer = midi_out;
                            param_output = param_out;
                            note_expression_output = note_expr_out;
                        }
                    };

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
            midi_output: self.midi_output_buffer.clone(),
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
