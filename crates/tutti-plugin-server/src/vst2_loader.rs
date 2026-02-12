//! VST2 plugin loader.

use std::path::Path;
use std::sync::{Arc, Mutex};
use tutti_midi_io::{ChannelVoiceMsg, ControlChange};
use tutti_plugin::protocol::{AudioBuffer, MidiEvent};
use tutti_plugin::{BridgeError, LoadStage, PluginMetadata, Result};

#[cfg(feature = "vst2")]
use vst::host::{Host, PluginLoader};
#[cfg(feature = "vst2")]
use vst::plugin::Plugin as VstPlugin;

pub type ParameterChange = (i32, f32);

/// Wrapper to make `Box<dyn Editor>` Send.
/// Safety: The editor is only accessed from one thread at a time via the
/// InProcessBridge's parking_lot::Mutex. GUI methods are called on the main
/// thread; the bridge thread never touches the editor.
#[cfg(feature = "vst2")]
struct SendEditor(Box<dyn vst::editor::Editor>);
#[cfg(feature = "vst2")]
unsafe impl Send for SendEditor {}

/// Wrapper to make `Arc<dyn PluginParameters>` Send.
/// Safety: The concrete type (`PluginParametersInstance`) is `Send + Sync`,
/// but that info is erased by `get_parameter_object()` returning `Arc<dyn PluginParameters>`.
#[cfg(feature = "vst2")]
struct SendParams(std::sync::Arc<dyn vst::plugin::PluginParameters>);
#[cfg(feature = "vst2")]
unsafe impl Send for SendParams {}

#[cfg(feature = "vst2")]
impl std::ops::Deref for SendParams {
    type Target = dyn vst::plugin::PluginParameters;
    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

pub struct Vst2Instance {
    #[cfg(feature = "vst2")]
    instance: vst::host::PluginInstance,
    #[cfg(feature = "vst2")]
    #[allow(dead_code)]
    host: Arc<Mutex<BridgeHost>>,
    #[cfg(feature = "vst2")]
    time_info: Arc<arc_swap::ArcSwap<Option<vst::api::TimeInfo>>>,
    #[cfg(feature = "vst2")]
    editor: Option<SendEditor>,
    #[cfg(feature = "vst2")]
    params: SendParams,
    metadata: PluginMetadata,
    sample_rate: f64,
    #[cfg(feature = "vst2")]
    param_rx: crossbeam_channel::Receiver<ParameterChange>,
}

impl Vst2Instance {
    /// Resolve a macOS `.vst` bundle path to the inner mach-o binary.
    ///
    /// On macOS, VST2 plugins are bundles (directories) with the actual dylib at
    /// `Contents/MacOS/<name>`. The `vst` crate's `PluginLoader` calls `dlopen`
    /// directly and doesn't resolve bundles, so we do it here.
    #[cfg(feature = "vst2")]
    fn resolve_bundle_path(path: &Path) -> std::path::PathBuf {
        if path.is_dir() && path.extension().and_then(|e| e.to_str()) == Some("vst") {
            let stem = path.file_stem().unwrap_or_default();
            let inner = path.join("Contents").join("MacOS").join(stem);
            if inner.exists() {
                return inner;
            }
        }
        path.to_path_buf()
    }

    pub fn load(path: &Path, sample_rate: f64, block_size: usize) -> Result<Self> {
        #[cfg(feature = "vst2")]
        {
            let resolved = Self::resolve_bundle_path(path);
            let (param_tx, param_rx) = crossbeam_channel::unbounded();
            let time_info = Arc::new(arc_swap::ArcSwap::from_pointee(None));
            let host = Arc::new(Mutex::new(BridgeHost::new(
                param_tx,
                Arc::clone(&time_info),
            )));

            let mut loader =
                PluginLoader::load(&resolved, Arc::clone(&host)).map_err(|e| {
                    BridgeError::LoadFailed {
                        path: path.to_path_buf(),
                        stage: LoadStage::Opening,
                        reason: format!("Failed to load VST: {:?}", e),
                    }
                })?;

            let mut instance = loader.instance().map_err(|e| BridgeError::LoadFailed {
                path: path.to_path_buf(),
                stage: LoadStage::Instantiation,
                reason: format!("Failed to create instance: {:?}", e),
            })?;

            instance.init();
            instance.set_sample_rate(sample_rate as f32);
            instance.set_block_size(block_size as i64);
            instance.resume();

            let info = instance.get_info();
            // get_editor() can only be called once per PluginInstance (the vst crate
            // sets is_editor_active = true on first call, returns None on subsequent).
            // We call it here and store the result for later use.
            let editor = instance.get_editor().map(SendEditor);
            let has_editor = editor.is_some();
            let metadata =
                PluginMetadata::new(format!("vst2.{}", info.unique_id), info.name.clone())
                    .author(info.vendor.clone())
                    .version(format!("{}", info.version))
                    .audio_io(info.inputs as usize, info.outputs as usize)
                    .midi(info.midi_inputs > 0 || info.midi_outputs > 0)
                    .f64_support(info.f64_precision)
                    .editor(has_editor, None);

            let params = SendParams(instance.get_parameter_object());

            Ok(Self {
                instance,
                host,
                time_info,
                editor,
                params,
                metadata,
                sample_rate,
                param_rx,
            })
        }

        #[cfg(not(feature = "vst2"))]
        {
            let _ = (path, sample_rate, block_size);
            Err(BridgeError::LoadFailed {
                path: path.to_path_buf(),
                stage: LoadStage::Opening,
                reason: "VST2 support not compiled (enable 'vst2' feature)".to_string(),
            })
        }
    }

    pub fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    pub fn supports_f64(&self) -> bool {
        self.metadata.supports_f64
    }

    pub fn process(
        &mut self,
        buffer: &mut AudioBuffer,
        midi_events: &[MidiEvent],
        transport: Option<&tutti_plugin::protocol::TransportInfo>,
    ) -> tutti_plugin::protocol::MidiEventVec {
        #[cfg(feature = "vst2")]
        {
            if let Some(t) = transport {
                self.time_info
                    .store(Arc::new(Some(build_vst2_time_info(t, buffer.sample_rate))));
            }
            self.process_f32_internal(buffer, midi_events)
        }

        #[cfg(not(feature = "vst2"))]
        {
            let _ = (buffer, midi_events, transport);
            smallvec::SmallVec::new()
        }
    }

    /// Converts f64 → f32, processes, then converts back (VST2 crate limitation).
    pub fn process_f64(
        &mut self,
        buffer: &mut tutti_plugin::protocol::AudioBuffer64,
        midi_events: &[MidiEvent],
        transport: Option<&tutti_plugin::protocol::TransportInfo>,
    ) -> tutti_plugin::protocol::MidiEventVec {
        #[cfg(feature = "vst2")]
        {
            if let Some(t) = transport {
                self.time_info
                    .store(Arc::new(Some(build_vst2_time_info(t, buffer.sample_rate))));
            }
            self.process_f64_internal(buffer, midi_events)
        }

        #[cfg(not(feature = "vst2"))]
        {
            let _ = (buffer, midi_events, transport);
            smallvec::SmallVec::new()
        }
    }

    #[cfg(feature = "vst2")]
    fn process_f32_internal(
        &mut self,
        buffer: &mut AudioBuffer,
        midi_events: &[MidiEvent],
    ) -> tutti_plugin::protocol::MidiEventVec {
        use vst::buffer::AudioBuffer as VstBuffer;

        let num_samples = buffer.num_samples;
        if num_samples == 0 {
            return smallvec::SmallVec::new();
        }

        self.send_midi_events(midi_events);

        let mut input_vecs: Vec<Vec<f32>> =
            buffer.inputs.iter().map(|slice| slice.to_vec()).collect();
        let mut output_vecs: Vec<Vec<f32>> = buffer
            .outputs
            .iter()
            .map(|_| vec![0.0; num_samples])
            .collect();

        let info = self.instance.get_info();
        while input_vecs.len() < info.inputs as usize {
            input_vecs.push(vec![0.0; num_samples]);
        }
        while output_vecs.len() < info.outputs as usize {
            output_vecs.push(vec![0.0; num_samples]);
        }

        let input_ptrs: Vec<*const f32> = input_vecs.iter().map(|v| v.as_ptr()).collect();
        let mut output_ptrs: Vec<*mut f32> =
            output_vecs.iter_mut().map(|v| v.as_mut_ptr()).collect();

        let mut vst_buffer = unsafe {
            VstBuffer::from_raw(
                input_ptrs.len(),
                output_ptrs.len(),
                input_ptrs.as_ptr(),
                output_ptrs.as_mut_ptr(),
                num_samples,
            )
        };

        self.instance.process(&mut vst_buffer);

        for (i, out_channel) in buffer.outputs.iter_mut().enumerate() {
            if i < output_vecs.len() {
                out_channel.copy_from_slice(&output_vecs[i][..num_samples]);
            }
        }

        smallvec::SmallVec::new()
    }

    #[cfg(feature = "vst2")]
    fn process_f64_internal(
        &mut self,
        buffer: &mut tutti_plugin::protocol::AudioBuffer64,
        midi_events: &[MidiEvent],
    ) -> tutti_plugin::protocol::MidiEventVec {
        use vst::buffer::AudioBuffer as VstBuffer;

        let num_samples = buffer.num_samples;
        if num_samples == 0 {
            return smallvec::SmallVec::new();
        }

        self.send_midi_events(midi_events);

        let mut input_vecs: Vec<Vec<f32>> = buffer
            .inputs
            .iter()
            .map(|ch| ch.iter().map(|&s| s as f32).collect())
            .collect();
        let mut output_vecs: Vec<Vec<f32>> = buffer
            .outputs
            .iter()
            .map(|_| vec![0.0f32; num_samples])
            .collect();

        let info = self.instance.get_info();
        while input_vecs.len() < info.inputs as usize {
            input_vecs.push(vec![0.0; num_samples]);
        }
        while output_vecs.len() < info.outputs as usize {
            output_vecs.push(vec![0.0; num_samples]);
        }

        let input_ptrs: Vec<*const f32> = input_vecs.iter().map(|v| v.as_ptr()).collect();
        let mut output_ptrs: Vec<*mut f32> =
            output_vecs.iter_mut().map(|v| v.as_mut_ptr()).collect();

        let mut vst_buffer = unsafe {
            VstBuffer::from_raw(
                input_ptrs.len(),
                output_ptrs.len(),
                input_ptrs.as_ptr(),
                output_ptrs.as_mut_ptr(),
                num_samples,
            )
        };

        self.instance.process(&mut vst_buffer);

        for (i, out_channel) in buffer.outputs.iter_mut().enumerate() {
            if i < output_vecs.len() {
                for (j, sample) in out_channel.iter_mut().enumerate().take(num_samples) {
                    *sample = output_vecs[i][j] as f64;
                }
            }
        }

        smallvec::SmallVec::new()
    }

    #[cfg(feature = "vst2")]
    fn send_midi_events(&mut self, midi_events: &[MidiEvent]) {
        use vst::api;

        if midi_events.is_empty() {
            return;
        }

        let mut api_events: Vec<api::MidiEvent> = midi_events
            .iter()
            .filter_map(Self::midi_to_api_event)
            .collect();

        if api_events.is_empty() {
            return;
        }

        let num_events = api_events.len() as i32;

        // Build pointers into our Vec. Using as_mut_ptr() gives *mut MidiEvent
        // which we cast to *mut Event (the VST2 API's base event type).
        // Safety: api_events Vec is not moved/reallocated while pointers are live.
        let event_ptrs: Vec<*mut api::Event> = api_events
            .iter_mut()
            .map(|e| e as *mut api::MidiEvent as *mut api::Event)
            .collect();

        // api::Events has a [*mut Event; 2] flexible array member.
        // For >2 events we allocate a buffer with enough room for extra pointers,
        // using the struct's actual layout (offset_of events field).
        let events_offset = std::mem::offset_of!(api::Events, events);
        let needed = events_offset + event_ptrs.len() * std::mem::size_of::<*mut api::Event>();
        let alloc_size = needed.max(std::mem::size_of::<api::Events>());

        // Use a Vec<u64> for 8-byte alignment (api::Events requires isize alignment).
        let u64_count = (alloc_size + 7) / 8;
        let mut buf = vec![0u64; u64_count];

        unsafe {
            let p = buf.as_mut_ptr() as *mut u8;
            let events = &mut *(p as *mut api::Events);
            events.num_events = num_events;
            events._reserved = 0;
            let base = p.add(events_offset) as *mut *mut api::Event;
            for (i, ptr) in event_ptrs.iter().enumerate() {
                *base.add(i) = *ptr;
            }
            self.instance.process_events(events);
        }
        // api_events (MidiEvent data) and buf (Events header) are dropped here,
        // after process_events has returned. VST2 spec says plugins must copy
        // event data during processEvents if they need it later.
    }

    #[cfg(feature = "vst2")]
    fn midi_to_api_event(event: &MidiEvent) -> Option<vst::api::MidiEvent> {
        use std::mem;
        use vst::api;

        let channel_num = event.channel as u8;
        let (status, data1, data2) = match event.msg {
            ChannelVoiceMsg::NoteOn { note, velocity } => (0x90 | channel_num, note, velocity),
            ChannelVoiceMsg::NoteOff { note, velocity } => (0x80 | channel_num, note, velocity),
            ChannelVoiceMsg::PolyPressure { note, pressure } => {
                (0xA0 | channel_num, note, pressure)
            }
            ChannelVoiceMsg::ControlChange { control } => {
                let (cc, value) = match control {
                    ControlChange::CC { control, value } => (control, value),
                    ControlChange::CCHighRes {
                        control1, value, ..
                    } => (control1, (value >> 7) as u8),
                    _ => return None,
                };
                (0xB0 | channel_num, cc, value)
            }
            ChannelVoiceMsg::ProgramChange { program } => (0xC0 | channel_num, program, 0),
            ChannelVoiceMsg::ChannelPressure { pressure } => (0xD0 | channel_num, pressure, 0),
            ChannelVoiceMsg::PitchBend { bend } => {
                // Clamp to valid 14-bit range (0..=16383)
                let bend = (bend as u16).min(16383);
                let lsb = (bend & 0x7F) as u8;
                let msb = ((bend >> 7) & 0x7F) as u8;
                (0xE0 | channel_num, lsb, msb)
            }
            _ => return None,
        };

        Some(api::MidiEvent {
            event_type: api::EventType::Midi,
            byte_size: mem::size_of::<api::MidiEvent>() as i32,
            delta_frames: event.frame_offset as i32,
            flags: api::MidiEventFlags::REALTIME_EVENT.bits(),
            note_length: 0,
            note_offset: 0,
            midi_data: [status, data1, data2],
            _midi_reserved: 0,
            detune: 0,
            note_off_velocity: 0,
            _reserved1: 0,
            _reserved2: 0,
        })
    }

    pub fn process_simple(&mut self, buffer: &mut AudioBuffer) {
        self.process(buffer, &[], None);
    }

    pub fn set_sample_rate(&mut self, rate: f64) {
        self.sample_rate = rate;

        #[cfg(feature = "vst2")]
        {
            self.instance.set_sample_rate(rate as f32);
        }
    }

    pub fn has_editor(&self) -> bool {
        self.metadata.has_editor
    }

    #[cfg(feature = "vst2")]
    pub fn open_editor(&mut self, parent: *mut std::ffi::c_void) -> Result<(u32, u32)> {
        let editor = &mut self
            .editor
            .as_mut()
            .ok_or_else(|| BridgeError::EditorError("Plugin has no editor".into()))?
            .0;

        let opened = editor.open(parent);
        if !opened {
            return Err(BridgeError::EditorError(
                "VST2 editor.open() returned false".into(),
            ));
        }
        let size = editor.size();
        Ok((size.0 as u32, size.1 as u32))
    }

    #[cfg(not(feature = "vst2"))]
    pub fn open_editor(&mut self, _parent: *mut std::ffi::c_void) -> Result<(u32, u32)> {
        Err(BridgeError::EditorError(
            "VST2 support not compiled".into(),
        ))
    }

    #[cfg(feature = "vst2")]
    pub fn close_editor(&mut self) {
        if let Some(editor) = self.editor.as_mut() {
            editor.0.close();
        }
    }

    #[cfg(not(feature = "vst2"))]
    pub fn close_editor(&mut self) {}

    #[cfg(feature = "vst2")]
    pub fn editor_idle(&mut self) {
        if let Some(editor) = self.editor.as_mut() {
            editor.0.idle();
        }
    }

    #[cfg(not(feature = "vst2"))]
    pub fn editor_idle(&mut self) {}

    #[cfg(feature = "vst2")]
    pub fn get_parameter_count(&self) -> i32 {
        self.instance.get_info().parameters
    }

    #[cfg(not(feature = "vst2"))]
    pub fn get_parameter_count(&self) -> i32 {
        0
    }

    #[cfg(feature = "vst2")]
    pub fn get_parameter(&mut self, index: i32) -> f32 {
        let params = &self.params;
        params.get_parameter(index)
    }

    #[cfg(not(feature = "vst2"))]
    pub fn get_parameter(&mut self, _index: i32) -> f32 {
        0.0
    }

    #[cfg(feature = "vst2")]
    pub fn set_parameter(&mut self, index: i32, value: f32) {
        let params = &self.params;
        params.set_parameter(index, value);
    }

    #[cfg(not(feature = "vst2"))]
    pub fn set_parameter(&mut self, _index: i32, _value: f32) {}

    #[cfg(feature = "vst2")]
    pub fn get_parameter_name(&mut self, index: i32) -> String {
        let params = &self.params;
        params.get_parameter_name(index)
    }

    #[cfg(not(feature = "vst2"))]
    pub fn get_parameter_name(&mut self, _index: i32) -> String {
        String::new()
    }

    #[cfg(feature = "vst2")]
    pub fn get_parameter_text(&mut self, index: i32) -> String {
        let params = &self.params;
        params.get_parameter_text(index)
    }

    #[cfg(not(feature = "vst2"))]
    pub fn get_parameter_text(&mut self, _index: i32) -> String {
        String::new()
    }

    #[cfg(feature = "vst2")]
    pub fn get_parameter_list(&mut self) -> Vec<tutti_plugin::protocol::ParameterInfo> {
        let count = self.instance.get_info().parameters;
        let params = &self.params;

        (0..count)
            .map(|i| {
                let name = params.get_parameter_name(i);
                let value = params.get_parameter(i);

                tutti_plugin::protocol::ParameterInfo {
                    id: i as u32,
                    name,
                    unit: String::new(),
                    min_value: 0.0,
                    max_value: 1.0,
                    default_value: value as f64,
                    step_count: 0,
                    flags: tutti_plugin::protocol::ParameterFlags {
                        automatable: true,
                        read_only: false,
                        wrap: false,
                        is_bypass: false,
                        hidden: false,
                    },
                }
            })
            .collect()
    }

    #[cfg(not(feature = "vst2"))]
    pub fn get_parameter_list(&mut self) -> Vec<tutti_plugin::protocol::ParameterInfo> {
        Vec::new()
    }

    #[cfg(feature = "vst2")]
    pub fn get_parameter_info(&mut self, param_id: u32) -> Option<tutti_plugin::protocol::ParameterInfo> {
        let count = self.instance.get_info().parameters;
        let index = param_id as i32;

        if index < 0 || index >= count {
            return None;
        }

        let params = &self.params;
        let name = params.get_parameter_name(index);
        let value = params.get_parameter(index);

        Some(tutti_plugin::protocol::ParameterInfo {
            id: param_id,
            name,
            unit: String::new(),
            min_value: 0.0,
            max_value: 1.0,
            default_value: value as f64,
            step_count: 0,
            flags: tutti_plugin::protocol::ParameterFlags {
                automatable: true,
                read_only: false,
                wrap: false,
                is_bypass: false,
                hidden: false,
            },
        })
    }

    #[cfg(not(feature = "vst2"))]
    pub fn get_parameter_info(&mut self, _param_id: u32) -> Option<tutti_plugin::protocol::ParameterInfo> {
        None
    }

    /// State format header bytes:
    /// - `b"CHK\0"` = chunk-based state (plugin's own binary format)
    /// - `b"PRM\0"` = parameter-based state (host-serialized f32 values)
    const STATE_HEADER_CHUNK: [u8; 4] = *b"CHK\0";
    const STATE_HEADER_PARAMS: [u8; 4] = *b"PRM\0";

    #[cfg(feature = "vst2")]
    pub fn get_state(&mut self) -> Result<Vec<u8>> {
        let info = self.instance.get_info();
        let params = &self.params;

        // Prefer chunk mechanism if plugin supports it
        if info.preset_chunks {
            let chunk = params.get_preset_data();
            if !chunk.is_empty() {
                let mut state = Vec::with_capacity(4 + chunk.len());
                state.extend_from_slice(&Self::STATE_HEADER_CHUNK);
                state.extend_from_slice(&chunk);
                return Ok(state);
            }
        }

        // Fallback: serialize all parameters
        let param_count = info.parameters;
        let mut state = Vec::with_capacity(4 + 4 + (param_count as usize) * 4);
        state.extend_from_slice(&Self::STATE_HEADER_PARAMS);
        state.extend_from_slice(&param_count.to_le_bytes());

        for i in 0..param_count {
            let value = params.get_parameter(i);
            state.extend_from_slice(&value.to_le_bytes());
        }

        Ok(state)
    }

    #[cfg(not(feature = "vst2"))]
    pub fn get_state(&mut self) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }

    #[cfg(feature = "vst2")]
    pub fn set_state(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < 4 {
            return Err(BridgeError::StateRestoreError(
                "State data too short (missing header)".into(),
            ));
        }

        let header: [u8; 4] = [data[0], data[1], data[2], data[3]];
        let payload = &data[4..];

        if header == Self::STATE_HEADER_CHUNK {
            // Chunk-based restore
            if payload.is_empty() {
                return Err(BridgeError::StateRestoreError(
                    "Empty chunk data".into(),
                ));
            }
            let params = &self.params;
            params.load_preset_data(payload);
            Ok(())
        } else if header == Self::STATE_HEADER_PARAMS {
            // Parameter-based restore
            if payload.len() < 4 {
                return Err(BridgeError::StateRestoreError(
                    "Invalid parameter state (missing count)".into(),
                ));
            }

            let param_count = i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
            if param_count < 0 {
                return Err(BridgeError::StateRestoreError(
                    format!("Invalid parameter count: {}", param_count),
                ));
            }

            let expected_payload = 4 + (param_count as usize) * 4;
            if payload.len() != expected_payload {
                return Err(BridgeError::StateRestoreError(format!(
                    "Parameter state size mismatch: expected {} bytes, got {}",
                    expected_payload,
                    payload.len()
                )));
            }

            // Validate param_count against actual plugin parameter count
            let actual_count = self.instance.get_info().parameters;
            if param_count > actual_count {
                return Err(BridgeError::StateRestoreError(format!(
                    "State has {} parameters but plugin only has {}",
                    param_count, actual_count
                )));
            }

            let params = &self.params;
            for i in 0..param_count {
                let offset = 4 + (i as usize * 4);
                let value = f32::from_le_bytes([
                    payload[offset],
                    payload[offset + 1],
                    payload[offset + 2],
                    payload[offset + 3],
                ]);
                // Clamp to valid range
                let value = value.clamp(0.0, 1.0);
                params.set_parameter(i, value);
            }

            Ok(())
        } else {
            Err(BridgeError::StateRestoreError(format!(
                "Unknown state header: {:?}",
                header
            )))
        }
    }

    #[cfg(not(feature = "vst2"))]
    pub fn set_state(&mut self, _data: &[u8]) -> Result<()> {
        Ok(())
    }

    #[cfg(feature = "vst2")]
    pub fn poll_parameter_changes(&self) -> Vec<ParameterChange> {
        self.param_rx.try_iter().collect()
    }

    #[cfg(not(feature = "vst2"))]
    pub fn poll_parameter_changes(&self) -> Vec<ParameterChange> {
        Vec::new()
    }
}

// VST2 TimeInfo flag constants (from VST2.4 SDK)
#[cfg(feature = "vst2")]
mod time_info_flags {
    pub const TRANSPORT_CHANGED: i32 = 1 << 0;
    pub const TRANSPORT_PLAYING: i32 = 1 << 1;
    pub const TRANSPORT_CYCLE_ACTIVE: i32 = 1 << 2;
    pub const TRANSPORT_RECORDING: i32 = 1 << 6;
    pub const TEMPO_VALID: i32 = 1 << 9;
    pub const TIME_SIG_VALID: i32 = 1 << 10;
    pub const PPQ_POS_VALID: i32 = 1 << 11;
    pub const BARS_VALID: i32 = 1 << 13;
}

#[cfg(feature = "vst2")]
fn build_vst2_time_info(
    transport: &tutti_plugin::protocol::TransportInfo,
    sample_rate: f64,
) -> vst::api::TimeInfo {
    use time_info_flags::*;

    let mut flags = TRANSPORT_CHANGED | TEMPO_VALID | TIME_SIG_VALID | PPQ_POS_VALID | BARS_VALID;

    if transport.playing {
        flags |= TRANSPORT_PLAYING;
    }
    if transport.recording {
        flags |= TRANSPORT_RECORDING;
    }
    if transport.cycle_active {
        flags |= TRANSPORT_CYCLE_ACTIVE;
    }

    vst::api::TimeInfo {
        sample_rate,
        sample_pos: transport.position_samples as f64,
        ppq_pos: transport.position_quarters,
        tempo: transport.tempo,
        bar_start_pos: transport.bar_position_quarters,
        cycle_start_pos: transport.cycle_start_quarters,
        cycle_end_pos: transport.cycle_end_quarters,
        time_sig_numerator: transport.time_sig_numerator,
        time_sig_denominator: transport.time_sig_denominator,
        flags,
        ..Default::default()
    }
}

#[cfg(feature = "vst2")]
struct BridgeHost {
    param_tx: crossbeam_channel::Sender<ParameterChange>,
    time_info: Arc<arc_swap::ArcSwap<Option<vst::api::TimeInfo>>>,
}

#[cfg(feature = "vst2")]
impl BridgeHost {
    fn new(
        param_tx: crossbeam_channel::Sender<ParameterChange>,
        time_info: Arc<arc_swap::ArcSwap<Option<vst::api::TimeInfo>>>,
    ) -> Self {
        Self {
            param_tx,
            time_info,
        }
    }
}

#[cfg(feature = "vst2")]
impl Host for BridgeHost {
    fn automate(&self, index: i32, value: f32) {
        let _ = self.param_tx.try_send((index, value));
    }

    fn get_plugin_id(&self) -> i32 {
        // "DAWI" in ASCII (0x44=D, 0x41=A, 0x57=W, 0x49=I)
        0x44415749
    }

    fn idle(&self) {}

    fn get_time_info(&self, _mask: i32) -> Option<vst::api::TimeInfo> {
        **self.time_info.load()
    }
}

#[cfg(feature = "vst2")]
impl Drop for Vst2Instance {
    fn drop(&mut self) {
        // Close editor before unloading
        if let Some(editor) = self.editor.as_mut() {
            editor.0.close();
        }
        self.editor = None;
        // Suspend processing before the plugin is unloaded
        self.instance.suspend();
    }
}

impl crate::instance::PluginInstance for Vst2Instance {
    fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    fn supports_f64(&self) -> bool {
        self.metadata.supports_f64
    }

    fn process_f32<'a>(
        &mut self,
        buffer: &'a mut AudioBuffer<'a>,
        ctx: &crate::instance::ProcessContext,
    ) -> crate::instance::ProcessOutput {
        let midi_out = Vst2Instance::process(self, buffer, ctx.midi_events, ctx.transport);
        crate::instance::ProcessOutput {
            midi_events: midi_out,
            param_changes: tutti_plugin::protocol::ParameterChanges::new(),
            note_expression: tutti_plugin::protocol::NoteExpressionChanges::new(),
        }
    }

    fn process_f64<'a>(
        &mut self,
        buffer: &'a mut tutti_plugin::protocol::AudioBuffer64<'a>,
        ctx: &crate::instance::ProcessContext,
    ) -> crate::instance::ProcessOutput {
        let midi_out = Vst2Instance::process_f64(self, buffer, ctx.midi_events, ctx.transport);
        crate::instance::ProcessOutput {
            midi_events: midi_out,
            param_changes: tutti_plugin::protocol::ParameterChanges::new(),
            note_expression: tutti_plugin::protocol::NoteExpressionChanges::new(),
        }
    }

    fn set_sample_rate(&mut self, rate: f64) {
        Vst2Instance::set_sample_rate(self, rate);
    }

    fn get_parameter_count(&self) -> usize {
        Vst2Instance::get_parameter_count(self) as usize
    }

    fn get_parameter(&self, id: u32) -> f64 {
        #[cfg(feature = "vst2")]
        {
            self.params.get_parameter(id as i32) as f64
        }
        #[cfg(not(feature = "vst2"))]
        {
            let _ = id;
            0.0
        }
    }

    fn set_parameter(&mut self, id: u32, value: f64) {
        Vst2Instance::set_parameter(self, id as i32, value as f32);
    }

    fn get_parameter_list(&mut self) -> Vec<tutti_plugin::protocol::ParameterInfo> {
        Vst2Instance::get_parameter_list(self)
    }

    fn get_parameter_info(&mut self, id: u32) -> Option<tutti_plugin::protocol::ParameterInfo> {
        Vst2Instance::get_parameter_info(self, id)
    }

    fn has_editor(&mut self) -> bool {
        Vst2Instance::has_editor(self)
    }

    unsafe fn open_editor(&mut self, parent: *mut std::ffi::c_void) -> Result<(u32, u32)> {
        Vst2Instance::open_editor(self, parent)
    }

    fn close_editor(&mut self) {
        Vst2Instance::close_editor(self);
    }

    fn editor_idle(&mut self) {
        Vst2Instance::editor_idle(self);
    }

    fn stop_processing(&mut self) {
        #[cfg(feature = "vst2")]
        self.instance.suspend();
    }

    fn get_state(&mut self) -> Result<Vec<u8>> {
        Vst2Instance::get_state(self)
    }

    fn set_state(&mut self, data: &[u8]) -> Result<()> {
        Vst2Instance::set_state(self, data)
    }
}

#[cfg(test)]
#[cfg(feature = "vst2")]
mod tests {
    use super::*;
    use crate::instance::PluginInstance;
    use std::path::Path;
    use tutti_midi_io::{Channel, ChannelVoiceMsg};
    use tutti_plugin::protocol::{AudioBuffer, MidiEvent};

    const VST2_PLUGIN: &str = "/Library/Audio/Plug-Ins/VST/TAL-NoiseMaker.vst";

    #[test]
    fn test_vst2_load() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let instance = Vst2Instance::load(path, 44100.0, 512);
        assert!(
            instance.is_ok(),
            "Failed to load VST2 plugin: {:?}",
            instance.err()
        );

        let instance = instance.unwrap();
        let meta = instance.metadata();
        assert!(!meta.name.is_empty(), "Plugin name should not be empty");
        assert!(!meta.id.is_empty(), "Plugin id should not be empty");
    }

    #[test]
    fn test_vst2_metadata() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let instance = Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");
        let meta = instance.metadata();

        assert!(
            meta.audio_io.outputs > 0,
            "Expected audio outputs > 0, got {}",
            meta.audio_io.outputs
        );
    }

    #[test]
    fn test_vst2_parameter_count() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let instance = Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

        let count = instance.get_parameter_count();
        assert!(
            count > 0,
            "TAL-NoiseMaker should have parameters, got {}",
            count
        );
    }

    #[test]
    fn test_vst2_parameter_list() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let mut instance =
            Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

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
    fn test_vst2_get_parameter() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        #[allow(unused_mut)]
        let mut instance =
            Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

        let value = instance.get_parameter(0);
        assert!(
            value.is_finite(),
            "Parameter value should be finite, got {}",
            value
        );
    }

    #[test]
    fn test_vst2_process_f32_silence() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let mut instance =
            Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

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
    fn test_vst2_process_f32_with_note() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let mut instance =
            Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

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

    /// Helper: load plugin and process one block, returning output data.
    fn load_and_process_block(
        instance: &mut Vst2Instance,
        num_samples: usize,
        midi: &[MidiEvent],
    ) -> Vec<Vec<f32>> {
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

        let ctx = crate::instance::ProcessContext::new().midi(midi);
        let _output = PluginInstance::process_f32(instance, &mut buffer, &ctx);
        output_data
    }

    #[test]
    fn test_vst2_parameter_set_get_roundtrip() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let mut instance =
            Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

        let count = instance.get_parameter_count();
        assert!(count > 0);

        // Set parameter 0 to 0.5, read it back
        instance.set_parameter(0, 0.5);
        let value = instance.get_parameter(0);
        assert!(
            (value - 0.5).abs() < 0.01,
            "Expected ~0.5 after set, got {}",
            value
        );

        // Set to 0.0
        instance.set_parameter(0, 0.0);
        let value = instance.get_parameter(0);
        assert!(
            value.abs() < 0.01,
            "Expected ~0.0 after set, got {}",
            value
        );

        // Set to 1.0
        instance.set_parameter(0, 1.0);
        let value = instance.get_parameter(0);
        assert!(
            (value - 1.0).abs() < 0.01,
            "Expected ~1.0 after set, got {}",
            value
        );
    }

    #[test]
    fn test_vst2_parameter_info() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let mut instance =
            Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

        // Valid parameter
        let info = instance.get_parameter_info(0);
        assert!(info.is_some(), "Parameter 0 should exist");
        let info = info.unwrap();
        assert_eq!(info.id, 0);
        assert!(!info.name.is_empty());

        // Out of bounds
        let info = instance.get_parameter_info(99999);
        assert!(info.is_none(), "Out-of-bounds param should be None");
    }

    #[test]
    fn test_vst2_state_save_restore() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let mut instance =
            Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

        // Set some parameters to known values
        instance.set_parameter(0, 0.25);
        instance.set_parameter(1, 0.75);

        // Save state
        let state = instance.get_state().expect("get_state should succeed");
        assert!(!state.is_empty(), "State should not be empty");

        // Change parameters
        instance.set_parameter(0, 0.9);
        instance.set_parameter(1, 0.1);

        // Restore state
        instance
            .set_state(&state)
            .expect("set_state should succeed");

        // Verify restored values
        let v0 = instance.get_parameter(0);
        let v1 = instance.get_parameter(1);
        assert!(
            (v0 - 0.25).abs() < 0.02,
            "Param 0 should be ~0.25 after restore, got {}",
            v0
        );
        assert!(
            (v1 - 0.75).abs() < 0.02,
            "Param 1 should be ~0.75 after restore, got {}",
            v1
        );
    }

    #[test]
    fn test_vst2_state_restore_invalid_data() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let mut instance =
            Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

        // Too short
        assert!(instance.set_state(&[]).is_err());
        assert!(instance.set_state(&[0, 1, 2]).is_err());

        // Invalid header
        assert!(instance.set_state(&[0xFF, 0xFF, 0xFF, 0xFF]).is_err());

        // Valid PRM header but wrong payload size
        let mut bad_state = Vec::new();
        bad_state.extend_from_slice(b"PRM\0");
        bad_state.extend_from_slice(&2i32.to_le_bytes()); // says 2 params
        bad_state.extend_from_slice(&0.5f32.to_le_bytes()); // only 1 param value
        assert!(instance.set_state(&bad_state).is_err());
    }

    #[test]
    fn test_vst2_midi_conversion_note_on_off() {
        // NoteOn
        let event = MidiEvent {
            frame_offset: 10,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::NoteOn {
                note: 60,
                velocity: 127,
            },
        };
        let api = Vst2Instance::midi_to_api_event(&event).expect("NoteOn should convert");
        assert_eq!(api.midi_data[0], 0x90); // NoteOn, channel 0
        assert_eq!(api.midi_data[1], 60);
        assert_eq!(api.midi_data[2], 127);
        assert_eq!(api.delta_frames, 10);

        // NoteOff
        let event = MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch10,
            msg: ChannelVoiceMsg::NoteOff {
                note: 48,
                velocity: 64,
            },
        };
        let api = Vst2Instance::midi_to_api_event(&event).expect("NoteOff should convert");
        assert_eq!(api.midi_data[0], 0x80 | 9); // NoteOff, channel 9
        assert_eq!(api.midi_data[1], 48);
        assert_eq!(api.midi_data[2], 64);
    }

    #[test]
    fn test_vst2_midi_conversion_cc() {
        let event = MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::ControlChange {
                control: ControlChange::CC {
                    control: 74,
                    value: 100,
                },
            },
        };
        let api = Vst2Instance::midi_to_api_event(&event).expect("CC should convert");
        assert_eq!(api.midi_data[0], 0xB0);
        assert_eq!(api.midi_data[1], 74);
        assert_eq!(api.midi_data[2], 100);
    }

    #[test]
    fn test_vst2_midi_conversion_pitch_bend() {
        // Center (8192)
        let event = MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::PitchBend { bend: 8192 },
        };
        let api = Vst2Instance::midi_to_api_event(&event).expect("PitchBend should convert");
        assert_eq!(api.midi_data[0], 0xE0);
        assert_eq!(api.midi_data[1], 0x00); // LSB
        assert_eq!(api.midi_data[2], 0x40); // MSB

        // Min (0)
        let event = MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::PitchBend { bend: 0 },
        };
        let api = Vst2Instance::midi_to_api_event(&event).expect("PitchBend min should convert");
        assert_eq!(api.midi_data[1], 0x00);
        assert_eq!(api.midi_data[2], 0x00);

        // Max (16383)
        let event = MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::PitchBend { bend: 16383 },
        };
        let api = Vst2Instance::midi_to_api_event(&event).expect("PitchBend max should convert");
        assert_eq!(api.midi_data[1], 0x7F);
        assert_eq!(api.midi_data[2], 0x7F);
    }

    #[test]
    fn test_vst2_midi_conversion_program_change() {
        let event = MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::ProgramChange { program: 42 },
        };
        let api = Vst2Instance::midi_to_api_event(&event).expect("ProgramChange should convert");
        assert_eq!(api.midi_data[0], 0xC0);
        assert_eq!(api.midi_data[1], 42);
        assert_eq!(api.midi_data[2], 0);
    }

    #[test]
    fn test_vst2_midi_conversion_channel_pressure() {
        let event = MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::ChannelPressure { pressure: 100 },
        };
        let api =
            Vst2Instance::midi_to_api_event(&event).expect("ChannelPressure should convert");
        assert_eq!(api.midi_data[0], 0xD0);
        assert_eq!(api.midi_data[1], 100);
        assert_eq!(api.midi_data[2], 0);
    }

    #[test]
    fn test_vst2_midi_conversion_poly_pressure() {
        let event = MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::PolyPressure {
                note: 60,
                pressure: 80,
            },
        };
        let api = Vst2Instance::midi_to_api_event(&event).expect("PolyPressure should convert");
        assert_eq!(api.midi_data[0], 0xA0);
        assert_eq!(api.midi_data[1], 60);
        assert_eq!(api.midi_data[2], 80);
    }

    #[test]
    fn test_vst2_process_with_transport() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let mut instance =
            Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

        let transport = tutti_plugin::protocol::TransportInfo {
            playing: true,
            recording: false,
            cycle_active: false,
            position_samples: 44100,
            position_quarters: 2.0,
            tempo: 120.0,
            bar_position_quarters: 0.0,
            cycle_start_quarters: 0.0,
            cycle_end_quarters: 4.0,
            time_sig_numerator: 4,
            time_sig_denominator: 4,
        };

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

        // Should not panic
        instance.process(&mut buffer, &[], Some(&transport));
    }

    #[test]
    fn test_vst2_process_empty_buffer() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let mut instance =
            Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

        let input_slices: Vec<&[f32]> = vec![];
        let mut output_slices: Vec<&mut [f32]> = vec![];

        let mut buffer = AudioBuffer {
            inputs: &input_slices,
            outputs: &mut output_slices,
            num_samples: 0,
            sample_rate: 44100.0,
        };

        // Should handle gracefully (num_samples == 0 early return)
        instance.process(&mut buffer, &[], None);
    }

    #[test]
    fn test_vst2_note_on_off_lifecycle() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let mut instance =
            Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

        let num_samples = 512;

        // Send NoteOn
        let note_on = [MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::NoteOn {
                note: 60,
                velocity: 100,
            },
        }];
        load_and_process_block(&mut instance, num_samples, &note_on);

        // Process a few blocks to let sound develop
        for _ in 0..4 {
            load_and_process_block(&mut instance, num_samples, &[]);
        }

        // Send NoteOff
        let note_off = [MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::NoteOff {
                note: 60,
                velocity: 0,
            },
        }];
        load_and_process_block(&mut instance, num_samples, &note_off);

        // Process more blocks — sound should fade out
        let mut total_energy = 0.0f64;
        for _ in 0..20 {
            let output = load_and_process_block(&mut instance, num_samples, &[]);
            for ch in &output {
                for &s in ch {
                    total_energy += (s as f64) * (s as f64);
                }
            }
        }
        // After NoteOff + 20 blocks, energy should be very low (release tail)
        let rms = (total_energy / (20.0 * num_samples as f64 * 2.0)).sqrt();
        assert!(
            rms < 0.5,
            "Expected low energy after NoteOff, RMS = {}",
            rms
        );
    }

    #[test]
    fn test_vst2_sample_rate_change() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let mut instance =
            Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

        // Change sample rate — should not panic
        instance.set_sample_rate(48000.0);

        // Process a block at new sample rate
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
            sample_rate: 48000.0,
        };

        instance.process(&mut buffer, &[], None);
    }

    #[test]
    fn test_vst2_has_editor() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let instance =
            Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

        // TAL-NoiseMaker has an editor
        assert!(
            instance.has_editor(),
            "TAL-NoiseMaker should have an editor"
        );
    }

    #[test]
    fn test_vst2_stop_processing() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let mut instance =
            Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

        // stop_processing (suspend) should not panic
        PluginInstance::stop_processing(&mut instance);
    }

    #[test]
    fn test_vst2_many_midi_events() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let mut instance =
            Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

        // Send more than 2 events (tests the flexible array allocation in send_midi_events)
        let events: Vec<MidiEvent> = (0..10)
            .map(|i| MidiEvent {
                frame_offset: i,
                channel: Channel::Ch1,
                msg: ChannelVoiceMsg::NoteOn {
                    note: 60 + i as u8,
                    velocity: 100,
                },
            })
            .collect();

        load_and_process_block(&mut instance, 512, &events);

        // Send NoteOffs
        let note_offs: Vec<MidiEvent> = (0..10)
            .map(|i| MidiEvent {
                frame_offset: 0,
                channel: Channel::Ch1,
                msg: ChannelVoiceMsg::NoteOff {
                    note: 60 + i as u8,
                    velocity: 0,
                },
            })
            .collect();

        load_and_process_block(&mut instance, 512, &note_offs);
    }

    #[test]
    fn test_vst2_parameter_name() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let mut instance =
            Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

        let name = instance.get_parameter_name(0);
        assert!(
            !name.is_empty(),
            "Parameter 0 should have a non-empty name"
        );

        let text = instance.get_parameter_text(0);
        // Text may or may not be empty depending on plugin
        assert!(text.len() < 256, "Parameter text should be reasonable length");
    }

    #[test]
    fn test_vst2_trait_get_parameter() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(VST2_PLUGIN);
        let instance =
            Vst2Instance::load(path, 44100.0, 512).expect("Failed to load VST2 plugin");

        // Test the PluginInstance trait's get_parameter (was returning 0.0 before fix)
        let value = PluginInstance::get_parameter(&instance, 0);
        assert!(
            value.is_finite(),
            "Trait get_parameter should return finite value"
        );
    }

    #[test]
    fn test_vst2_load_nonexistent_path() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let result = Vst2Instance::load(Path::new("/nonexistent/plugin.vst"), 44100.0, 512);
        assert!(result.is_err(), "Loading nonexistent path should fail");
    }
}
