//! VST3 plugin loader
//!
//! This module provides VST3 plugin hosting using the `vst3-host` crate.
//! It wraps the generic VST3 types to work with Tutti's protocol types.

use std::path::Path;

use crate::error::{BridgeError, LoadStage, Result};
use crate::protocol::{
    AudioBuffer, AudioBuffer64, MidiEvent, MidiEventVec, NoteExpressionChanges,
    NoteExpressionType, ParameterChanges, ParameterPoint, ParameterQueue, PluginMetadata,
    TransportInfo,
};

// Import tutti_midi_io types for conversion
use tutti_midi_io::{Channel, ChannelVoiceMsg, ControlChange};

// Re-export the vst3-host crate for advanced users
pub use vst3_host;

/// VST3 plugin instance wrapper for Tutti.
///
/// This wraps `vst3_host::Vst3Instance` and provides conversion between
/// Tutti's protocol types and vst3-host's generic types.
pub struct Vst3Instance {
    inner: vst3_host::Vst3Instance,
    metadata: PluginMetadata,
}

impl Vst3Instance {
    /// Load a VST3 plugin from path.
    pub fn load(path: &Path, sample_rate: f64, block_size: usize) -> Result<Self> {
        let inner =
            vst3_host::Vst3Instance::load(path, sample_rate, block_size).map_err(|e| match e {
                vst3_host::Vst3Error::LoadFailed {
                    path,
                    stage,
                    reason,
                } => BridgeError::LoadFailed {
                    path,
                    stage: convert_load_stage(stage),
                    reason,
                },
                vst3_host::Vst3Error::PluginError { stage, code } => BridgeError::PluginError {
                    stage: convert_load_stage(stage),
                    code,
                },
                _ => BridgeError::LoadFailed {
                    path: path.to_path_buf(),
                    stage: LoadStage::Opening,
                    reason: e.to_string(),
                },
            })?;

        let info = inner.info();
        let metadata = PluginMetadata::new(info.id.clone(), info.name.clone())
            .author(info.vendor.clone())
            .version(info.version.clone())
            .audio_io(info.num_inputs, info.num_outputs)
            .midi(info.has_midi_input)
            .f64_support(info.supports_f64);

        Ok(Self { inner, metadata })
    }

    /// Get plugin metadata.
    pub fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    /// Check if this plugin supports 64-bit (f64) audio processing.
    pub fn supports_f64(&self) -> bool {
        self.inner.supports_f64()
    }

    /// Alias for supports_f64() for compatibility with server.rs.
    pub fn can_process_f64(&self) -> bool {
        self.inner.supports_f64()
    }

    /// Set the sample format (f32 or f64).
    pub fn set_sample_format(&mut self, format: crate::protocol::SampleFormat) -> Result<()> {
        let use_f64 = matches!(format, crate::protocol::SampleFormat::Float64);
        self.inner.set_use_f64(use_f64).map_err(|e| {
            BridgeError::LoadFailed {
                path: std::path::PathBuf::from("format"),
                stage: LoadStage::Initialization,
                reason: e.to_string(),
            }
        })
    }

    /// Process audio with MIDI events.
    pub fn process_with_midi<'a>(
        &mut self,
        buffer: &'a mut AudioBuffer<'a>,
        midi_events: &[MidiEvent],
    ) -> MidiEventVec {
        // Convert Tutti MIDI events to vst3-host events
        let vst3_midi: Vec<TuttiMidiWrapper> =
            midi_events.iter().map(TuttiMidiWrapper::from).collect();

        // Create vst3-host audio buffer from Tutti buffer
        let mut vst3_buffer = vst3_host::AudioBuffer::new(
            buffer.inputs,
            buffer.outputs,
            buffer.num_samples,
            buffer.sample_rate as f64,
        );

        let transport = vst3_host::TransportState::new();

        // Process through vst3-host (unified API)
        let (output_midi, _) = self.inner.process(&mut vst3_buffer, &vst3_midi, None, &[], &transport);

        // Convert output MIDI back to Tutti format
        output_midi
            .into_iter()
            .filter_map(|e| convert_vst3_midi_to_tutti(&e))
            .collect()
    }

    /// Process audio with MIDI events and transport info.
    pub fn process_with_transport<'a>(
        &mut self,
        buffer: &'a mut AudioBuffer<'a>,
        midi_events: &[MidiEvent],
        transport: &TransportInfo,
    ) -> MidiEventVec {
        let vst3_midi: Vec<TuttiMidiWrapper> =
            midi_events.iter().map(TuttiMidiWrapper::from).collect();

        let mut vst3_buffer = vst3_host::AudioBuffer::new(
            buffer.inputs,
            buffer.outputs,
            buffer.num_samples,
            buffer.sample_rate as f64,
        );

        let vst3_transport = convert_transport_to_vst3(transport);

        let (output_midi, _) = self.inner.process(&mut vst3_buffer, &vst3_midi, None, &[], &vst3_transport);

        output_midi
            .into_iter()
            .filter_map(|e| convert_vst3_midi_to_tutti(&e))
            .collect()
    }

    /// Process audio with full automation (MIDI + parameter changes + note expression).
    pub fn process_with_automation<'a>(
        &mut self,
        buffer: &'a mut AudioBuffer<'a>,
        midi_events: &[MidiEvent],
        param_changes: &ParameterChanges,
        note_expression: &NoteExpressionChanges,
        transport: &TransportInfo,
    ) -> (MidiEventVec, ParameterChanges, NoteExpressionChanges) {
        let vst3_midi: Vec<TuttiMidiWrapper> =
            midi_events.iter().map(TuttiMidiWrapper::from).collect();

        let mut vst3_buffer = vst3_host::AudioBuffer::new(
            buffer.inputs,
            buffer.outputs,
            buffer.num_samples,
            buffer.sample_rate as f64,
        );

        let vst3_transport = convert_transport_to_vst3(transport);
        let vst3_params = convert_params_to_vst3(param_changes);
        let vst3_note_expr = convert_note_expression_to_vst3(note_expression);

        let (output_midi, output_params) = self.inner.process(
            &mut vst3_buffer,
            &vst3_midi,
            Some(&vst3_params),
            &vst3_note_expr,
            &vst3_transport,
        );

        let tutti_midi = output_midi
            .into_iter()
            .filter_map(|e| convert_vst3_midi_to_tutti(&e))
            .collect();
        let tutti_params = convert_params_from_vst3(&output_params);

        (tutti_midi, tutti_params, NoteExpressionChanges::new())
    }

    /// Process audio with f64 buffers.
    pub fn process_with_midi_f64<'a>(
        &mut self,
        buffer: &'a mut AudioBuffer64<'a>,
        midi_events: &[MidiEvent],
    ) -> MidiEventVec {
        let vst3_midi: Vec<TuttiMidiWrapper> =
            midi_events.iter().map(TuttiMidiWrapper::from).collect();

        let mut vst3_buffer = vst3_host::AudioBuffer::new(
            buffer.inputs,
            buffer.outputs,
            buffer.num_samples,
            buffer.sample_rate as f64,
        );

        let transport = vst3_host::TransportState::new();
        let (output_midi, _) = self.inner.process(&mut vst3_buffer, &vst3_midi, None, &[], &transport);

        output_midi
            .into_iter()
            .filter_map(|e| convert_vst3_midi_to_tutti(&e))
            .collect()
    }

    /// Process audio with f64 buffers and transport info.
    pub fn process_with_transport_f64<'a>(
        &mut self,
        buffer: &'a mut AudioBuffer64<'a>,
        midi_events: &[MidiEvent],
        transport: &TransportInfo,
    ) -> MidiEventVec {
        let vst3_midi: Vec<TuttiMidiWrapper> =
            midi_events.iter().map(TuttiMidiWrapper::from).collect();

        let mut vst3_buffer = vst3_host::AudioBuffer::new(
            buffer.inputs,
            buffer.outputs,
            buffer.num_samples,
            buffer.sample_rate as f64,
        );

        let vst3_transport = convert_transport_to_vst3(transport);
        let (output_midi, _) = self.inner.process(&mut vst3_buffer, &vst3_midi, None, &[], &vst3_transport);

        output_midi
            .into_iter()
            .filter_map(|e| convert_vst3_midi_to_tutti(&e))
            .collect()
    }

    /// Process audio with f64 buffers and full automation.
    pub fn process_with_automation_f64<'a>(
        &mut self,
        buffer: &'a mut AudioBuffer64<'a>,
        midi_events: &[MidiEvent],
        param_changes: &ParameterChanges,
        note_expression: &NoteExpressionChanges,
        transport: &TransportInfo,
    ) -> (MidiEventVec, ParameterChanges, NoteExpressionChanges) {
        let vst3_midi: Vec<TuttiMidiWrapper> =
            midi_events.iter().map(TuttiMidiWrapper::from).collect();

        let mut vst3_buffer = vst3_host::AudioBuffer::new(
            buffer.inputs,
            buffer.outputs,
            buffer.num_samples,
            buffer.sample_rate as f64,
        );

        let vst3_transport = convert_transport_to_vst3(transport);
        let vst3_params = convert_params_to_vst3(param_changes);
        let vst3_note_expr = convert_note_expression_to_vst3(note_expression);

        let (output_midi, output_params) = self.inner.process(
            &mut vst3_buffer,
            &vst3_midi,
            Some(&vst3_params),
            &vst3_note_expr,
            &vst3_transport,
        );

        let tutti_midi = output_midi
            .into_iter()
            .filter_map(|e| convert_vst3_midi_to_tutti(&e))
            .collect();
        let tutti_params = convert_params_from_vst3(&output_params);

        (tutti_midi, tutti_params, NoteExpressionChanges::new())
    }

    /// Simple f64 processing (no MIDI, no automation).
    pub fn process_f64<'a>(&mut self, buffer: &'a mut AudioBuffer64<'a>) {
        let mut vst3_buffer = vst3_host::AudioBuffer::new(
            buffer.inputs,
            buffer.outputs,
            buffer.num_samples,
            buffer.sample_rate as f64,
        );

        let transport = vst3_host::TransportState::new();
        let empty_midi: Vec<TuttiMidiWrapper> = vec![];
        let _ = self.inner.process::<f64, _>(&mut vst3_buffer, &empty_midi, None, &[], &transport);
    }

    /// Set sample rate.
    pub fn set_sample_rate(&mut self, rate: f64) {
        self.inner.set_sample_rate(rate);
    }

    /// Get parameter count.
    pub fn get_parameter_count(&self) -> i32 {
        self.inner.get_parameter_count()
    }

    /// Get parameter value by native ParamID (normalized 0-1).
    ///
    /// Note: VST3 ParamIDs are stable identifiers that may be sparse (e.g., 0, 5, 1000).
    /// Use `get_parameter_count()` and the underlying VST3 API to enumerate parameters.
    pub fn get_parameter_by_id(&self, param_id: u32) -> f64 {
        self.inner.get_parameter(param_id)
    }

    /// Set parameter value by native ParamID (normalized 0-1).
    ///
    /// Note: VST3 ParamIDs are stable identifiers that may be sparse (e.g., 0, 5, 1000).
    pub fn set_parameter_by_id(&mut self, param_id: u32, value: f64) {
        self.inner.set_parameter(param_id, value);
    }

    /// Get list of all parameters with their metadata.
    ///
    /// For VST3, the param_id is the native ParamID which may be sparse.
    pub fn get_parameter_list(&self) -> Vec<crate::protocol::ParameterInfo> {
        let count = self.inner.get_parameter_count();
        let mut result = Vec::with_capacity(count as usize);

        for i in 0..count {
            if let Some(info) = self.inner.get_parameter_info(i) {
                let flags = crate::protocol::ParameterFlags {
                    automatable: info.can_automate(),
                    read_only: info.is_read_only(),
                    wrap: info.is_wrap(),
                    is_bypass: info.is_bypass(),
                    hidden: info.is_hidden(),
                };

                result.push(crate::protocol::ParameterInfo {
                    id: info.id,
                    name: info.title_string(),
                    unit: info.units_string(),
                    min_value: 0.0, // VST3 uses normalized 0-1
                    max_value: 1.0,
                    default_value: info.default_normalized_value,
                    step_count: info.step_count as u32,
                    flags,
                });
            }
        }

        result
    }

    /// Get info about a specific parameter by its native ParamID.
    pub fn get_parameter_info(&self, param_id: u32) -> Option<crate::protocol::ParameterInfo> {
        // VST3 getParameterInfo takes an index, not an ID.
        // We need to iterate to find the matching ID.
        let count = self.inner.get_parameter_count();

        for i in 0..count {
            if let Some(info) = self.inner.get_parameter_info(i) {
                if info.id == param_id {
                    let flags = crate::protocol::ParameterFlags {
                        automatable: info.can_automate(),
                        read_only: info.is_read_only(),
                        wrap: info.is_wrap(),
                        is_bypass: info.is_bypass(),
                        hidden: info.is_hidden(),
                    };

                    return Some(crate::protocol::ParameterInfo {
                        id: info.id,
                        name: info.title_string(),
                        unit: info.units_string(),
                        min_value: 0.0,
                        max_value: 1.0,
                        default_value: info.default_normalized_value,
                        step_count: info.step_count as u32,
                        flags,
                    });
                }
            }
        }

        None
    }

    /// Save plugin state.
    pub fn get_state(&self) -> Result<Vec<u8>> {
        self.inner.get_state().map_err(|e| BridgeError::LoadFailed {
            path: std::path::PathBuf::from("state"),
            stage: LoadStage::Initialization,
            reason: e.to_string(),
        })
    }

    /// Load plugin state.
    pub fn set_state(&mut self, data: &[u8]) -> Result<()> {
        self.inner
            .set_state(data)
            .map_err(|e| BridgeError::LoadFailed {
                path: std::path::PathBuf::from("state"),
                stage: LoadStage::Initialization,
                reason: e.to_string(),
            })
    }

    /// Check if plugin has editor.
    pub fn has_editor(&self) -> bool {
        self.inner.has_editor()
    }

    /// Open plugin editor.
    ///
    /// # Safety
    ///
    /// The `parent` pointer must be a valid window handle.
    pub unsafe fn open_editor(&mut self, parent: *mut std::ffi::c_void) -> Result<(u32, u32)> {
        self.inner
            .open_editor(parent)
            .map_err(|e| BridgeError::LoadFailed {
                path: std::path::PathBuf::from("editor"),
                stage: LoadStage::Initialization,
                reason: e.to_string(),
            })
    }

    /// Close plugin editor.
    pub fn close_editor(&mut self) {
        self.inner.close_editor();
    }

    /// Editor idle (no-op for VST3, included for API compatibility).
    pub fn editor_idle(&mut self) {
        // VST3 editors don't have explicit idle
    }
}

// =============================================================================
// MIDI Wrapper for tutti_midi_io::MidiEvent
// =============================================================================

/// Wrapper to implement vst3_host::Vst3MidiEvent for Tutti's MidiEvent.
struct TuttiMidiWrapper<'a> {
    event: &'a MidiEvent,
}

impl<'a> From<&'a MidiEvent> for TuttiMidiWrapper<'a> {
    fn from(event: &'a MidiEvent) -> Self {
        Self { event }
    }
}

impl vst3_host::Vst3MidiEvent for TuttiMidiWrapper<'_> {
    fn sample_offset(&self) -> i32 {
        self.event.frame_offset as i32
    }

    fn to_vst3_event(&self) -> Option<vst3_host::ffi::Vst3Event> {
        use vst3_host::ffi::*;

        let channel = self.event.channel as i16;
        let header = EventHeader {
            bus_index: 0,
            sample_offset: self.event.frame_offset as i32,
            ppq_position: 0.0,
            flags: 0,
            event_type: 0,
        };

        match self.event.msg {
            ChannelVoiceMsg::NoteOn { note, velocity } => {
                let mut h = header;
                h.event_type = K_NOTE_ON_EVENT;
                Some(Vst3Event::NoteOn(NoteOnEvent {
                    header: h,
                    channel,
                    pitch: note as i16,
                    tuning: 0.0,
                    velocity: velocity as f32 / 127.0,
                    length: 0,
                    note_id: -1,
                }))
            }
            ChannelVoiceMsg::NoteOff { note, velocity } => {
                let mut h = header;
                h.event_type = K_NOTE_OFF_EVENT;
                Some(Vst3Event::NoteOff(NoteOffEvent {
                    header: h,
                    channel,
                    pitch: note as i16,
                    velocity: velocity as f32 / 127.0,
                    note_id: -1,
                    tuning: 0.0,
                }))
            }
            ChannelVoiceMsg::PolyPressure { note, pressure } => {
                let mut h = header;
                h.event_type = K_POLY_PRESSURE_EVENT;
                Some(Vst3Event::PolyPressure(PolyPressureEvent {
                    header: h,
                    channel,
                    pitch: note as i16,
                    pressure: pressure as f32 / 127.0,
                    note_id: -1,
                }))
            }
            ChannelVoiceMsg::ControlChange { control } => {
                let channel_byte = self.event.channel as u8;
                let (cc, value) = match control {
                    ControlChange::CC { control, value } => (control, value),
                    ControlChange::CCHighRes {
                        control1, value, ..
                    } => (control1, (value >> 7) as u8),
                    _ => return None,
                };

                let mut h = header;
                h.event_type = K_DATA_EVENT;
                let mut bytes = [0u8; 16];
                bytes[0] = 0xB0 | channel_byte;
                bytes[1] = cc;
                bytes[2] = value;

                Some(Vst3Event::Data(DataEvent {
                    header: h,
                    size: 3,
                    event_type: 0,
                    bytes,
                }))
            }
            ChannelVoiceMsg::ProgramChange { program } => {
                let mut h = header;
                h.event_type = K_DATA_EVENT;
                let mut bytes = [0u8; 16];
                bytes[0] = 0xC0 | (self.event.channel as u8);
                bytes[1] = program;

                Some(Vst3Event::Data(DataEvent {
                    header: h,
                    size: 2,
                    event_type: 0,
                    bytes,
                }))
            }
            ChannelVoiceMsg::ChannelPressure { pressure } => {
                let mut h = header;
                h.event_type = K_DATA_EVENT;
                let mut bytes = [0u8; 16];
                bytes[0] = 0xD0 | (self.event.channel as u8);
                bytes[1] = pressure;

                Some(Vst3Event::Data(DataEvent {
                    header: h,
                    size: 2,
                    event_type: 0,
                    bytes,
                }))
            }
            ChannelVoiceMsg::PitchBend { bend } => {
                let mut h = header;
                h.event_type = K_DATA_EVENT;
                let mut bytes = [0u8; 16];
                bytes[0] = 0xE0 | (self.event.channel as u8);
                bytes[1] = (bend & 0x7F) as u8;
                bytes[2] = ((bend >> 7) & 0x7F) as u8;

                Some(Vst3Event::Data(DataEvent {
                    header: h,
                    size: 3,
                    event_type: 0,
                    bytes,
                }))
            }
            _ => None,
        }
    }
}

// =============================================================================
// Conversion Helpers
// =============================================================================

fn convert_load_stage(stage: vst3_host::LoadStage) -> LoadStage {
    match stage {
        vst3_host::LoadStage::Scanning => LoadStage::Scanning,
        vst3_host::LoadStage::Opening => LoadStage::Opening,
        vst3_host::LoadStage::Factory => LoadStage::Factory,
        vst3_host::LoadStage::Instantiation => LoadStage::Instantiation,
        vst3_host::LoadStage::Initialization => LoadStage::Initialization,
        vst3_host::LoadStage::Setup => LoadStage::Setup,
        vst3_host::LoadStage::Activation => LoadStage::Activation,
    }
}

fn convert_transport_to_vst3(transport: &TransportInfo) -> vst3_host::TransportState {
    vst3_host::TransportState::new()
        .playing(transport.playing)
        .recording(transport.recording)
        .cycle_active(transport.cycle_active)
        .tempo(transport.tempo)
        .time_signature(transport.time_sig_numerator, transport.time_sig_denominator)
        .position_samples(transport.position_samples)
        .position_beats(transport.position_quarters)
        .bar_position_beats(transport.bar_position_quarters)
        .cycle_range(transport.cycle_start_quarters, transport.cycle_end_quarters)
}

fn convert_params_to_vst3(params: &ParameterChanges) -> vst3_host::ParameterChanges {
    let mut vst3_params = vst3_host::ParameterChanges::new();
    for queue in &params.queues {
        for point in &queue.points {
            vst3_params.add_change(queue.param_id, point.sample_offset, point.value);
        }
    }
    vst3_params
}

fn convert_params_from_vst3(params: &vst3_host::ParameterChanges) -> ParameterChanges {
    let mut tutti_params = ParameterChanges::new();
    for queue in &params.queues {
        let mut tutti_queue = ParameterQueue {
            param_id: queue.param_id,
            points: Vec::new(),
        };
        for point in &queue.points {
            tutti_queue.points.push(ParameterPoint {
                sample_offset: point.sample_offset,
                value: point.value,
            });
        }
        tutti_params.queues.push(tutti_queue);
    }
    tutti_params
}

fn convert_note_expression_to_vst3(
    note_expr: &NoteExpressionChanges,
) -> Vec<vst3_host::NoteExpressionValue> {
    note_expr
        .changes
        .iter()
        .map(|e| vst3_host::NoteExpressionValue {
            sample_offset: e.sample_offset,
            note_id: e.note_id,
            expression_type: match e.expression_type {
                NoteExpressionType::Volume => vst3_host::NoteExpressionType::Volume,
                NoteExpressionType::Pan => vst3_host::NoteExpressionType::Pan,
                NoteExpressionType::Tuning => vst3_host::NoteExpressionType::Tuning,
                NoteExpressionType::Vibrato => vst3_host::NoteExpressionType::Vibrato,
                NoteExpressionType::Brightness => vst3_host::NoteExpressionType::Brightness,
            },
            value: e.value,
        })
        .collect()
}

fn convert_vst3_midi_to_tutti(event: &vst3_host::MidiEvent) -> Option<MidiEvent> {
    let channel = Channel::from_u8(event.channel);

    let msg = match event.data {
        vst3_host::MidiData::NoteOn { note, velocity } => ChannelVoiceMsg::NoteOn {
            note,
            velocity: (velocity * 127.0) as u8,
        },
        vst3_host::MidiData::NoteOff { note, velocity } => ChannelVoiceMsg::NoteOff {
            note,
            velocity: (velocity * 127.0) as u8,
        },
        vst3_host::MidiData::PolyPressure { note, pressure } => ChannelVoiceMsg::PolyPressure {
            note,
            pressure: (pressure * 127.0) as u8,
        },
        vst3_host::MidiData::ControlChange { cc, value } => ChannelVoiceMsg::ControlChange {
            control: ControlChange::CC {
                control: cc,
                value,
            },
        },
        vst3_host::MidiData::ProgramChange { program } => {
            ChannelVoiceMsg::ProgramChange { program }
        }
        vst3_host::MidiData::ChannelPressure { pressure } => {
            ChannelVoiceMsg::ChannelPressure { pressure }
        }
        vst3_host::MidiData::PitchBend { value } => ChannelVoiceMsg::PitchBend { bend: value },
    };

    Some(MidiEvent {
        frame_offset: event.sample_offset as usize,
        channel,
        msg,
    })
}

// Implement the unified PluginInstance trait
impl crate::instance::PluginInstance for Vst3Instance {
    fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    fn supports_f64(&self) -> bool {
        self.inner.supports_f64()
    }

    fn process_f32<'a>(
        &mut self,
        buffer: &'a mut AudioBuffer<'a>,
        ctx: &crate::instance::ProcessContext,
    ) -> crate::instance::ProcessOutput {
        let vst3_midi: Vec<TuttiMidiWrapper> =
            ctx.midi_events.iter().map(TuttiMidiWrapper::from).collect();

        let mut vst3_buffer = vst3_host::AudioBuffer::new(
            buffer.inputs,
            buffer.outputs,
            buffer.num_samples,
            buffer.sample_rate as f64,
        );

        let vst3_transport = ctx
            .transport
            .map(convert_transport_to_vst3)
            .unwrap_or_else(vst3_host::TransportState::new);

        let vst3_params = ctx.param_changes.map(convert_params_to_vst3);
        let vst3_note_expr = ctx
            .note_expression
            .map(convert_note_expression_to_vst3)
            .unwrap_or_default();

        let (output_midi, output_params) = self.inner.process(
            &mut vst3_buffer,
            &vst3_midi,
            vst3_params.as_ref(),
            &vst3_note_expr,
            &vst3_transport,
        );

        let midi_events = output_midi
            .into_iter()
            .filter_map(|e| convert_vst3_midi_to_tutti(&e))
            .collect();
        let param_changes = convert_params_from_vst3(&output_params);

        crate::instance::ProcessOutput {
            midi_events,
            param_changes,
            note_expression: NoteExpressionChanges::new(),
        }
    }

    fn process_f64<'a>(
        &mut self,
        buffer: &'a mut AudioBuffer64<'a>,
        ctx: &crate::instance::ProcessContext,
    ) -> crate::instance::ProcessOutput {
        let vst3_midi: Vec<TuttiMidiWrapper> =
            ctx.midi_events.iter().map(TuttiMidiWrapper::from).collect();

        let mut vst3_buffer = vst3_host::AudioBuffer::new(
            buffer.inputs,
            buffer.outputs,
            buffer.num_samples,
            buffer.sample_rate as f64,
        );

        let vst3_transport = ctx
            .transport
            .map(convert_transport_to_vst3)
            .unwrap_or_else(vst3_host::TransportState::new);

        let vst3_params = ctx.param_changes.map(convert_params_to_vst3);
        let vst3_note_expr = ctx
            .note_expression
            .map(convert_note_expression_to_vst3)
            .unwrap_or_default();

        let (output_midi, output_params) = self.inner.process(
            &mut vst3_buffer,
            &vst3_midi,
            vst3_params.as_ref(),
            &vst3_note_expr,
            &vst3_transport,
        );

        let midi_events = output_midi
            .into_iter()
            .filter_map(|e| convert_vst3_midi_to_tutti(&e))
            .collect();
        let param_changes = convert_params_from_vst3(&output_params);

        crate::instance::ProcessOutput {
            midi_events,
            param_changes,
            note_expression: NoteExpressionChanges::new(),
        }
    }

    fn set_sample_rate(&mut self, rate: f64) {
        self.inner.set_sample_rate(rate);
    }

    fn get_parameter_count(&self) -> usize {
        self.inner.get_parameter_count() as usize
    }

    fn get_parameter(&self, id: u32) -> f64 {
        self.inner.get_parameter(id)
    }

    fn set_parameter(&mut self, id: u32, value: f64) {
        self.inner.set_parameter(id, value);
    }

    fn get_parameter_list(&mut self) -> Vec<crate::protocol::ParameterInfo> {
        Vst3Instance::get_parameter_list(self)
    }

    fn get_parameter_info(&mut self, id: u32) -> Option<crate::protocol::ParameterInfo> {
        Vst3Instance::get_parameter_info(self, id)
    }

    fn has_editor(&mut self) -> bool {
        self.inner.has_editor()
    }

    fn open_editor(&mut self, parent: *mut std::ffi::c_void) -> crate::error::Result<(u32, u32)> {
        unsafe { Vst3Instance::open_editor(self, parent) }
    }

    fn close_editor(&mut self) {
        self.inner.close_editor();
    }

    fn editor_idle(&mut self) {
        // VST3 doesn't have explicit idle
    }

    fn get_state(&mut self) -> crate::error::Result<Vec<u8>> {
        Vst3Instance::get_state(self)
    }

    fn set_state(&mut self, data: &[u8]) -> crate::error::Result<()> {
        Vst3Instance::set_state(self, data)
    }
}
