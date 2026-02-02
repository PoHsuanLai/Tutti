//! CLAP plugin loader
//!
//! This module handles loading and interfacing with CLAP plugins.

use crate::error::LoadStage;
use std::path::PathBuf;

#[cfg(feature = "clap")]
use clap_sys::entry::clap_plugin_entry;
#[cfg(feature = "clap")]
use clap_sys::events::{
    clap_event_header, clap_event_midi, clap_event_note, clap_event_note_expression,
    clap_event_param_value, clap_event_transport, clap_input_events, clap_output_events,
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_MIDI, CLAP_EVENT_NOTE_EXPRESSION, CLAP_EVENT_NOTE_OFF,
    CLAP_EVENT_NOTE_ON, CLAP_EVENT_PARAM_VALUE, CLAP_NOTE_EXPRESSION_BRIGHTNESS,
    CLAP_NOTE_EXPRESSION_PAN, CLAP_NOTE_EXPRESSION_TUNING, CLAP_NOTE_EXPRESSION_VIBRATO,
    CLAP_NOTE_EXPRESSION_VOLUME, CLAP_TRANSPORT_HAS_BEATS_TIMELINE, CLAP_TRANSPORT_HAS_TEMPO,
    CLAP_TRANSPORT_HAS_TIME_SIGNATURE, CLAP_TRANSPORT_IS_LOOP_ACTIVE, CLAP_TRANSPORT_IS_PLAYING,
    CLAP_TRANSPORT_IS_RECORDING,
};
#[cfg(all(feature = "clap", target_os = "macos"))]
use clap_sys::ext::gui::CLAP_WINDOW_API_COCOA;
#[cfg(all(feature = "clap", target_os = "windows"))]
use clap_sys::ext::gui::CLAP_WINDOW_API_WIN32;
#[cfg(all(feature = "clap", target_os = "linux"))]
use clap_sys::ext::gui::CLAP_WINDOW_API_X11;
#[cfg(feature = "clap")]
use clap_sys::ext::gui::{clap_plugin_gui, clap_window, clap_window_handle, CLAP_EXT_GUI};
#[cfg(feature = "clap")]
use clap_sys::ext::params::{clap_plugin_params, CLAP_EXT_PARAMS};
#[cfg(feature = "clap")]
use clap_sys::ext::state::{clap_plugin_state, CLAP_EXT_STATE};
#[cfg(feature = "clap")]
use clap_sys::fixedpoint::CLAP_BEATTIME_FACTOR;
#[cfg(feature = "clap")]
use clap_sys::host::clap_host;
#[cfg(feature = "clap")]
use clap_sys::plugin::clap_plugin;
#[cfg(feature = "clap")]
use clap_sys::process::{clap_process, CLAP_PROCESS_ERROR};
#[cfg(feature = "clap")]
use clap_sys::stream::{clap_istream, clap_ostream};
#[cfg(feature = "clap")]
use clap_sys::version::CLAP_VERSION;

use crate::error::{BridgeError, Result};
use crate::protocol::{AudioBuffer, MidiEvent, PluginMetadata};

// Import MIDI types for event conversion
use std::ffi::{CStr, CString};
use std::path::Path;
use std::ptr;
use tutti_midi_io::{ChannelVoiceMsg, ControlChange};

/// CLAP plugin instance wrapper
pub struct ClapInstance {
    #[cfg(feature = "clap")]
    plugin: *const clap_plugin,

    #[cfg(feature = "clap")]
    _library: libloading::Library,

    metadata: PluginMetadata,
    sample_rate: f32,
    max_frames: u32,
    is_processing: bool,

    /// CLAP was designed with f64 support from the start
    supports_f64: bool,
}

/// CLAP event wrapper
///
/// These fields match the CLAP event ABI and are accessed by plugins via FFI.
/// The compiler sees them as unused because they're accessed through unsafe pointers.
#[cfg(feature = "clap")]
#[allow(dead_code)]
enum ClapEvent {
    NoteOn {
        header: clap_event_header,
        note_id: i32,
        port_index: i16,
        channel: u16,
        key: i16,
        velocity: f64,
    },
    NoteOff {
        header: clap_event_header,
        note_id: i32,
        port_index: i16,
        channel: u16,
        key: i16,
        velocity: f64,
    },
    Midi {
        header: clap_event_header,
        port_index: u16,
        data: [u8; 3],
    },
    NoteExpression {
        header: clap_event_header,
        expression_id: i32,
        note_id: i32,
        port_index: i16,
        channel: i16,
        key: i16,
        value: f64,
    },
    ParamValue {
        header: clap_event_header,
        param_id: u32,
        cookie: *mut std::ffi::c_void,
        note_id: i32,
        port_index: i16,
        channel: i16,
        key: i16,
        value: f64,
    },
}

// Safety: ClapEvent is only accessed on the audio thread and plugin pointers are thread-local
#[cfg(feature = "clap")]
unsafe impl Send for ClapEvent {}
#[cfg(feature = "clap")]
unsafe impl Sync for ClapEvent {}

/// CLAP input event list implementation
#[cfg(feature = "clap")]
struct ClapInputEventList {
    list: clap_input_events,
    events: Vec<ClapEvent>,
}

#[cfg(feature = "clap")]
impl ClapInputEventList {
    fn new(events: Vec<ClapEvent>) -> Self {
        let list = clap_input_events {
            ctx: ptr::null_mut(),
            size: Some(input_events_size),
            get: Some(input_events_get),
        };

        Self { list, events }
    }
}

#[cfg(feature = "clap")]
unsafe extern "C" fn input_events_size(list: *const clap_input_events) -> u32 {
    let event_list = &*(list as *const ClapInputEventList);
    event_list.events.len() as u32
}

#[cfg(feature = "clap")]
unsafe extern "C" fn input_events_get(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    let event_list = &*(list as *const ClapInputEventList);
    if index >= event_list.events.len() as u32 {
        return ptr::null();
    }

    match &event_list.events[index as usize] {
        ClapEvent::NoteOn { header, .. } => header as *const _,
        ClapEvent::NoteOff { header, .. } => header as *const _,
        ClapEvent::Midi { header, .. } => header as *const _,
        ClapEvent::NoteExpression { header, .. } => header as *const _,
        ClapEvent::ParamValue { header, .. } => header as *const _,
    }
}

/// CLAP output event list implementation
#[cfg(feature = "clap")]
struct ClapOutputEventList {
    list: clap_output_events,
    events: Vec<ClapEvent>,
}

#[cfg(feature = "clap")]
impl ClapOutputEventList {
    fn new() -> Self {
        let list = clap_output_events {
            ctx: ptr::null_mut(),
            try_push: Some(output_events_try_push),
        };

        Self {
            list,
            events: Vec::new(),
        }
    }

    /// Extract MIDI output events
    fn to_midi_events(&self) -> crate::protocol::MidiEventVec {
        self.events
            .iter()
            .filter_map(ClapInstance::clap_to_midi_event)
            .collect()
    }

    /// Extract note expression output events
    fn to_note_expression_changes(&self) -> crate::protocol::NoteExpressionChanges {
        let mut changes = crate::protocol::NoteExpressionChanges::new();
        for event in &self.events {
            if let ClapEvent::NoteExpression {
                header,
                expression_id,
                note_id,
                value,
                ..
            } = event
            {
                let expression_type = match *expression_id {
                    id if id == CLAP_NOTE_EXPRESSION_VOLUME => {
                        crate::protocol::NoteExpressionType::Volume
                    }
                    id if id == CLAP_NOTE_EXPRESSION_PAN => {
                        crate::protocol::NoteExpressionType::Pan
                    }
                    id if id == CLAP_NOTE_EXPRESSION_TUNING => {
                        crate::protocol::NoteExpressionType::Tuning
                    }
                    id if id == CLAP_NOTE_EXPRESSION_VIBRATO => {
                        crate::protocol::NoteExpressionType::Vibrato
                    }
                    id if id == CLAP_NOTE_EXPRESSION_BRIGHTNESS => {
                        crate::protocol::NoteExpressionType::Brightness
                    }
                    _ => continue,
                };
                changes.add_change(crate::protocol::NoteExpressionValue {
                    sample_offset: header.time as i32,
                    note_id: *note_id,
                    expression_type,
                    value: *value,
                });
            }
        }
        changes
    }

    /// Extract parameter value output events
    fn to_param_changes(&self) -> crate::protocol::ParameterChanges {
        let mut changes = crate::protocol::ParameterChanges::new();
        // Group by param_id
        let mut queues: std::collections::HashMap<u32, crate::protocol::ParameterQueue> =
            std::collections::HashMap::new();
        for event in &self.events {
            if let ClapEvent::ParamValue {
                header,
                param_id,
                value,
                ..
            } = event
            {
                queues
                    .entry(*param_id)
                    .or_insert_with(|| crate::protocol::ParameterQueue::new(*param_id))
                    .add_point(header.time as i32, *value);
            }
        }
        for (_, queue) in queues {
            changes.add_queue(queue);
        }
        changes
    }
}

#[cfg(feature = "clap")]
unsafe extern "C" fn output_events_try_push(
    list: *const clap_output_events,
    event: *const clap_event_header,
) -> bool {
    if event.is_null() || list.is_null() {
        return false;
    }

    let output_list = &mut *(list as *mut ClapOutputEventList);
    let header = &*event;

    match header.type_ {
        CLAP_EVENT_NOTE_ON => {
            let e = &*(event as *const clap_event_note);
            output_list.events.push(ClapEvent::NoteOn {
                header: *header,
                note_id: e.note_id,
                port_index: e.port_index,
                channel: e.channel as u16,
                key: e.key,
                velocity: e.velocity,
            });
            true
        }
        CLAP_EVENT_NOTE_OFF => {
            let e = &*(event as *const clap_event_note);
            output_list.events.push(ClapEvent::NoteOff {
                header: *header,
                note_id: e.note_id,
                port_index: e.port_index,
                channel: e.channel as u16,
                key: e.key,
                velocity: e.velocity,
            });
            true
        }
        CLAP_EVENT_MIDI => {
            let e = &*(event as *const clap_event_midi);
            output_list.events.push(ClapEvent::Midi {
                header: *header,
                port_index: e.port_index,
                data: e.data,
            });
            true
        }
        CLAP_EVENT_NOTE_EXPRESSION => {
            let e = &*(event as *const clap_event_note_expression);
            output_list.events.push(ClapEvent::NoteExpression {
                header: *header,
                expression_id: e.expression_id,
                note_id: e.note_id,
                port_index: e.port_index,
                channel: e.channel,
                key: e.key,
                value: e.value,
            });
            true
        }
        CLAP_EVENT_PARAM_VALUE => {
            let e = &*(event as *const clap_event_param_value);
            output_list.events.push(ClapEvent::ParamValue {
                header: *header,
                param_id: e.param_id,
                cookie: e.cookie,
                note_id: e.note_id,
                port_index: e.port_index,
                channel: e.channel,
                key: e.key,
                value: e.value,
            });
            true
        }
        _ => {
            // Unknown event type, skip
            false
        }
    }
}

impl ClapInstance {
    /// Load a CLAP plugin from path
    pub fn load(path: &Path, sample_rate: f32) -> Result<Self> {
        #[cfg(feature = "clap")]
        {
            // Load library
            let library = unsafe {
                libloading::Library::new(path).map_err(|e| BridgeError::LoadFailed {
                    path: path.to_path_buf(),
                    stage: LoadStage::Opening,
                    reason: format!("Failed to load library: {}", e),
                })?
            };

            // Get entry point
            let entry: libloading::Symbol<unsafe extern "C" fn() -> *const clap_plugin_entry> = unsafe {
                library.get(b"clap_entry\0").map_err(|e| BridgeError::LoadFailed {
                    path: path.to_path_buf(),
                    stage: LoadStage::Opening,
                    reason: format!("No clap_entry symbol: {}", e),
                })?
            };

            let entry_ptr = unsafe { entry() };
            if entry_ptr.is_null() {
                return Err(BridgeError::LoadFailed {
                    path: path.to_path_buf(),
                    stage: LoadStage::Opening,
                    reason: "clap_entry returned null".to_string(),
                });
            }

            let entry_struct = unsafe { &*entry_ptr };

            // Check version compatibility
            let init_fn = entry_struct.init.ok_or_else(|| BridgeError::LoadFailed {
                path: path.to_path_buf(),
                stage: LoadStage::Opening,
                reason: "No init function".to_string(),
            })?;
            let path_str = path.to_str().ok_or_else(|| BridgeError::LoadFailed {
                path: path.to_path_buf(),
                stage: LoadStage::Opening,
                reason: "Path contains invalid UTF-8".to_string(),
            })?;
            if !unsafe { init_fn(path_str.as_ptr() as *const i8) } {
                return Err(BridgeError::LoadFailed {
                    path: path.to_path_buf(),
                    stage: LoadStage::Opening,
                    reason: "Entry init failed".to_string(),
                });
            }

            // Create host
            let host = Box::into_raw(Box::new(create_clap_host()));

            // Get plugin factory
            let get_factory_fn = entry_struct.get_factory.ok_or_else(|| BridgeError::LoadFailed {
                path: path.to_path_buf(),
                stage: LoadStage::Factory,
                reason: "No get_factory function".to_string(),
            })?;
            let factory_ptr = unsafe {
                get_factory_fn(clap_sys::factory::plugin_factory::CLAP_PLUGIN_FACTORY_ID.as_ptr())
            };

            if factory_ptr.is_null() {
                return Err(BridgeError::LoadFailed {
                    path: path.to_path_buf(),
                    stage: LoadStage::Factory,
                    reason: "No plugin factory".to_string(),
                });
            }

            let factory = unsafe {
                &*(factory_ptr as *const clap_sys::factory::plugin_factory::clap_plugin_factory)
            };

            // Get plugin count and use first plugin
            let factory_typed =
                factory_ptr as *const clap_sys::factory::plugin_factory::clap_plugin_factory;
            let get_count_fn = factory.get_plugin_count.ok_or_else(|| BridgeError::LoadFailed {
                path: path.to_path_buf(),
                stage: LoadStage::Factory,
                reason: "No get_plugin_count function".to_string(),
            })?;
            let plugin_count = unsafe { get_count_fn(factory_typed) };
            if plugin_count == 0 {
                return Err(BridgeError::LoadFailed {
                    path: path.to_path_buf(),
                    stage: LoadStage::Factory,
                    reason: "No plugins in factory".to_string(),
                });
            }

            // Get first plugin descriptor
            let get_desc_fn = factory.get_plugin_descriptor.ok_or_else(|| BridgeError::LoadFailed {
                path: path.to_path_buf(),
                stage: LoadStage::Factory,
                reason: "No get_plugin_descriptor function".to_string(),
            })?;
            let desc_ptr = unsafe { get_desc_fn(factory_typed, 0) };
            if desc_ptr.is_null() {
                return Err(BridgeError::LoadFailed {
                    path: path.to_path_buf(),
                    stage: LoadStage::Factory,
                    reason: "No plugin descriptor".to_string(),
                });
            }

            let descriptor = unsafe { &*desc_ptr };

            // Create plugin instance
            let plugin_id = unsafe { CStr::from_ptr(descriptor.id) }.to_string_lossy();
            let plugin_id_cstr = CString::new(plugin_id.as_ref()).map_err(|e| {
                BridgeError::LoadFailed {
                    path: path.to_path_buf(),
                    stage: LoadStage::Instantiation,
                    reason: format!("Invalid plugin ID (contains null byte): {}", e),
                }
            })?;

            let create_fn = factory.create_plugin.ok_or_else(|| BridgeError::LoadFailed {
                path: path.to_path_buf(),
                stage: LoadStage::Instantiation,
                reason: "No create_plugin function".to_string(),
            })?;
            let plugin = unsafe { create_fn(factory_typed, host, plugin_id_cstr.as_ptr()) };

            if plugin.is_null() {
                return Err(BridgeError::LoadFailed {
                    path: path.to_path_buf(),
                    stage: LoadStage::Instantiation,
                    reason: "Failed to create plugin instance".to_string(),
                });
            }

            // Initialize plugin
            let plugin_ref = unsafe { &*plugin };
            let init_fn = plugin_ref.init.ok_or_else(|| BridgeError::LoadFailed {
                path: path.to_path_buf(),
                stage: LoadStage::Initialization,
                reason: "No plugin init function".to_string(),
            })?;
            if !unsafe { init_fn(plugin) } {
                return Err(BridgeError::LoadFailed {
                    path: path.to_path_buf(),
                    stage: LoadStage::Initialization,
                    reason: "Plugin init failed".to_string(),
                });
            }

            // Build metadata
            let name = unsafe { CStr::from_ptr(descriptor.name) }.to_string_lossy();
            let vendor = unsafe { CStr::from_ptr(descriptor.vendor) }.to_string_lossy();
            let version = unsafe { CStr::from_ptr(descriptor.version) }.to_string_lossy();

            // CLAP natively supports both f32 and f64 via data32/data64 pointers
            let supports_f64 = true;

            let metadata = PluginMetadata::new(format!("clap.{}", plugin_id), name.to_string())
                .author(vendor.to_string())
                .version(version.to_string())
                .audio_io(2, 2)
                .f64_support(supports_f64);

            Ok(Self {
                plugin,
                _library: library,
                metadata,
                sample_rate,
                max_frames: 8192,
                is_processing: false,
                supports_f64,
            })
        }

        #[cfg(not(feature = "clap"))]
        {
            let _ = (path, sample_rate);
            Err(BridgeError::LoadFailed {
                path: path.to_path_buf(),
                stage: LoadStage::Opening,
                reason: "CLAP support not compiled (enable 'clap' feature)".to_string(),
            })
        }
    }

    /// Get plugin metadata
    pub fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    /// Activate plugin for processing
    #[cfg(feature = "clap")]
    fn ensure_activated(&mut self) -> Result<()> {
        if !self.is_processing {
            let plugin_ref = unsafe { &*self.plugin };

            // Activate
            let activate_fn = plugin_ref.activate.ok_or_else(|| BridgeError::LoadFailed {
                path: PathBuf::from("unknown"),
                stage: LoadStage::Activation,
                reason: "No activate function".to_string(),
            })?;
            if !unsafe { activate_fn(self.plugin, self.sample_rate as f64, 1, self.max_frames) } {
                return Err(BridgeError::LoadFailed {
                    path: PathBuf::from("unknown"),
                    stage: LoadStage::Activation,
                    reason: "Activate failed".to_string(),
                });
            }

            // Start processing
            let start_fn = plugin_ref.start_processing.ok_or_else(|| BridgeError::LoadFailed {
                path: PathBuf::from("unknown"),
                stage: LoadStage::Activation,
                reason: "No start_processing function".to_string(),
            })?;
            if !unsafe { start_fn(self.plugin) } {
                return Err(BridgeError::LoadFailed {
                    path: PathBuf::from("unknown"),
                    stage: LoadStage::Activation,
                    reason: "Start processing failed".to_string(),
                });
            }

            self.is_processing = true;
        }
        Ok(())
    }

    /// Process audio through the plugin
    pub fn process(&mut self, buffer: &mut AudioBuffer) {
        #[cfg(feature = "clap")]
        {
            if let Err(_e) = self.ensure_activated() {
                return;
            }

            let num_samples = buffer.num_samples;
            if num_samples == 0 {
                return;
            }

            // Build CLAP audio buffers
            let mut input_ptrs: Vec<*mut f32> = buffer
                .inputs
                .iter()
                .map(|slice| slice.as_ptr() as *mut f32)
                .collect();

            let mut output_ptrs: Vec<*mut f32> = buffer
                .outputs
                .iter_mut()
                .map(|slice| slice.as_mut_ptr())
                .collect();

            let mut audio_inputs = clap_sys::audio_buffer::clap_audio_buffer {
                data32: input_ptrs.as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: buffer.inputs.len() as u32,
                latency: 0,
                constant_mask: 0,
            };

            let mut audio_outputs = clap_sys::audio_buffer::clap_audio_buffer {
                data32: output_ptrs.as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: buffer.outputs.len() as u32,
                latency: 0,
                constant_mask: 0,
            };

            let process_data = clap_process {
                steady_time: 0,
                frames_count: num_samples as u32,
                transport: ptr::null(),
                audio_inputs: &mut audio_inputs,
                audio_outputs: &mut audio_outputs,
                audio_inputs_count: 1,
                audio_outputs_count: 1,
                in_events: ptr::null(),
                out_events: ptr::null(),
            };

            // Process
            let plugin_ref = unsafe { &*self.plugin };
            if let Some(process_fn) = plugin_ref.process {
                let status = unsafe { process_fn(self.plugin, &process_data) };

                if status == CLAP_PROCESS_ERROR {}
            }
        }

        #[cfg(not(feature = "clap"))]
        {
            let _ = buffer;
        }
    }

    /// Process audio with MIDI events through the plugin
    pub fn process_with_midi(
        &mut self,
        buffer: &mut AudioBuffer,
        midi_events: &[MidiEvent],
    ) -> crate::protocol::MidiEventVec {
        #[cfg(feature = "clap")]
        {
            if let Err(_e) = self.ensure_activated() {
                return smallvec::SmallVec::new();
            }

            let num_samples = buffer.num_samples;
            if num_samples == 0 {
                return smallvec::SmallVec::new();
            }

            // Build CLAP audio buffers
            let mut input_ptrs: Vec<*mut f32> = buffer
                .inputs
                .iter()
                .map(|slice| slice.as_ptr() as *mut f32)
                .collect();

            let mut output_ptrs: Vec<*mut f32> = buffer
                .outputs
                .iter_mut()
                .map(|slice| slice.as_mut_ptr())
                .collect();

            let mut audio_inputs = clap_sys::audio_buffer::clap_audio_buffer {
                data32: input_ptrs.as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: buffer.inputs.len() as u32,
                latency: 0,
                constant_mask: 0,
            };

            let mut audio_outputs = clap_sys::audio_buffer::clap_audio_buffer {
                data32: output_ptrs.as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: buffer.outputs.len() as u32,
                latency: 0,
                constant_mask: 0,
            };

            // Convert MIDI events to CLAP events
            let clap_events: Vec<ClapEvent> = midi_events
                .iter()
                .filter_map(Self::midi_to_clap_event)
                .collect();

            // Create input event list
            let input_events = ClapInputEventList::new(clap_events);

            // Create output event list (empty, for collecting plugin output)
            let mut output_events = ClapOutputEventList::new();

            let process_data = clap_process {
                steady_time: 0,
                frames_count: num_samples as u32,
                transport: ptr::null(),
                audio_inputs: &mut audio_inputs,
                audio_outputs: &mut audio_outputs,
                audio_inputs_count: 1,
                audio_outputs_count: 1,
                in_events: &input_events.list as *const _ as *const _,
                out_events: &mut output_events.list as *mut _ as *mut _,
            };

            // Process
            let plugin_ref = unsafe { &*self.plugin };
            if let Some(process_fn) = plugin_ref.process {
                let status = unsafe { process_fn(self.plugin, &process_data) };

                if status == CLAP_PROCESS_ERROR {}
            }

            // Convert output events back to MIDI (if any)
            // Most plugins don't output MIDI, so return empty for now
            smallvec::SmallVec::new()
        }

        #[cfg(not(feature = "clap"))]
        {
            let _ = (buffer, midi_events);
            smallvec::SmallVec::new()
        }
    }

    /// Convert Tutti MidiEvent to CLAP event
    #[cfg(feature = "clap")]
    fn midi_to_clap_event(event: &MidiEvent) -> Option<ClapEvent> {
        let channel = event.channel as u16;
        let time = event.frame_offset as u32;

        match event.msg {
            ChannelVoiceMsg::NoteOn { note, velocity } => Some(ClapEvent::NoteOn {
                header: clap_event_header {
                    size: std::mem::size_of::<clap_event_note>() as u32,
                    time,
                    space_id: CLAP_CORE_EVENT_SPACE_ID,
                    type_: CLAP_EVENT_NOTE_ON,
                    flags: 0,
                },
                note_id: -1,
                port_index: 0,
                channel,
                key: note as i16,
                velocity: (velocity as f64) / 127.0,
            }),
            ChannelVoiceMsg::NoteOff { note, velocity } => Some(ClapEvent::NoteOff {
                header: clap_event_header {
                    size: std::mem::size_of::<clap_event_note>() as u32,
                    time,
                    space_id: CLAP_CORE_EVENT_SPACE_ID,
                    type_: CLAP_EVENT_NOTE_OFF,
                    flags: 0,
                },
                note_id: -1,
                port_index: 0,
                channel,
                key: note as i16,
                velocity: (velocity as f64) / 127.0,
            }),
            ChannelVoiceMsg::ControlChange { .. }
            | ChannelVoiceMsg::ProgramChange { .. }
            | ChannelVoiceMsg::ChannelPressure { .. }
            | ChannelVoiceMsg::PitchBend { .. } => {
                // Convert to MIDI bytes for CLAP MIDI event
                let (status, data1, data2) = Self::encode_midi_bytes(event)?;

                Some(ClapEvent::Midi {
                    header: clap_event_header {
                        size: std::mem::size_of::<clap_event_midi>() as u32,
                        time,
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        type_: CLAP_EVENT_MIDI,
                        flags: 0,
                    },
                    port_index: 0,
                    data: [status, data1, data2],
                })
            }
            _ => None,
        }
    }

    /// Encode MIDI event to 3 bytes
    #[cfg(feature = "clap")]
    fn encode_midi_bytes(event: &MidiEvent) -> Option<(u8, u8, u8)> {
        let channel_num = event.channel as u8;

        match event.msg {
            ChannelVoiceMsg::ControlChange { control } => {
                let (cc, value) = match control {
                    ControlChange::CC { control, value } => (control, value),
                    ControlChange::CCHighRes {
                        control1, value, ..
                    } => (control1, (value >> 7) as u8),
                    _ => return None,
                };
                Some((0xB0 | channel_num, cc, value))
            }
            ChannelVoiceMsg::ProgramChange { program } => Some((0xC0 | channel_num, program, 0)),
            ChannelVoiceMsg::ChannelPressure { pressure } => {
                Some((0xD0 | channel_num, pressure, 0))
            }
            ChannelVoiceMsg::PitchBend { bend } => {
                let lsb = (bend & 0x7F) as u8;
                let msb = ((bend >> 7) & 0x7F) as u8;
                Some((0xE0 | channel_num, lsb, msb))
            }
            _ => None,
        }
    }

    /// Convert CLAP output event back to Tutti MidiEvent
    #[cfg(feature = "clap")]
    fn clap_to_midi_event(event: &ClapEvent) -> Option<MidiEvent> {
        match event {
            ClapEvent::NoteOn {
                header,
                channel,
                key,
                velocity,
                ..
            } => Some(MidiEvent {
                frame_offset: header.time as usize,
                channel: tutti_midi_io::Channel::from_u8(*channel as u8),
                msg: ChannelVoiceMsg::NoteOn {
                    note: *key as u8,
                    velocity: (*velocity * 127.0) as u8,
                },
            }),
            ClapEvent::NoteOff {
                header,
                channel,
                key,
                velocity,
                ..
            } => Some(MidiEvent {
                frame_offset: header.time as usize,
                channel: tutti_midi_io::Channel::from_u8(*channel as u8),
                msg: ChannelVoiceMsg::NoteOff {
                    note: *key as u8,
                    velocity: (*velocity * 127.0) as u8,
                },
            }),
            ClapEvent::Midi { header, data, .. } => {
                let status = data[0];
                let channel = tutti_midi_io::Channel::from_u8(status & 0x0F);
                let msg_type = status & 0xF0;

                let msg = match msg_type {
                    0xB0 => ChannelVoiceMsg::ControlChange {
                        control: ControlChange::CC {
                            control: data[1],
                            value: data[2],
                        },
                    },
                    0xC0 => ChannelVoiceMsg::ProgramChange { program: data[1] },
                    0xD0 => ChannelVoiceMsg::ChannelPressure { pressure: data[1] },
                    0xE0 => {
                        let bend = (data[1] as u16) | ((data[2] as u16) << 7);
                        ChannelVoiceMsg::PitchBend { bend }
                    }
                    0xA0 => ChannelVoiceMsg::PolyPressure {
                        note: data[1],
                        pressure: data[2],
                    },
                    _ => return None,
                };

                Some(MidiEvent {
                    frame_offset: header.time as usize,
                    channel,
                    msg,
                })
            }
            // Note expression and param value are not MIDI
            ClapEvent::NoteExpression { .. } | ClapEvent::ParamValue { .. } => None,
        }
    }

    /// Convert protocol note expression to CLAP event
    #[cfg(feature = "clap")]
    fn note_expression_to_clap_event(expr: &crate::protocol::NoteExpressionValue) -> ClapEvent {
        let expression_id = match expr.expression_type {
            crate::protocol::NoteExpressionType::Volume => CLAP_NOTE_EXPRESSION_VOLUME,
            crate::protocol::NoteExpressionType::Pan => CLAP_NOTE_EXPRESSION_PAN,
            crate::protocol::NoteExpressionType::Tuning => CLAP_NOTE_EXPRESSION_TUNING,
            crate::protocol::NoteExpressionType::Vibrato => CLAP_NOTE_EXPRESSION_VIBRATO,
            crate::protocol::NoteExpressionType::Brightness => CLAP_NOTE_EXPRESSION_BRIGHTNESS,
        };

        ClapEvent::NoteExpression {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_note_expression>() as u32,
                time: expr.sample_offset as u32,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_NOTE_EXPRESSION,
                flags: 0,
            },
            expression_id,
            note_id: expr.note_id,
            port_index: 0,
            channel: -1,
            key: -1,
            value: expr.value,
        }
    }

    /// Convert protocol parameter changes to CLAP events
    #[cfg(feature = "clap")]
    fn param_changes_to_clap_events(
        param_changes: &crate::protocol::ParameterChanges,
    ) -> Vec<ClapEvent> {
        let mut events = Vec::new();
        for queue in &param_changes.queues {
            for point in &queue.points {
                events.push(ClapEvent::ParamValue {
                    header: clap_event_header {
                        size: std::mem::size_of::<clap_event_param_value>() as u32,
                        time: point.sample_offset as u32,
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        type_: CLAP_EVENT_PARAM_VALUE,
                        flags: 0,
                    },
                    param_id: queue.param_id,
                    cookie: ptr::null_mut(),
                    note_id: -1,
                    port_index: -1,
                    channel: -1,
                    key: -1,
                    value: point.value,
                });
            }
        }
        events
    }

    /// Build CLAP transport from protocol TransportInfo
    #[cfg(feature = "clap")]
    fn build_transport(transport: &crate::protocol::TransportInfo) -> clap_event_transport {
        let mut flags: u32 = 0;
        flags |= CLAP_TRANSPORT_HAS_TEMPO;
        flags |= CLAP_TRANSPORT_HAS_BEATS_TIMELINE;
        flags |= CLAP_TRANSPORT_HAS_TIME_SIGNATURE;
        if transport.playing {
            flags |= CLAP_TRANSPORT_IS_PLAYING;
        }
        if transport.recording {
            flags |= CLAP_TRANSPORT_IS_RECORDING;
        }
        if transport.cycle_active {
            flags |= CLAP_TRANSPORT_IS_LOOP_ACTIVE;
        }

        clap_event_transport {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_transport>() as u32,
                time: 0,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: 9, // CLAP_EVENT_TRANSPORT
                flags: 0,
            },
            flags,
            song_pos_beats: (transport.position_quarters * CLAP_BEATTIME_FACTOR as f64) as i64,
            song_pos_seconds: 0, // Not computing seconds timeline
            tempo: transport.tempo,
            tempo_inc: 0.0,
            loop_start_beats: (transport.cycle_start_quarters * CLAP_BEATTIME_FACTOR as f64) as i64,
            loop_end_beats: (transport.cycle_end_quarters * CLAP_BEATTIME_FACTOR as f64) as i64,
            loop_start_seconds: 0,
            loop_end_seconds: 0,
            bar_start: (transport.bar_position_quarters * CLAP_BEATTIME_FACTOR as f64) as i64,
            bar_number: 0,
            tsig_num: transport.time_sig_numerator as u16,
            tsig_denom: transport.time_sig_denominator as u16,
        }
    }

    /// Process audio with full automation (MIDI + parameters + note expression + transport)
    pub fn process_with_automation(
        &mut self,
        buffer: &mut AudioBuffer,
        midi_events: &[MidiEvent],
        param_changes: &crate::protocol::ParameterChanges,
        note_expression: &crate::protocol::NoteExpressionChanges,
        transport: &crate::protocol::TransportInfo,
    ) -> (
        crate::protocol::MidiEventVec,
        crate::protocol::ParameterChanges,
        crate::protocol::NoteExpressionChanges,
    ) {
        #[cfg(feature = "clap")]
        {
            if let Err(_e) = self.ensure_activated() {
                return (
                    smallvec::SmallVec::new(),
                    crate::protocol::ParameterChanges::new(),
                    crate::protocol::NoteExpressionChanges::new(),
                );
            }

            let num_samples = buffer.num_samples;
            if num_samples == 0 {
                return (
                    smallvec::SmallVec::new(),
                    crate::protocol::ParameterChanges::new(),
                    crate::protocol::NoteExpressionChanges::new(),
                );
            }

            // Build CLAP audio buffers
            let mut input_ptrs: Vec<*mut f32> = buffer
                .inputs
                .iter()
                .map(|slice| slice.as_ptr() as *mut f32)
                .collect();

            let mut output_ptrs: Vec<*mut f32> = buffer
                .outputs
                .iter_mut()
                .map(|slice| slice.as_mut_ptr())
                .collect();

            let mut audio_inputs = clap_sys::audio_buffer::clap_audio_buffer {
                data32: input_ptrs.as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: buffer.inputs.len() as u32,
                latency: 0,
                constant_mask: 0,
            };

            let mut audio_outputs = clap_sys::audio_buffer::clap_audio_buffer {
                data32: output_ptrs.as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: buffer.outputs.len() as u32,
                latency: 0,
                constant_mask: 0,
            };

            // Build all input events: MIDI + note expression + param changes
            let mut clap_events: Vec<ClapEvent> = midi_events
                .iter()
                .filter_map(Self::midi_to_clap_event)
                .collect();

            // Add note expression events
            for expr in &note_expression.changes {
                clap_events.push(Self::note_expression_to_clap_event(expr));
            }

            // Add parameter automation events
            clap_events.extend(Self::param_changes_to_clap_events(param_changes));

            // Sort all events by time for proper ordering
            clap_events.sort_by_key(|event| match event {
                ClapEvent::NoteOn { header, .. }
                | ClapEvent::NoteOff { header, .. }
                | ClapEvent::Midi { header, .. }
                | ClapEvent::NoteExpression { header, .. }
                | ClapEvent::ParamValue { header, .. } => header.time,
            });

            // Create event lists
            let input_events = ClapInputEventList::new(clap_events);
            let mut output_events = ClapOutputEventList::new();

            // Build transport
            let clap_transport = Self::build_transport(transport);

            let process_data = clap_process {
                steady_time: transport.position_samples,
                frames_count: num_samples as u32,
                transport: &clap_transport,
                audio_inputs: &mut audio_inputs,
                audio_outputs: &mut audio_outputs,
                audio_inputs_count: 1,
                audio_outputs_count: 1,
                in_events: &input_events.list as *const _ as *const _,
                out_events: &mut output_events.list as *mut _ as *mut _,
            };

            // Process
            let plugin_ref = unsafe { &*self.plugin };
            if let Some(process_fn) = plugin_ref.process {
                let status = unsafe { process_fn(self.plugin, &process_data) };

                if status == CLAP_PROCESS_ERROR {}
            }

            // Collect all outputs
            let midi_output = output_events.to_midi_events();
            let param_output = output_events.to_param_changes();
            let note_expression_output = output_events.to_note_expression_changes();

            (midi_output, param_output, note_expression_output)
        }

        #[cfg(not(feature = "clap"))]
        {
            let _ = (
                buffer,
                midi_events,
                param_changes,
                note_expression,
                transport,
            );
            (
                smallvec::SmallVec::new(),
                crate::protocol::ParameterChanges::new(),
                crate::protocol::NoteExpressionChanges::new(),
            )
        }
    }

    /// Check if this plugin supports 64-bit (f64) audio processing
    pub fn supports_f64(&self) -> bool {
        self.supports_f64
    }

    /// Process audio with f64 buffers through the plugin
    pub fn process_f64(&mut self, buffer: &mut crate::protocol::AudioBuffer64) {
        #[cfg(feature = "clap")]
        {
            if let Err(_e) = self.ensure_activated() {
                return;
            }

            let num_samples = buffer.num_samples;
            if num_samples == 0 {
                return;
            }

            // Build CLAP audio buffers using data64 instead of data32
            let mut input_ptrs_f64: Vec<*mut f64> = buffer
                .inputs
                .iter()
                .map(|slice| slice.as_ptr() as *mut f64)
                .collect();

            let mut output_ptrs_f64: Vec<*mut f64> = buffer
                .outputs
                .iter_mut()
                .map(|slice| slice.as_mut_ptr())
                .collect();

            let mut audio_inputs = clap_sys::audio_buffer::clap_audio_buffer {
                data32: ptr::null_mut(),
                data64: input_ptrs_f64.as_mut_ptr(),
                channel_count: buffer.inputs.len() as u32,
                latency: 0,
                constant_mask: 0,
            };

            let mut audio_outputs = clap_sys::audio_buffer::clap_audio_buffer {
                data32: ptr::null_mut(),
                data64: output_ptrs_f64.as_mut_ptr(),
                channel_count: buffer.outputs.len() as u32,
                latency: 0,
                constant_mask: 0,
            };

            let process_data = clap_process {
                steady_time: 0,
                frames_count: num_samples as u32,
                transport: ptr::null(),
                audio_inputs: &mut audio_inputs,
                audio_outputs: &mut audio_outputs,
                audio_inputs_count: 1,
                audio_outputs_count: 1,
                in_events: ptr::null(),
                out_events: ptr::null(),
            };

            let plugin_ref = unsafe { &*self.plugin };
            if let Some(process_fn) = plugin_ref.process {
                let status = unsafe { process_fn(self.plugin, &process_data) };
                if status == CLAP_PROCESS_ERROR {}
            }
        }

        #[cfg(not(feature = "clap"))]
        {
            let _ = buffer;
        }
    }

    /// Process audio with full automation using f64 buffers
    pub fn process_with_automation_f64(
        &mut self,
        buffer: &mut crate::protocol::AudioBuffer64,
        midi_events: &[MidiEvent],
        param_changes: &crate::protocol::ParameterChanges,
        note_expression: &crate::protocol::NoteExpressionChanges,
        transport: &crate::protocol::TransportInfo,
    ) -> (
        crate::protocol::MidiEventVec,
        crate::protocol::ParameterChanges,
        crate::protocol::NoteExpressionChanges,
    ) {
        #[cfg(feature = "clap")]
        {
            if let Err(_e) = self.ensure_activated() {
                return (
                    smallvec::SmallVec::new(),
                    crate::protocol::ParameterChanges::new(),
                    crate::protocol::NoteExpressionChanges::new(),
                );
            }

            let num_samples = buffer.num_samples;
            if num_samples == 0 {
                return (
                    smallvec::SmallVec::new(),
                    crate::protocol::ParameterChanges::new(),
                    crate::protocol::NoteExpressionChanges::new(),
                );
            }

            // Build CLAP audio buffers using data64
            let mut input_ptrs_f64: Vec<*mut f64> = buffer
                .inputs
                .iter()
                .map(|slice| slice.as_ptr() as *mut f64)
                .collect();

            let mut output_ptrs_f64: Vec<*mut f64> = buffer
                .outputs
                .iter_mut()
                .map(|slice| slice.as_mut_ptr())
                .collect();

            let mut audio_inputs = clap_sys::audio_buffer::clap_audio_buffer {
                data32: ptr::null_mut(),
                data64: input_ptrs_f64.as_mut_ptr(),
                channel_count: buffer.inputs.len() as u32,
                latency: 0,
                constant_mask: 0,
            };

            let mut audio_outputs = clap_sys::audio_buffer::clap_audio_buffer {
                data32: ptr::null_mut(),
                data64: output_ptrs_f64.as_mut_ptr(),
                channel_count: buffer.outputs.len() as u32,
                latency: 0,
                constant_mask: 0,
            };

            // Build all input events (same as f32 path)
            let mut clap_events: Vec<ClapEvent> = midi_events
                .iter()
                .filter_map(Self::midi_to_clap_event)
                .collect();

            for expr in &note_expression.changes {
                clap_events.push(Self::note_expression_to_clap_event(expr));
            }
            clap_events.extend(Self::param_changes_to_clap_events(param_changes));

            clap_events.sort_by_key(|event| match event {
                ClapEvent::NoteOn { header, .. }
                | ClapEvent::NoteOff { header, .. }
                | ClapEvent::Midi { header, .. }
                | ClapEvent::NoteExpression { header, .. }
                | ClapEvent::ParamValue { header, .. } => header.time,
            });

            let input_events = ClapInputEventList::new(clap_events);
            let mut output_events = ClapOutputEventList::new();
            let clap_transport = Self::build_transport(transport);

            let process_data = clap_process {
                steady_time: transport.position_samples,
                frames_count: num_samples as u32,
                transport: &clap_transport,
                audio_inputs: &mut audio_inputs,
                audio_outputs: &mut audio_outputs,
                audio_inputs_count: 1,
                audio_outputs_count: 1,
                in_events: &input_events.list as *const _ as *const _,
                out_events: &mut output_events.list as *mut _ as *mut _,
            };

            let plugin_ref = unsafe { &*self.plugin };
            if let Some(process_fn) = plugin_ref.process {
                let status = unsafe { process_fn(self.plugin, &process_data) };
                if status == CLAP_PROCESS_ERROR {}
            }

            let midi_output = output_events.to_midi_events();
            let param_output = output_events.to_param_changes();
            let note_expression_output = output_events.to_note_expression_changes();

            (midi_output, param_output, note_expression_output)
        }

        #[cfg(not(feature = "clap"))]
        {
            let _ = (
                buffer,
                midi_events,
                param_changes,
                note_expression,
                transport,
            );
            (
                smallvec::SmallVec::new(),
                crate::protocol::ParameterChanges::new(),
                crate::protocol::NoteExpressionChanges::new(),
            )
        }
    }

    /// Set sample rate
    pub fn set_sample_rate(&mut self, rate: f32) {
        #[cfg(feature = "clap")]
        {
            // Deactivate first
            if self.is_processing {
                let plugin_ref = unsafe { &*self.plugin };
                if let Some(stop_fn) = plugin_ref.stop_processing {
                    unsafe { stop_fn(self.plugin) };
                }
                if let Some(deactivate_fn) = plugin_ref.deactivate {
                    unsafe { deactivate_fn(self.plugin) };
                }
                self.is_processing = false;
            }

            self.sample_rate = rate;
            // Will reactivate on next process call
        }

        #[cfg(not(feature = "clap"))]
        {
            self.sample_rate = rate;
        }
    }

    /// Get state extension (helper)
    #[cfg(feature = "clap")]
    fn get_state_extension(&self) -> Option<&clap_plugin_state> {
        let plugin_ref = unsafe { &*self.plugin };
        let get_ext = plugin_ref.get_extension?;

        let ext_ptr = unsafe { get_ext(self.plugin, CLAP_EXT_STATE.as_ptr()) };

        if ext_ptr.is_null() {
            None
        } else {
            Some(unsafe { &*(ext_ptr as *const clap_plugin_state) })
        }
    }

    /// Get state
    pub fn get_state(&self) -> Result<Vec<u8>> {
        #[cfg(feature = "clap")]
        {
            if let Some(state_ext) = self.get_state_extension() {
                if let Some(save_fn) = state_ext.save {
                    // Create output stream
                    let mut buffer = Vec::new();
                    let stream = create_output_stream(&mut buffer);

                    if unsafe { save_fn(self.plugin, &stream) } {
                        return Ok(buffer);
                    } else {
                        return Err(BridgeError::LoadFailed {
                            path: PathBuf::from("unknown"),
                            stage: LoadStage::Initialization,
                            reason: "State save failed".to_string(),
                        });
                    }
                }
            }
        }
        Ok(Vec::new())
    }

    /// Set state
    pub fn set_state(&mut self, data: &[u8]) -> Result<()> {
        #[cfg(feature = "clap")]
        {
            if data.is_empty() {
                return Ok(());
            }

            if let Some(state_ext) = self.get_state_extension() {
                if let Some(load_fn) = state_ext.load {
                    // Create input stream
                    let stream = create_input_stream(data);

                    if unsafe { load_fn(self.plugin, &stream) } {
                        return Ok(());
                    } else {
                        return Err(BridgeError::LoadFailed {
                            path: PathBuf::from("unknown"),
                            stage: LoadStage::Initialization,
                            reason: "State load failed".to_string(),
                        });
                    }
                }
            }
        }

        #[cfg(not(feature = "clap"))]
        let _ = data;

        Ok(())
    }

    /// Get GUI extension (helper)
    #[cfg(feature = "clap")]
    fn get_gui_extension(&self) -> Option<&clap_plugin_gui> {
        let plugin_ref = unsafe { &*self.plugin };
        let get_ext = plugin_ref.get_extension?;

        let ext_ptr = unsafe { get_ext(self.plugin, CLAP_EXT_GUI.as_ptr()) };

        if ext_ptr.is_null() {
            None
        } else {
            Some(unsafe { &*(ext_ptr as *const clap_plugin_gui) })
        }
    }

    /// Check if plugin has editor
    pub fn has_editor(&self) -> bool {
        #[cfg(feature = "clap")]
        {
            self.get_gui_extension().is_some()
        }

        #[cfg(not(feature = "clap"))]
        false
    }

    /// Open editor
    pub fn open_editor(&mut self, parent: *mut std::ffi::c_void) -> Result<(u32, u32)> {
        #[cfg(feature = "clap")]
        {
            if let Some(gui) = self.get_gui_extension() {
                // Create GUI (platform-specific API name)
                if let Some(create_fn) = gui.create {
                    #[cfg(target_os = "macos")]
                    let api = c"cocoa".as_ptr();
                    #[cfg(target_os = "windows")]
                    let api = c"win32".as_ptr();
                    #[cfg(target_os = "linux")]
                    let api = c"x11".as_ptr();

                    let is_floating = false;

                    if !unsafe { create_fn(self.plugin, api, is_floating) } {
                        return Err(BridgeError::LoadFailed {
                            path: PathBuf::from("unknown"),
                            stage: LoadStage::Initialization,
                            reason: "GUI create failed".to_string(),
                        });
                    }
                }

                // Set parent
                if let Some(set_parent_fn) = gui.set_parent {
                    #[cfg(target_os = "macos")]
                    let window = clap_window {
                        api: CLAP_WINDOW_API_COCOA.as_ptr(),
                        specific: clap_window_handle { cocoa: parent },
                    };

                    #[cfg(target_os = "windows")]
                    let window = clap_window {
                        api: CLAP_WINDOW_API_WIN32.as_ptr(),
                        specific: clap_window_handle { win32: parent },
                    };

                    #[cfg(target_os = "linux")]
                    let window = clap_window {
                        api: CLAP_WINDOW_API_X11.as_ptr(),
                        specific: clap_window_handle { x11: parent as u64 },
                    };

                    if !unsafe { set_parent_fn(self.plugin, &window) } {
                        return Err(BridgeError::LoadFailed {
                            path: PathBuf::from("unknown"),
                            stage: LoadStage::Initialization,
                            reason: "GUI set_parent failed".to_string(),
                        });
                    }
                }

                // Get size
                if let Some(get_size_fn) = gui.get_size {
                    let mut width: u32 = 0;
                    let mut height: u32 = 0;
                    if unsafe { get_size_fn(self.plugin, &mut width, &mut height) } {
                        // Show
                        if let Some(show_fn) = gui.show {
                            unsafe { show_fn(self.plugin) };
                        }
                        return Ok((width, height));
                    }
                }

                return Err(BridgeError::LoadFailed {
                    path: PathBuf::from("unknown"),
                    stage: LoadStage::Initialization,
                    reason: "Could not get GUI size".to_string(),
                });
            }

            Err(BridgeError::LoadFailed {
                path: PathBuf::from("unknown"),
                stage: LoadStage::Initialization,
                reason: "Plugin has no GUI extension".to_string(),
            })
        }

        #[cfg(not(feature = "clap"))]
        {
            let _ = parent;
            Err(BridgeError::LoadFailed {
                path: PathBuf::from("unknown"),
                stage: LoadStage::Opening,
                reason: "CLAP support not compiled".to_string(),
            })
        }
    }

    /// Close editor
    pub fn close_editor(&mut self) {
        #[cfg(feature = "clap")]
        {
            if let Some(gui) = self.get_gui_extension() {
                // Hide
                if let Some(hide_fn) = gui.hide {
                    unsafe { hide_fn(self.plugin) };
                }

                // Destroy
                if let Some(destroy_fn) = gui.destroy {
                    unsafe { destroy_fn(self.plugin) };
                }
            }
        }
    }

    /// Editor idle (stub for now)
    pub fn editor_idle(&mut self) {
        // CLAP doesn't need explicit idle ticks
    }

    /// Get params extension (helper)
    #[cfg(feature = "clap")]
    fn get_params_extension(&self) -> Option<&clap_plugin_params> {
        let plugin_ref = unsafe { &*self.plugin };
        let get_ext = plugin_ref.get_extension?;

        let ext_ptr = unsafe { get_ext(self.plugin, CLAP_EXT_PARAMS.as_ptr()) };

        if ext_ptr.is_null() {
            None
        } else {
            Some(unsafe { &*(ext_ptr as *const clap_plugin_params) })
        }
    }

    /// Get parameter count
    pub fn get_parameter_count(&self) -> u32 {
        #[cfg(feature = "clap")]
        {
            if let Some(params) = self.get_params_extension() {
                if let Some(count_fn) = params.count {
                    return unsafe { count_fn(self.plugin) };
                }
            }
        }
        0
    }

    /// Get parameter value by index (queries param ID first)
    pub fn get_parameter(&self, index: u32) -> f64 {
        #[cfg(feature = "clap")]
        {
            if let Some(params) = self.get_params_extension() {
                // Get param info to get the ID
                if let Some(get_info_fn) = params.get_info {
                    let mut param_info: clap_sys::ext::params::clap_param_info =
                        unsafe { std::mem::zeroed() };
                    if unsafe { get_info_fn(self.plugin, index, &mut param_info) } {
                        // Now get value by ID
                        if let Some(get_value_fn) = params.get_value {
                            let mut value: f64 = 0.0;
                            if unsafe { get_value_fn(self.plugin, param_info.id, &mut value) } {
                                return value;
                            }
                        }
                    }
                }
            }
        }

        #[cfg(not(feature = "clap"))]
        let _ = index;

        0.0
    }

    /// Set parameter value by index
    pub fn set_parameter(&mut self, index: u32, value: f64) {
        #[cfg(feature = "clap")]
        {
            if let Some(params) = self.get_params_extension() {
                // Get param info to get the ID
                if let Some(get_info_fn) = params.get_info {
                    let mut param_info: clap_sys::ext::params::clap_param_info =
                        unsafe { std::mem::zeroed() };
                    if unsafe { get_info_fn(self.plugin, index, &mut param_info) } {
                        // CLAP doesn't have a direct "set" - parameters are changed via events
                        // For now, we'd need to implement event queuing
                        // This is a simplified version that won't work for all plugins
                        let _ = (param_info, value);
                    }
                }
            }
        }

        #[cfg(not(feature = "clap"))]
        {
            let _ = (index, value);
        }
    }
}

#[cfg(feature = "clap")]
fn create_clap_host() -> clap_host {
    use std::os::raw::c_void;

    unsafe extern "C" fn get_extension(
        _host: *const clap_host,
        _extension_id: *const i8,
    ) -> *const c_void {
        ptr::null()
    }

    unsafe extern "C" fn request_restart(_host: *const clap_host) {}
    unsafe extern "C" fn request_process(_host: *const clap_host) {}
    unsafe extern "C" fn request_callback(_host: *const clap_host) {}

    clap_host {
        clap_version: CLAP_VERSION,
        host_data: ptr::null_mut(),
        name: c"DAWAI".as_ptr(),
        vendor: c"DAWAI Project".as_ptr(),
        url: c"https://github.com/dawai".as_ptr(),
        version: c"0.1.0".as_ptr(),
        get_extension: Some(get_extension),
        request_restart: Some(request_restart),
        request_process: Some(request_process),
        request_callback: Some(request_callback),
    }
}

/// Create output stream for saving state
#[cfg(feature = "clap")]
fn create_output_stream(buffer: &mut Vec<u8>) -> clap_ostream {
    use std::os::raw::c_void;
    use std::slice;

    unsafe extern "C" fn write(
        stream: *const clap_ostream,
        buffer: *const c_void,
        size: u64,
    ) -> i64 {
        let out_buffer = &mut *((*stream).ctx as *mut Vec<u8>);
        let data = slice::from_raw_parts(buffer as *const u8, size as usize);
        out_buffer.extend_from_slice(data);
        size as i64
    }

    clap_ostream {
        ctx: buffer as *mut Vec<u8> as *mut c_void,
        write: Some(write),
    }
}

/// Create input stream for loading state
#[cfg(feature = "clap")]
fn create_input_stream(data: &[u8]) -> clap_istream {
    use std::os::raw::c_void;
    use std::slice;

    unsafe extern "C" fn read(stream: *const clap_istream, buffer: *mut c_void, size: u64) -> i64 {
        let ctx = (*stream).ctx as *mut StreamContext;
        let ctx_ref = &*ctx;
        let remaining = ctx_ref.data.len() - ctx_ref.position;
        let to_read = (size as usize).min(remaining);

        if to_read == 0 {
            return 0;
        }

        let source = &ctx_ref.data[ctx_ref.position..ctx_ref.position + to_read];
        let dest = slice::from_raw_parts_mut(buffer as *mut u8, to_read);
        dest.copy_from_slice(source);

        (*ctx).position += to_read;
        to_read as i64
    }

    // Create context on heap (will leak, but short-lived)
    let ctx = Box::into_raw(Box::new(StreamContext { data, position: 0 }));

    clap_istream {
        ctx: ctx as *mut c_void,
        read: Some(read),
    }
}

#[cfg(feature = "clap")]
struct StreamContext<'a> {
    data: &'a [u8],
    position: usize,
}

#[cfg(feature = "clap")]
impl Drop for ClapInstance {
    fn drop(&mut self) {
        let plugin_ref = unsafe { &*self.plugin };

        // Stop processing
        if self.is_processing {
            if let Some(stop_fn) = plugin_ref.stop_processing {
                unsafe { stop_fn(self.plugin) };
            }
            if let Some(deactivate_fn) = plugin_ref.deactivate {
                unsafe { deactivate_fn(self.plugin) };
            }
        }

        // Destroy plugin
        if let Some(destroy_fn) = plugin_ref.destroy {
            unsafe { destroy_fn(self.plugin) };
        }
    }
}
