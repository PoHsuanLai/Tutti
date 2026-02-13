//! VST3 plugin loader using the `vst3-host` crate.

use std::path::Path;

use tutti_plugin::protocol::{
    AudioBuffer, AudioBuffer64, MidiEvent, MidiEventVec, NoteExpressionChanges, NoteExpressionType,
    ParameterChanges, ParameterPoint, ParameterQueue, TransportInfo,
};
use tutti_plugin::{BridgeError, LoadStage, PluginMetadata, Result};

use tutti_midi_io::{Channel, ChannelVoiceMsg, ControlChange};

pub use vst3_host;

pub struct Vst3Instance {
    inner: vst3_host::Vst3Instance,
    metadata: PluginMetadata,
}

impl Vst3Instance {
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
        let has_editor = inner.has_editor();
        let metadata = PluginMetadata::new(info.id.clone(), info.name.clone())
            .author(info.vendor.clone())
            .version(info.version.clone())
            .audio_io(info.num_inputs, info.num_outputs)
            .midi(info.has_midi_input)
            .f64_support(info.supports_f64)
            .editor(has_editor, None);

        Ok(Self { inner, metadata })
    }

    pub fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    pub fn supports_f64(&self) -> bool {
        self.inner.supports_f64()
    }

    /// Alias for supports_f64() for compatibility with server.rs.
    pub fn can_process_f64(&self) -> bool {
        self.inner.supports_f64()
    }

    pub fn set_sample_format(
        &mut self,
        format: tutti_plugin::protocol::SampleFormat,
    ) -> Result<()> {
        let use_f64 = matches!(format, tutti_plugin::protocol::SampleFormat::Float64);
        self.inner
            .set_use_f64(use_f64)
            .map_err(|e| BridgeError::LoadFailed {
                path: std::path::PathBuf::from("format"),
                stage: LoadStage::Initialization,
                reason: e.to_string(),
            })
    }

    pub fn process_with_midi<'a>(
        &mut self,
        buffer: &'a mut AudioBuffer<'a>,
        midi_events: &[MidiEvent],
    ) -> MidiEventVec {
        let vst3_midi: Vec<TuttiMidiWrapper> =
            midi_events.iter().map(TuttiMidiWrapper::from).collect();

        let mut vst3_buffer = vst3_host::AudioBuffer::new(
            buffer.inputs,
            buffer.outputs,
            buffer.num_samples,
            buffer.sample_rate,
        );

        let transport = vst3_host::TransportState::new();

        let (output_midi, _) =
            self.inner
                .process(&mut vst3_buffer, &vst3_midi, None, &[], &transport);

        output_midi
            .into_iter()
            .filter_map(|e| convert_vst3_midi_to_tutti(&e))
            .collect()
    }

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
            buffer.sample_rate,
        );

        let vst3_transport = convert_transport_to_vst3(transport);

        let (output_midi, _) =
            self.inner
                .process(&mut vst3_buffer, &vst3_midi, None, &[], &vst3_transport);

        output_midi
            .into_iter()
            .filter_map(|e| convert_vst3_midi_to_tutti(&e))
            .collect()
    }

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
            buffer.sample_rate,
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
            buffer.sample_rate,
        );

        let transport = vst3_host::TransportState::new();
        let (output_midi, _) =
            self.inner
                .process(&mut vst3_buffer, &vst3_midi, None, &[], &transport);

        output_midi
            .into_iter()
            .filter_map(|e| convert_vst3_midi_to_tutti(&e))
            .collect()
    }

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
            buffer.sample_rate,
        );

        let vst3_transport = convert_transport_to_vst3(transport);
        let (output_midi, _) =
            self.inner
                .process(&mut vst3_buffer, &vst3_midi, None, &[], &vst3_transport);

        output_midi
            .into_iter()
            .filter_map(|e| convert_vst3_midi_to_tutti(&e))
            .collect()
    }

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
            buffer.sample_rate,
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

    /// No MIDI, no automation.
    pub fn process_f64<'a>(&mut self, buffer: &'a mut AudioBuffer64<'a>) {
        let mut vst3_buffer = vst3_host::AudioBuffer::new(
            buffer.inputs,
            buffer.outputs,
            buffer.num_samples,
            buffer.sample_rate,
        );

        let transport = vst3_host::TransportState::new();
        let empty_midi: Vec<TuttiMidiWrapper> = vec![];
        let _ = self
            .inner
            .process::<f64, _>(&mut vst3_buffer, &empty_midi, None, &[], &transport);
    }

    pub fn set_sample_rate(&mut self, rate: f64) {
        self.inner.set_sample_rate(rate);
    }

    pub fn get_parameter_count(&self) -> i32 {
        self.inner.get_parameter_count()
    }

    /// Note: VST3 ParamIDs are stable identifiers that may be sparse (e.g., 0, 5, 1000).
    /// Use `get_parameter_count()` and the underlying VST3 API to enumerate parameters.
    pub fn get_parameter_by_id(&self, param_id: u32) -> f64 {
        self.inner.get_parameter(param_id)
    }

    pub fn set_parameter_by_id(&mut self, param_id: u32, value: f64) {
        self.inner.set_parameter(param_id, value);
    }

    pub fn get_parameter_list(&self) -> Vec<tutti_plugin::protocol::ParameterInfo> {
        let count = self.inner.get_parameter_count();
        let mut result = Vec::with_capacity(count as usize);

        for i in 0..count {
            if let Some(info) = self.inner.get_parameter_info(i) {
                let flags = tutti_plugin::protocol::ParameterFlags {
                    automatable: info.can_automate(),
                    read_only: info.is_read_only(),
                    wrap: info.is_wrap(),
                    is_bypass: info.is_bypass(),
                    hidden: info.is_hidden(),
                };

                result.push(tutti_plugin::protocol::ParameterInfo {
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

    pub fn get_parameter_info(
        &self,
        param_id: u32,
    ) -> Option<tutti_plugin::protocol::ParameterInfo> {
        // VST3 getParameterInfo takes an index, not an ID.
        // We need to iterate to find the matching ID.
        let count = self.inner.get_parameter_count();

        for i in 0..count {
            if let Some(info) = self.inner.get_parameter_info(i) {
                if info.id == param_id {
                    let flags = tutti_plugin::protocol::ParameterFlags {
                        automatable: info.can_automate(),
                        read_only: info.is_read_only(),
                        wrap: info.is_wrap(),
                        is_bypass: info.is_bypass(),
                        hidden: info.is_hidden(),
                    };

                    return Some(tutti_plugin::protocol::ParameterInfo {
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

    pub fn get_state(&self) -> Result<Vec<u8>> {
        self.inner
            .get_state()
            .map_err(|e| BridgeError::StateSaveError(e.to_string()))
    }

    pub fn set_state(&mut self, data: &[u8]) -> Result<()> {
        self.inner
            .set_state(data)
            .map_err(|e| BridgeError::StateRestoreError(e.to_string()))
    }

    pub fn has_editor(&self) -> bool {
        self.inner.has_editor()
    }

    /// # Safety
    ///
    /// The `parent` pointer must be a valid window handle.
    pub unsafe fn open_editor(&mut self, parent: *mut std::ffi::c_void) -> Result<(u32, u32)> {
        self.inner
            .open_editor(parent)
            .map_err(|e| BridgeError::EditorError(e.to_string()))
    }

    pub fn close_editor(&mut self) {
        self.inner.close_editor();
    }

    /// Editor idle (no-op for VST3, included for API compatibility).
    pub fn editor_idle(&mut self) {
        // VST3 editors don't have explicit idle
    }
}

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
                let bend = bend.min(16383);
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
        let mut tutti_queue = ParameterQueue::new(queue.param_id);
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
            control: ControlChange::CC { control: cc, value },
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
            buffer.sample_rate,
        );

        let vst3_transport = ctx
            .transport
            .map(convert_transport_to_vst3)
            .unwrap_or_default();

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
            buffer.sample_rate,
        );

        let vst3_transport = ctx
            .transport
            .map(convert_transport_to_vst3)
            .unwrap_or_default();

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

    fn get_parameter_list(&mut self) -> Vec<tutti_plugin::protocol::ParameterInfo> {
        Vst3Instance::get_parameter_list(self)
    }

    fn get_parameter_info(&mut self, id: u32) -> Option<tutti_plugin::protocol::ParameterInfo> {
        Vst3Instance::get_parameter_info(self, id)
    }

    fn has_editor(&mut self) -> bool {
        self.inner.has_editor()
    }

    unsafe fn open_editor(&mut self, parent: *mut std::ffi::c_void) -> Result<(u32, u32)> {
        Vst3Instance::open_editor(self, parent)
    }

    fn close_editor(&mut self) {
        self.inner.close_editor();
    }

    fn editor_idle(&mut self) {
        // VST3 doesn't have explicit idle
    }

    fn get_state(&mut self) -> Result<Vec<u8>> {
        Vst3Instance::get_state(self)
    }

    fn set_state(&mut self, data: &[u8]) -> Result<()> {
        Vst3Instance::set_state(self, data)
    }
}

#[cfg(test)]
#[cfg(feature = "vst3")]
mod tests {
    use super::*;
    use crate::instance::PluginInstance;
    use std::path::Path;
    use tutti_midi_io::{Channel, ChannelVoiceMsg};
    use tutti_plugin::protocol::MidiEvent;

    const VST3_PLUGIN: &str = "/Library/Audio/Plug-Ins/VST3/TAL-NoiseMaker.vst3";

    #[test]
    fn test_vst3_load() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST3_PLUGIN);
        let instance = Vst3Instance::load(path, 44100.0, 512);
        assert!(
            instance.is_ok(),
            "Failed to load VST3 plugin: {:?}",
            instance.err()
        );

        let instance = instance.unwrap();
        let meta = instance.metadata();
        assert!(!meta.name.is_empty(), "Plugin name should not be empty");
        assert!(!meta.id.is_empty(), "Plugin id should not be empty");
    }

    #[test]
    fn test_vst3_metadata() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST3_PLUGIN);
        let instance = Vst3Instance::load(path, 44100.0, 512).expect("Failed to load VST3 plugin");
        let meta = instance.metadata();

        assert!(
            meta.audio_io.outputs > 0,
            "Expected audio outputs > 0, got {}",
            meta.audio_io.outputs
        );
    }

    #[test]
    fn test_vst3_parameter_count() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST3_PLUGIN);
        let instance = Vst3Instance::load(path, 44100.0, 512).expect("Failed to load VST3 plugin");

        let count = instance.get_parameter_count();
        assert!(
            count > 0,
            "TAL-NoiseMaker should have parameters, got {}",
            count
        );
    }

    #[test]
    fn test_vst3_parameter_list() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST3_PLUGIN);
        let instance = Vst3Instance::load(path, 44100.0, 512).expect("Failed to load VST3 plugin");

        let params = instance.get_parameter_list();
        assert!(
            !params.is_empty(),
            "Parameter list should not be empty for TAL-NoiseMaker"
        );

        for param in &params {
            assert!(
                !param.name.is_empty(),
                "Parameter id {} has empty name",
                param.id
            );
        }
    }

    #[test]
    fn test_vst3_get_parameter() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST3_PLUGIN);
        let instance = Vst3Instance::load(path, 44100.0, 512).expect("Failed to load VST3 plugin");

        let params = instance.get_parameter_list();
        assert!(!params.is_empty(), "Need at least one parameter");

        let first_id = params[0].id;
        let value = instance.get_parameter_by_id(first_id);
        assert!(
            value.is_finite(),
            "Parameter value should be finite, got {}",
            value
        );
    }

    #[test]
    fn test_vst3_process_f32_silence() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST3_PLUGIN);
        let mut instance =
            Vst3Instance::load(path, 44100.0, 512).expect("Failed to load VST3 plugin");

        let num_samples = 512;
        let input_data = vec![vec![0.0f32; num_samples]; 2];
        let mut output_data = vec![vec![0.0f32; num_samples]; 2];

        let input_slices: Vec<&[f32]> = input_data.iter().map(|v| v.as_slice()).collect();
        let mut output_slices: Vec<&mut [f32]> =
            output_data.iter_mut().map(|v| v.as_mut_slice()).collect();

        let mut buffer = AudioBuffer {
            inputs: &input_slices,
            outputs: &mut output_slices,
            num_samples,
            sample_rate: 44100.0,
        };

        let ctx = crate::instance::ProcessContext::new();
        // Should not panic
        let _output = PluginInstance::process_f32(&mut instance, &mut buffer, &ctx);
    }

    #[test]
    fn test_vst3_process_f32_with_note() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST3_PLUGIN);
        let mut instance =
            Vst3Instance::load(path, 44100.0, 512).expect("Failed to load VST3 plugin");

        let num_samples = 512;
        let note_on = [MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::NoteOn {
                note: 60,
                velocity: 100,
            },
        }];

        let mut has_nonzero = false;

        // Process block with NoteOn
        {
            let input_data = vec![vec![0.0f32; num_samples]; 2];
            let mut output_data = vec![vec![0.0f32; num_samples]; 2];

            let input_slices: Vec<&[f32]> = input_data.iter().map(|v| v.as_slice()).collect();
            let mut output_slices: Vec<&mut [f32]> =
                output_data.iter_mut().map(|v| v.as_mut_slice()).collect();

            let mut buffer = AudioBuffer {
                inputs: &input_slices,
                outputs: &mut output_slices,
                num_samples,
                sample_rate: 44100.0,
            };

            let ctx = crate::instance::ProcessContext::new().midi(&note_on);
            let _output = PluginInstance::process_f32(&mut instance, &mut buffer, &ctx);

            for ch in output_data.iter() {
                for &sample in ch.iter() {
                    if sample != 0.0 {
                        has_nonzero = true;
                    }
                }
            }
        }

        // Process additional blocks to give the synth time to produce sound
        let empty_ctx = crate::instance::ProcessContext::new();
        for _ in 0..4 {
            let input_data = vec![vec![0.0f32; num_samples]; 2];
            let mut output_data = vec![vec![0.0f32; num_samples]; 2];

            let input_slices: Vec<&[f32]> = input_data.iter().map(|v| v.as_slice()).collect();
            let mut output_slices: Vec<&mut [f32]> =
                output_data.iter_mut().map(|v| v.as_mut_slice()).collect();

            let mut buffer = AudioBuffer {
                inputs: &input_slices,
                outputs: &mut output_slices,
                num_samples,
                sample_rate: 44100.0,
            };

            let _output = PluginInstance::process_f32(&mut instance, &mut buffer, &empty_ctx);

            for ch in output_data.iter() {
                for &sample in ch.iter() {
                    if sample != 0.0 {
                        has_nonzero = true;
                    }
                }
            }
        }

        assert!(
            has_nonzero,
            "Expected at least one non-zero output sample after NoteOn"
        );
    }

    // =========================================================================
    // Voxengo SPAN / Boogex — f64 support tests (local fixture plugins)
    // =========================================================================

    const SPAN_VST3: &str = "tests/fixtures/plugins/SPAN.vst3";
    const BOOGEX_VST3: &str = "tests/fixtures/plugins/Boogex.vst3";

    /// Resolve fixture path relative to workspace root.
    fn fixture_path(relative: &str) -> std::path::PathBuf {
        let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // Go up from crates/tutti-plugin-server to the workspace root (crates/tutti)
        p.pop();
        p.pop();
        p.push(relative);
        p
    }

    #[test]
    fn test_voxengo_span_load_and_f64_support() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = fixture_path(SPAN_VST3);
        if !path.exists() {
            eprintln!("Skipping: SPAN.vst3 not found at {:?}", path);
            return;
        }
        let instance = Vst3Instance::load(&path, 44100.0, 512);
        assert!(
            instance.is_ok(),
            "Failed to load SPAN: {:?}",
            instance.err()
        );

        let instance = instance.unwrap();
        let meta = instance.metadata();
        eprintln!(
            "SPAN: name={}, supports_f64={}",
            meta.name, meta.supports_f64
        );
        assert!(!meta.name.is_empty());
    }

    #[test]
    fn test_voxengo_boogex_load_and_f64_support() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = fixture_path(BOOGEX_VST3);
        if !path.exists() {
            eprintln!("Skipping: Boogex.vst3 not found at {:?}", path);
            return;
        }
        let instance = Vst3Instance::load(&path, 44100.0, 512);
        assert!(
            instance.is_ok(),
            "Failed to load Boogex: {:?}",
            instance.err()
        );

        let instance = instance.unwrap();
        let meta = instance.metadata();
        eprintln!(
            "Boogex: name={}, supports_f64={}",
            meta.name, meta.supports_f64
        );
        assert!(!meta.name.is_empty());
    }

    #[test]
    fn test_voxengo_span_process_f64() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = fixture_path(SPAN_VST3);
        if !path.exists() {
            eprintln!("Skipping: SPAN.vst3 not found at {:?}", path);
            return;
        }
        let mut instance = Vst3Instance::load(&path, 44100.0, 512).expect("Failed to load SPAN");

        if !instance.supports_f64() {
            eprintln!("SPAN does not report f64 support, skipping f64 test");
            return;
        }

        // Enable f64 processing
        instance
            .set_sample_format(tutti_plugin::protocol::SampleFormat::Float64)
            .expect("Failed to set f64 format");

        let num_samples = 512;
        // Feed a 440Hz sine wave to test pass-through
        let input_data: Vec<Vec<f64>> = (0..2)
            .map(|_| {
                (0..num_samples)
                    .map(|i| (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 44100.0).sin() * 0.5)
                    .collect()
            })
            .collect();
        let mut output_data = vec![vec![0.0f64; num_samples]; 2];

        let input_slices: Vec<&[f64]> = input_data.iter().map(|v| v.as_slice()).collect();
        let mut output_slices: Vec<&mut [f64]> =
            output_data.iter_mut().map(|v| v.as_mut_slice()).collect();

        let mut buffer = AudioBuffer64 {
            inputs: &input_slices,
            outputs: &mut output_slices,
            num_samples,
            sample_rate: 44100.0,
        };

        let ctx = crate::instance::ProcessContext::new();
        let _output = PluginInstance::process_f64(&mut instance, &mut buffer, &ctx);

        // SPAN is an analyzer — it should pass audio through unchanged
        let mut all_zero = true;
        for ch in &output_data {
            for &s in ch {
                if s != 0.0 {
                    all_zero = false;
                    break;
                }
            }
        }
        eprintln!(
            "SPAN f64 output: first sample = {}, all_zero = {}",
            output_data[0][0], all_zero
        );
    }

    #[test]
    fn test_voxengo_boogex_process_f64() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = fixture_path(BOOGEX_VST3);
        if !path.exists() {
            eprintln!("Skipping: Boogex.vst3 not found at {:?}", path);
            return;
        }
        let mut instance = Vst3Instance::load(&path, 44100.0, 512).expect("Failed to load Boogex");

        if !instance.supports_f64() {
            eprintln!("Boogex does not report f64 support, skipping f64 test");
            return;
        }

        // Enable f64 processing
        instance
            .set_sample_format(tutti_plugin::protocol::SampleFormat::Float64)
            .expect("Failed to set f64 format");

        let num_samples = 512;
        // Feed a sine wave — Boogex is an amp sim so it should transform the audio
        let input_data: Vec<Vec<f64>> = (0..2)
            .map(|_| {
                (0..num_samples)
                    .map(|i| (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 44100.0).sin() * 0.5)
                    .collect()
            })
            .collect();
        let mut output_data = vec![vec![0.0f64; num_samples]; 2];

        let input_slices: Vec<&[f64]> = input_data.iter().map(|v| v.as_slice()).collect();
        let mut output_slices: Vec<&mut [f64]> =
            output_data.iter_mut().map(|v| v.as_mut_slice()).collect();

        let mut buffer = AudioBuffer64 {
            inputs: &input_slices,
            outputs: &mut output_slices,
            num_samples,
            sample_rate: 44100.0,
        };

        let ctx = crate::instance::ProcessContext::new();
        let _output = PluginInstance::process_f64(&mut instance, &mut buffer, &ctx);

        let mut has_nonzero = false;
        for ch in &output_data {
            for &s in ch {
                if s != 0.0 {
                    has_nonzero = true;
                    break;
                }
            }
        }
        eprintln!(
            "Boogex f64 output: first sample = {}, has_nonzero = {}",
            output_data[0][0], has_nonzero
        );
    }
}
