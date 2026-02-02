//! VST2 plugin loader
//!
//! This module handles loading and interfacing with VST2 plugins.

use crate::error::LoadStage;
use std::path::PathBuf;

#[cfg(feature = "vst2")]
use vst::plugin::Plugin as VstPlugin;

#[cfg(feature = "vst2")]
use vst::host::{Host, PluginLoader};

#[cfg(feature = "vst2")]
use vst::event::{Event as VstEvent, MidiEvent as VstMidiEvent};

// Import MIDI types for event conversion
use tutti_midi_io::{ChannelVoiceMsg, ControlChange};

use crate::error::{BridgeError, Result};
use crate::protocol::{AudioBuffer, MidiEvent, PluginMetadata};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Parameter change notification (index, value)
pub type ParameterChange = (i32, f32);

/// VST2 plugin instance wrapper
pub struct Vst2Instance {
    #[cfg(feature = "vst2")]
    instance: vst::host::PluginInstance,

    /// Kept alive for the `vst` crate's Host trait dispatch (automate, get_time_info, etc.)
    #[cfg(feature = "vst2")]
    #[allow(dead_code)]
    host: Arc<Mutex<BridgeHost>>,

    /// Shared lock-free time info — updated directly from process_with_transport,
    /// read lock-free by the plugin via Host::get_time_info.
    #[cfg(feature = "vst2")]
    time_info: Arc<arc_swap::ArcSwap<Option<vst::api::TimeInfo>>>,

    metadata: PluginMetadata,
    sample_rate: f32,

    /// Channel for receiving parameter changes from plugin automation
    #[cfg(feature = "vst2")]
    param_rx: crossbeam_channel::Receiver<ParameterChange>,
}

impl Vst2Instance {
    /// Load a VST2 plugin from path
    pub fn load(path: &Path, sample_rate: f32) -> Result<Self> {
        #[cfg(feature = "vst2")]
        {
            // Create channel for parameter automation
            let (param_tx, param_rx) = crossbeam_channel::unbounded();

            // Shared lock-free time info (updated by process_with_transport, read by plugin)
            let time_info = Arc::new(arc_swap::ArcSwap::from_pointee(None));

            // Create host with parameter change sender and shared time info
            let host = Arc::new(Mutex::new(BridgeHost::new(
                param_tx,
                Arc::clone(&time_info),
            )));

            // Load plugin (clone host since it will be moved into the loader)
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

            // Initialize plugin
            instance.init();
            instance.set_sample_rate(sample_rate);

            // Get plugin info
            let info = instance.get_info();

            // Build metadata (use id as unique identifier)
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
            let _ = (path, sample_rate);
            Err(BridgeError::LoadFailed {
                path: path.to_path_buf(),
                stage: LoadStage::Opening,
                reason: "VST2 support not compiled (enable 'vst2' feature)".to_string(),
            })
        }
    }

    /// Get plugin metadata
    pub fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    /// Check if this plugin supports 64-bit (f64) audio processing
    pub fn supports_f64(&self) -> bool {
        self.metadata.supports_f64
    }

    /// Process audio with MIDI events through the plugin
    pub fn process_with_midi(
        &mut self,
        buffer: &mut AudioBuffer,
        midi_events: &[MidiEvent],
    ) -> crate::protocol::MidiEventVec {
        #[cfg(feature = "vst2")]
        {
            use vst::api;
            use vst::buffer::AudioBuffer as VstBuffer;

            let num_samples = buffer.num_samples;
            if num_samples == 0 {
                return smallvec::SmallVec::new();
            }

            // Convert MIDI events to VST API format and send to plugin
            if !midi_events.is_empty() {
                let api_events: Vec<api::MidiEvent> = midi_events
                    .iter()
                    .filter_map(Self::midi_to_api_event)
                    .collect();

                if !api_events.is_empty() {
                    // Create api::Events structure with pointers to our events
                    // For more than 2 events, we need to allocate a larger array
                    let num_events = api_events.len() as i32;

                    // Box the events to get stable pointers
                    let boxed_events: Vec<Box<api::MidiEvent>> =
                        api_events.into_iter().map(Box::new).collect();

                    // Create event pointers (cast MidiEvent* to Event*)
                    let event_ptrs: Vec<*mut api::Event> = boxed_events
                        .iter()
                        .map(|e| e.as_ref() as *const api::MidiEvent as *mut api::Event)
                        .collect();

                    // Create Events struct
                    // We need to handle the variable-length array correctly
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
                        // For more than 2 events, we need custom layout
                        // Allocate properly sized Events structure
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

                        // Cast to api::Events (they have compatible layout for the fixed part)
                        let events_ptr = &large_events as *const LargeEvents as *const api::Events;
                        unsafe {
                            self.instance.process_events(&*events_ptr);
                        }
                    }

                    // Keep boxed_events alive until after process_events
                    drop(boxed_events);
                }
            }

            // Convert Tutti's borrowed slices to owned vectors for VST processing
            let mut input_vecs: Vec<Vec<f32>> =
                buffer.inputs.iter().map(|slice| slice.to_vec()).collect();

            let mut output_vecs: Vec<Vec<f32>> = buffer
                .outputs
                .iter()
                .map(|_| vec![0.0; num_samples])
                .collect();

            // Ensure we have at least the channels the plugin expects
            let info = self.instance.get_info();
            while input_vecs.len() < info.inputs as usize {
                input_vecs.push(vec![0.0; num_samples]);
            }
            while output_vecs.len() < info.outputs as usize {
                output_vecs.push(vec![0.0; num_samples]);
            }

            // Create raw pointer arrays for VST
            let input_ptrs: Vec<*const f32> = input_vecs.iter().map(|v| v.as_ptr()).collect();

            let mut output_ptrs: Vec<*mut f32> =
                output_vecs.iter_mut().map(|v| v.as_mut_ptr()).collect();

            // Create VST AudioBuffer from raw pointers
            let mut vst_buffer = unsafe {
                VstBuffer::from_raw(
                    input_ptrs.len(),
                    output_ptrs.len(),
                    input_ptrs.as_ptr(),
                    output_ptrs.as_mut_ptr(),
                    num_samples,
                )
            };

            // Process through VST
            self.instance.process(&mut vst_buffer);

            // Copy processed output back to Tutti buffer
            for (i, out_channel) in buffer.outputs.iter_mut().enumerate() {
                if i < output_vecs.len() {
                    out_channel.copy_from_slice(&output_vecs[i][..num_samples]);
                }
            }

            // VST2 doesn't have explicit MIDI output - return empty
            // (Some VST2 synths output MIDI via deprecated methods we don't support)
            smallvec::SmallVec::new()
        }

        #[cfg(not(feature = "vst2"))]
        {
            let _ = (buffer, midi_events);
            smallvec::SmallVec::new()
        }
    }

    /// Convert Tutti MidiEvent to VST API MidiEvent
    #[cfg(feature = "vst2")]
    fn midi_to_api_event(event: &MidiEvent) -> Option<vst::api::MidiEvent> {
        use std::mem;
        use vst::api;

        // Access midi_msg types through the imported MidiEvent
        // We need to match on the msg field which contains ChannelVoiceMsg
        let channel_num = event.channel as u8;

        // Convert to MIDI bytes
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

    /// Process audio with MIDI events and transport info
    pub fn process_with_transport(
        &mut self,
        buffer: &mut AudioBuffer,
        midi_events: &[MidiEvent],
        transport: &crate::protocol::TransportInfo,
    ) -> crate::protocol::MidiEventVec {
        #[cfg(feature = "vst2")]
        {
            // Update transport info lock-free via ArcSwap (no host Mutex needed)
            self.time_info.store(Arc::new(Some(build_vst2_time_info(
                transport,
                buffer.sample_rate,
            ))));

            // Then process with MIDI as usual
            self.process_with_midi(buffer, midi_events)
        }

        #[cfg(not(feature = "vst2"))]
        {
            let _ = (buffer, midi_events, transport);
            smallvec::SmallVec::new()
        }
    }

    /// Process audio with MIDI events using f64 buffers
    ///
    /// Uses `processDoubleReplacing` for plugins that report `f64_precision` support.
    /// Falls back to f32 processing with conversion if `process_f64` is not available.
    pub fn process_with_midi_f64(
        &mut self,
        buffer: &mut crate::protocol::AudioBuffer64,
        midi_events: &[MidiEvent],
    ) -> crate::protocol::MidiEventVec {
        #[cfg(feature = "vst2")]
        {
            use vst::api;
            use vst::buffer::AudioBuffer as VstBuffer;

            let num_samples = buffer.num_samples;
            if num_samples == 0 {
                return smallvec::SmallVec::new();
            }

            // Send MIDI events (same as f32 path)
            if !midi_events.is_empty() {
                let api_events: Vec<api::MidiEvent> = midi_events
                    .iter()
                    .filter_map(Self::midi_to_api_event)
                    .collect();

                if !api_events.is_empty() {
                    let num_events = api_events.len() as i32;
                    let boxed_events: Vec<Box<api::MidiEvent>> =
                        api_events.into_iter().map(Box::new).collect();
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
            }

            // VST2's process_f64 is not widely exposed in the `vst` crate,
            // so we convert f64 → f32, process, then convert f32 → f64.
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

            // Convert f32 output back to f64
            for (i, out_channel) in buffer.outputs.iter_mut().enumerate() {
                if i < output_vecs.len() {
                    for (j, sample) in out_channel.iter_mut().enumerate().take(num_samples) {
                        *sample = output_vecs[i][j] as f64;
                    }
                }
            }

            smallvec::SmallVec::new()
        }

        #[cfg(not(feature = "vst2"))]
        {
            let _ = (buffer, midi_events);
            smallvec::SmallVec::new()
        }
    }

    /// Process audio with MIDI events, transport info, and f64 buffers
    pub fn process_with_transport_f64(
        &mut self,
        buffer: &mut crate::protocol::AudioBuffer64,
        midi_events: &[MidiEvent],
        transport: &crate::protocol::TransportInfo,
    ) -> crate::protocol::MidiEventVec {
        #[cfg(feature = "vst2")]
        {
            // Update transport info lock-free via ArcSwap (no host Mutex needed)
            self.time_info.store(Arc::new(Some(build_vst2_time_info(
                transport,
                buffer.sample_rate,
            ))));

            self.process_with_midi_f64(buffer, midi_events)
        }

        #[cfg(not(feature = "vst2"))]
        {
            let _ = (buffer, midi_events, transport);
            smallvec::SmallVec::new()
        }
    }

    /// Convert MidiEvent to VST MidiEvent (deprecated - kept for reference)
    #[cfg(feature = "vst2")]
    #[allow(dead_code)]
    fn midi_to_vst_event(event: &MidiEvent) -> Option<VstEvent<'_>> {
        // Convert frame_offset to delta_frames (same thing in VST)
        let delta_frames = event.frame_offset as i32;
        let channel_num = event.channel as u8;

        // Convert to raw MIDI bytes
        let (status, data1, data2) = match event.msg {
            ChannelVoiceMsg::NoteOn { note, velocity } => {
                let status = 0x90 | channel_num;
                (status, note, velocity)
            }
            ChannelVoiceMsg::NoteOff { note, velocity } => {
                let status = 0x80 | channel_num;
                (status, note, velocity)
            }
            ChannelVoiceMsg::PolyPressure { note, pressure } => {
                let status = 0xA0 | channel_num;
                (status, note, pressure)
            }
            ChannelVoiceMsg::ControlChange { control } => {
                let status = 0xB0 | channel_num;
                match control {
                    ControlChange::CC { control, value } => (status, control, value),
                    ControlChange::CCHighRes {
                        control1, value, ..
                    } => {
                        // Send MSB only for high-res CC
                        (status, control1, (value >> 7) as u8)
                    }
                    _ => return None, // Skip other CC types
                }
            }
            ChannelVoiceMsg::ProgramChange { program } => {
                let status = 0xC0 | channel_num;
                (status, program, 0)
            }
            ChannelVoiceMsg::ChannelPressure { pressure } => {
                let status = 0xD0 | channel_num;
                (status, pressure, 0)
            }
            ChannelVoiceMsg::PitchBend { bend } => {
                let status = 0xE0 | channel_num;
                let lsb = (bend & 0x7F) as u8;
                let msb = ((bend >> 7) & 0x7F) as u8;
                (status, lsb, msb)
            }
            _ => return None, // Skip unsupported message types
        };

        Some(VstEvent::Midi(VstMidiEvent {
            data: [status, data1, data2],
            delta_frames,
            live: true,
            note_length: None,
            note_offset: None,
            detune: 0,
            note_off_velocity: 0,
        }))
    }

    /// Process audio through the plugin (legacy method without MIDI)
    pub fn process(&mut self, buffer: &mut AudioBuffer) {
        self.process_with_midi(buffer, &[]);
    }

    /// Legacy process method - now calls process_with_midi
    #[allow(dead_code)]
    fn process_legacy(&mut self, buffer: &mut AudioBuffer) {
        #[cfg(feature = "vst2")]
        {
            use vst::buffer::AudioBuffer as VstBuffer;

            let num_samples = buffer.num_samples;
            if num_samples == 0 {
                return;
            }

            // Convert Tutti's borrowed slices to owned vectors for VST processing
            // VST crate requires Vec<Vec<f32>> ownership model
            let mut input_vecs: Vec<Vec<f32>> =
                buffer.inputs.iter().map(|slice| slice.to_vec()).collect();

            let mut output_vecs: Vec<Vec<f32>> = buffer
                .outputs
                .iter()
                .map(|_| vec![0.0; num_samples])
                .collect();

            // Ensure we have at least the channels the plugin expects
            let info = self.instance.get_info();
            while input_vecs.len() < info.inputs as usize {
                input_vecs.push(vec![0.0; num_samples]);
            }
            while output_vecs.len() < info.outputs as usize {
                output_vecs.push(vec![0.0; num_samples]);
            }

            // Create raw pointer arrays for VST
            let input_ptrs: Vec<*const f32> = input_vecs.iter().map(|v| v.as_ptr()).collect();

            let mut output_ptrs: Vec<*mut f32> =
                output_vecs.iter_mut().map(|v| v.as_mut_ptr()).collect();

            // Create VST AudioBuffer from raw pointers
            // Safety: We own the data and pointers are valid for the duration of processing
            let mut vst_buffer = unsafe {
                VstBuffer::from_raw(
                    input_ptrs.len(),
                    output_ptrs.len(),
                    input_ptrs.as_ptr(),
                    output_ptrs.as_mut_ptr(),
                    num_samples,
                )
            };

            // Process through VST
            self.instance.process(&mut vst_buffer);

            // Copy processed output back to Tutti buffer
            for (i, out_channel) in buffer.outputs.iter_mut().enumerate() {
                if i < output_vecs.len() {
                    out_channel.copy_from_slice(&output_vecs[i][..num_samples]);
                }
            }
        }

        #[cfg(not(feature = "vst2"))]
        {
            let _ = buffer;
        }
    }

    /// Set sample rate
    pub fn set_sample_rate(&mut self, rate: f32) {
        self.sample_rate = rate;

        #[cfg(feature = "vst2")]
        {
            self.instance.set_sample_rate(rate);
        }
    }

    /// Check if plugin has editor
    #[cfg(feature = "vst2")]
    pub fn has_editor(&mut self) -> bool {
        // Try to get editor - if it returns Some, the plugin has an editor
        self.instance.get_editor().is_some()
    }

    #[cfg(not(feature = "vst2"))]
    pub fn has_editor(&mut self) -> bool {
        false
    }

    /// Open plugin editor
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

        // Get editor size
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

    /// Close plugin editor
    #[cfg(feature = "vst2")]
    pub fn close_editor(&mut self) {
        if let Some(mut editor) = self.instance.get_editor() {
            editor.close();
        }
    }

    #[cfg(not(feature = "vst2"))]
    pub fn close_editor(&mut self) {
        // No-op
    }

    /// Editor idle (call periodically to update GUI)
    #[cfg(feature = "vst2")]
    pub fn editor_idle(&mut self) {
        if let Some(mut editor) = self.instance.get_editor() {
            editor.idle();
        }
    }

    #[cfg(not(feature = "vst2"))]
    pub fn editor_idle(&mut self) {
        // No-op
    }

    /// Get parameter count
    #[cfg(feature = "vst2")]
    pub fn get_parameter_count(&self) -> i32 {
        self.instance.get_info().parameters
    }

    #[cfg(not(feature = "vst2"))]
    pub fn get_parameter_count(&self) -> i32 {
        0
    }

    /// Get parameter value (normalized 0-1)
    #[cfg(feature = "vst2")]
    pub fn get_parameter(&mut self, index: i32) -> f32 {
        let params = self.instance.get_parameter_object();
        params.get_parameter(index)
    }

    #[cfg(not(feature = "vst2"))]
    pub fn get_parameter(&mut self, _index: i32) -> f32 {
        0.0
    }

    /// Set parameter value (normalized 0-1)
    #[cfg(feature = "vst2")]
    pub fn set_parameter(&mut self, index: i32, value: f32) {
        let params = self.instance.get_parameter_object();
        params.set_parameter(index, value);
    }

    #[cfg(not(feature = "vst2"))]
    pub fn set_parameter(&mut self, _index: i32, _value: f32) {
        // No-op
    }

    /// Get parameter name
    #[cfg(feature = "vst2")]
    pub fn get_parameter_name(&mut self, index: i32) -> String {
        let params = self.instance.get_parameter_object();
        params.get_parameter_name(index)
    }

    #[cfg(not(feature = "vst2"))]
    pub fn get_parameter_name(&mut self, _index: i32) -> String {
        String::new()
    }

    /// Get parameter text (formatted value)
    #[cfg(feature = "vst2")]
    pub fn get_parameter_text(&mut self, index: i32) -> String {
        let params = self.instance.get_parameter_object();
        params.get_parameter_text(index)
    }

    #[cfg(not(feature = "vst2"))]
    pub fn get_parameter_text(&mut self, _index: i32) -> String {
        String::new()
    }

    /// Save plugin state to byte array
    #[cfg(feature = "vst2")]
    pub fn get_state(&mut self) -> Result<Vec<u8>> {
        // VST2 uses effGetChunk for state serialization
        // For now, we save all parameter values as a simple binary format
        let param_count = self.get_parameter_count();
        let mut state = Vec::with_capacity((param_count as usize + 1) * 4);

        // Write parameter count
        state.extend_from_slice(&param_count.to_le_bytes());

        // Write all parameter values (normalized 0-1)
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

    /// Load plugin state from byte array
    #[cfg(feature = "vst2")]
    pub fn set_state(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < 4 {
            return Err(BridgeError::LoadFailed {
                path: PathBuf::from("unknown"),
                stage: LoadStage::Initialization,
                reason: "Invalid state data".to_string(),
            });
        }

        // Read parameter count
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

        // Read and set all parameter values
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

    /// Poll for parameter changes from plugin automation
    /// Returns all pending parameter changes
    #[cfg(feature = "vst2")]
    pub fn poll_parameter_changes(&self) -> Vec<ParameterChange> {
        self.param_rx.try_iter().collect()
    }

    #[cfg(not(feature = "vst2"))]
    pub fn poll_parameter_changes(&self) -> Vec<ParameterChange> {
        Vec::new()
    }
}

/// Build VST2 TimeInfo from transport state.
#[cfg(feature = "vst2")]
fn build_vst2_time_info(
    transport: &crate::protocol::TransportInfo,
    sample_rate: f32,
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

    // Always valid fields
    flags |= 1 << 0; // kVstTransportChanged
    flags |= 1 << 9; // kVstTempoValid
    flags |= 1 << 10; // kVstTimeSigValid
    flags |= 1 << 11; // kVstPpqPosValid
    flags |= 1 << 13; // kVstBarsValid

    vst::api::TimeInfo {
        sample_rate: sample_rate as f64,
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

/// Host implementation for VST2 plugins
#[cfg(feature = "vst2")]
struct BridgeHost {
    /// Channel for sending parameter changes to main thread
    param_tx: crossbeam_channel::Sender<ParameterChange>,
    /// Lock-free transport/timing information (shared with Vst2Instance)
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
        // Send parameter change to main thread via lock-free channel
        let _ = self.param_tx.try_send((index, value));
    }

    fn get_plugin_id(&self) -> i32 {
        // Return a unique host ID
        0x44415749 // "DAWI" in ASCII
    }

    fn idle(&self) {
        // Called by plugin during processing
    }

    fn get_time_info(&self, _mask: i32) -> Option<vst::api::TimeInfo> {
        // Lock-free read of transport info
        **self.time_info.load()
    }
}

#[cfg(feature = "vst2")]
impl Drop for Vst2Instance {
    fn drop(&mut self) {
        // Plugin will be dropped automatically
    }
}
