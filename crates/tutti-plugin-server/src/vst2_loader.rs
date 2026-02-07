//! VST2 plugin loader.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tutti_midi_io::{ChannelVoiceMsg, ControlChange};
use tutti_plugin::protocol::{AudioBuffer, MidiEvent};
use tutti_plugin::{BridgeError, LoadStage, PluginMetadata, Result};

#[cfg(feature = "vst2")]
use vst::host::{Host, PluginLoader};
#[cfg(feature = "vst2")]
use vst::plugin::Plugin as VstPlugin;

pub type ParameterChange = (i32, f32);

pub struct Vst2Instance {
    #[cfg(feature = "vst2")]
    instance: vst::host::PluginInstance,
    #[cfg(feature = "vst2")]
    #[allow(dead_code)]
    host: Arc<Mutex<BridgeHost>>,
    #[cfg(feature = "vst2")]
    time_info: Arc<arc_swap::ArcSwap<Option<vst::api::TimeInfo>>>,
    metadata: PluginMetadata,
    sample_rate: f64,
    #[cfg(feature = "vst2")]
    param_rx: crossbeam_channel::Receiver<ParameterChange>,
}

impl Vst2Instance {
    pub fn load(path: &Path, sample_rate: f64, block_size: usize) -> Result<Self> {
        #[cfg(feature = "vst2")]
        {
            let (param_tx, param_rx) = crossbeam_channel::unbounded();
            let time_info = Arc::new(arc_swap::ArcSwap::from_pointee(None));
            let host = Arc::new(Mutex::new(BridgeHost::new(
                param_tx,
                Arc::clone(&time_info),
            )));

            let mut loader = PluginLoader::load(path, Arc::clone(&host)).map_err(|e| {
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

            let info = instance.get_info();
            let metadata =
                PluginMetadata::new(format!("vst2.{}", info.unique_id), info.name.clone())
                    .author(info.vendor.clone())
                    .version(format!("{}", info.version))
                    .audio_io(info.inputs as usize, info.outputs as usize)
                    .midi(info.midi_inputs > 0 || info.midi_outputs > 0)
                    .f64_support(info.f64_precision);

            Ok(Self {
                instance,
                host,
                time_info,
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

        let api_events: Vec<api::MidiEvent> = midi_events
            .iter()
            .filter_map(Self::midi_to_api_event)
            .collect();

        if api_events.is_empty() {
            return;
        }

        let num_events = api_events.len() as i32;
        let boxed_events: Vec<Box<api::MidiEvent>> = api_events.into_iter().map(Box::new).collect();
        let event_ptrs: Vec<*mut api::Event> = boxed_events
            .iter()
            .map(|e| e.as_ref() as *const api::MidiEvent as *mut api::Event)
            .collect();

        if num_events <= 2 {
            let mut events = api::Events {
                num_events,
                _reserved: 0,
                events: [std::ptr::null_mut(); 2],
            };
            for (i, ptr) in event_ptrs.iter().enumerate().take(2) {
                events.events[i] = *ptr;
            }
            self.instance.process_events(&events);
        } else {
            #[repr(C)]
            struct LargeEvents {
                num_events: i32,
                _reserved: isize,
                events: Vec<*mut api::Event>,
            }

            let large_events = LargeEvents {
                num_events,
                _reserved: 0,
                events: event_ptrs,
            };

            let events_ptr = &large_events as *const LargeEvents as *const api::Events;
            unsafe {
                self.instance.process_events(&*events_ptr);
            }
        }

        drop(boxed_events);
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

    #[cfg(feature = "vst2")]
    pub fn has_editor(&mut self) -> bool {
        self.instance.get_editor().is_some()
    }

    #[cfg(not(feature = "vst2"))]
    pub fn has_editor(&mut self) -> bool {
        false
    }

    #[cfg(feature = "vst2")]
    pub fn open_editor(&mut self, parent: *mut std::ffi::c_void) -> Result<(u32, u32)> {
        let mut editor = self
            .instance
            .get_editor()
            .ok_or_else(|| BridgeError::LoadFailed {
                path: PathBuf::from("unknown"),
                stage: LoadStage::Initialization,
                reason: "Plugin has no editor".to_string(),
            })?;

        editor.open(parent);
        let size = editor.size();
        Ok((size.0 as u32, size.1 as u32))
    }

    #[cfg(not(feature = "vst2"))]
    pub fn open_editor(&mut self, _parent: *mut std::ffi::c_void) -> Result<(u32, u32)> {
        Err(BridgeError::LoadFailed {
            path: PathBuf::from("unknown"),
            stage: LoadStage::Opening,
            reason: "VST2 support not compiled".to_string(),
        })
    }

    #[cfg(feature = "vst2")]
    pub fn close_editor(&mut self) {
        if let Some(mut editor) = self.instance.get_editor() {
            editor.close();
        }
    }

    #[cfg(not(feature = "vst2"))]
    pub fn close_editor(&mut self) {}

    #[cfg(feature = "vst2")]
    pub fn editor_idle(&mut self) {
        if let Some(mut editor) = self.instance.get_editor() {
            editor.idle();
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
        let params = self.instance.get_parameter_object();
        params.get_parameter(index)
    }

    #[cfg(not(feature = "vst2"))]
    pub fn get_parameter(&mut self, _index: i32) -> f32 {
        0.0
    }

    #[cfg(feature = "vst2")]
    pub fn set_parameter(&mut self, index: i32, value: f32) {
        let params = self.instance.get_parameter_object();
        params.set_parameter(index, value);
    }

    #[cfg(not(feature = "vst2"))]
    pub fn set_parameter(&mut self, _index: i32, _value: f32) {}

    #[cfg(feature = "vst2")]
    pub fn get_parameter_name(&mut self, index: i32) -> String {
        let params = self.instance.get_parameter_object();
        params.get_parameter_name(index)
    }

    #[cfg(not(feature = "vst2"))]
    pub fn get_parameter_name(&mut self, _index: i32) -> String {
        String::new()
    }

    #[cfg(feature = "vst2")]
    pub fn get_parameter_text(&mut self, index: i32) -> String {
        let params = self.instance.get_parameter_object();
        params.get_parameter_text(index)
    }

    #[cfg(not(feature = "vst2"))]
    pub fn get_parameter_text(&mut self, _index: i32) -> String {
        String::new()
    }

    #[cfg(feature = "vst2")]
    pub fn get_parameter_list(&mut self) -> Vec<tutti_plugin::protocol::ParameterInfo> {
        let count = self.instance.get_info().parameters;
        let params = self.instance.get_parameter_object();

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

        let params = self.instance.get_parameter_object();
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

    #[cfg(feature = "vst2")]
    pub fn get_state(&mut self) -> Result<Vec<u8>> {
        let param_count = self.get_parameter_count();
        let mut state = Vec::with_capacity((param_count as usize + 1) * 4);
        state.extend_from_slice(&param_count.to_le_bytes());

        for i in 0..param_count {
            let value = self.get_parameter(i);
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
            return Err(BridgeError::LoadFailed {
                path: PathBuf::from("unknown"),
                stage: LoadStage::Initialization,
                reason: "Invalid state data".to_string(),
            });
        }

        let param_count = i32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let expected_size = (param_count as usize + 1) * 4;

        if data.len() != expected_size {
            return Err(BridgeError::LoadFailed {
                path: PathBuf::from("unknown"),
                stage: LoadStage::Initialization,
                reason: format!(
                    "State size mismatch: expected {}, got {}",
                    expected_size,
                    data.len()
                ),
            });
        }

        for i in 0..param_count {
            let offset = 4 + (i as usize * 4);
            let value = f32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            self.set_parameter(i, value);
        }

        Ok(())
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

#[cfg(feature = "vst2")]
fn build_vst2_time_info(
    transport: &tutti_plugin::protocol::TransportInfo,
    sample_rate: f64,
) -> vst::api::TimeInfo {
    let mut flags = 0i32;

    if transport.playing {
        flags |= 1 << 1; // kVstTransportPlaying
    }
    if transport.recording {
        flags |= 1 << 6; // kVstTransportRecording
    }
    if transport.cycle_active {
        flags |= 1 << 2; // kVstTransportCycleActive
    }

    flags |= 1 << 0; // kVstTransportChanged
    flags |= 1 << 9; // kVstTempoValid
    flags |= 1 << 10; // kVstTimeSigValid
    flags |= 1 << 11; // kVstPpqPosValid
    flags |= 1 << 13; // kVstBarsValid

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
        0x44415749
    }

    fn idle(&self) {}

    fn get_time_info(&self, _mask: i32) -> Option<vst::api::TimeInfo> {
        **self.time_info.load()
    }
}

#[cfg(feature = "vst2")]
impl Drop for Vst2Instance {
    fn drop(&mut self) {}
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

    fn get_parameter(&self, _id: u32) -> f64 {
        0.0
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

    fn get_state(&mut self) -> Result<Vec<u8>> {
        Vst2Instance::get_state(self)
    }

    fn set_state(&mut self, data: &[u8]) -> Result<()> {
        Vst2Instance::set_state(self, data)
    }
}
