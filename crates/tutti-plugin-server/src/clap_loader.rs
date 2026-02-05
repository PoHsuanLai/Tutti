//! CLAP plugin loader - thin wrapper around clap-host crate.
//!
//! This module wraps `clap_host::ClapInstance` to implement the
//! `PluginInstance` trait used by tutti-plugin's server.

#![allow(dead_code)] // Some functions used only by server module

use tutti_plugin::{BridgeError, LoadStage, PluginMetadata, Result};
use tutti_plugin::protocol::{
    AudioBuffer, AudioBuffer64, NoteExpressionChanges, ParameterChanges, ParameterFlags,
    ParameterInfo,
};
use crate::instance::{PluginInstance, ProcessContext, ProcessOutput};
use std::path::Path;

#[cfg(feature = "clap")]
use clap_host::ClapInstance as ClapHostInstance;

/// CLAP plugin instance wrapper.
///
/// Wraps `clap_host::ClapInstance` and implements `PluginInstance` trait.
pub struct ClapInstance {
    #[cfg(feature = "clap")]
    inner: ClapHostInstance,
    metadata: PluginMetadata,
}

// Safety: ClapHostInstance is Send
unsafe impl Send for ClapInstance {}

impl ClapInstance {
    /// Load a CLAP plugin from path.
    pub fn load(path: &Path, sample_rate: f64, block_size: usize) -> Result<Self> {
        #[cfg(feature = "clap")]
        {
            let inner =
                ClapHostInstance::load(path, sample_rate, block_size as u32).map_err(|e| {
                    BridgeError::LoadFailed {
                        path: path.to_path_buf(),
                        stage: match e {
                            clap_host::ClapError::LoadFailed { stage, .. } => match stage {
                                clap_host::LoadStage::Opening => LoadStage::Opening,
                                clap_host::LoadStage::Factory => LoadStage::Factory,
                                clap_host::LoadStage::Instantiation => LoadStage::Instantiation,
                                clap_host::LoadStage::Initialization => LoadStage::Initialization,
                                clap_host::LoadStage::Activation => LoadStage::Activation,
                            },
                            _ => LoadStage::Opening,
                        },
                        reason: e.to_string(),
                    }
                })?;

            let info = inner.info();
            let metadata = PluginMetadata::new(info.id.clone(), info.name.clone())
                .author(info.vendor.clone())
                .version(info.version.clone())
                .audio_io(info.audio_inputs, info.audio_outputs)
                .f64_support(true); // CLAP always supports f64

            Ok(Self { inner, metadata })
        }

        #[cfg(not(feature = "clap"))]
        {
            let _ = (path, sample_rate, block_size);
            Err(BridgeError::LoadFailed {
                path: path.to_path_buf(),
                stage: LoadStage::Opening,
                reason: "CLAP support not compiled (enable 'clap' feature)".to_string(),
            })
        }
    }
}

#[cfg(feature = "clap")]
impl PluginInstance for ClapInstance {
    fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    fn supports_f64(&self) -> bool {
        true // CLAP always supports f64
    }

    fn process_f32<'a>(
        &mut self,
        buffer: &'a mut AudioBuffer<'a>,
        ctx: &ProcessContext,
    ) -> ProcessOutput {
        // Convert tutti types to clap-host types
        let midi_events: Vec<clap_host::MidiEvent> =
            ctx.midi_events.iter().map(convert_midi_event).collect();

        let param_changes = ctx.param_changes.map(convert_param_changes);
        let note_expressions = ctx.note_expression.map(convert_note_expressions);
        let transport = ctx.transport.map(convert_transport);

        // Create clap-host buffer wrapper
        let mut clap_buffer = clap_host::AudioBuffer32 {
            inputs: buffer.inputs,
            outputs: buffer.outputs,
            num_samples: buffer.num_samples,
        };

        // Process
        let result = self
            .inner
            .process_f32(
                &mut clap_buffer,
                if midi_events.is_empty() {
                    None
                } else {
                    Some(&midi_events)
                },
                param_changes.as_ref(),
                note_expressions.as_deref(),
                transport.as_ref(),
            )
            .unwrap_or_default();

        // Convert output back to tutti types
        convert_process_output(result)
    }

    fn process_f64<'a>(
        &mut self,
        buffer: &'a mut AudioBuffer64<'a>,
        ctx: &ProcessContext,
    ) -> ProcessOutput {
        // Convert tutti types to clap-host types
        let midi_events: Vec<clap_host::MidiEvent> =
            ctx.midi_events.iter().map(convert_midi_event).collect();

        let param_changes = ctx.param_changes.map(convert_param_changes);
        let note_expressions = ctx.note_expression.map(convert_note_expressions);
        let transport = ctx.transport.map(convert_transport);

        // Create clap-host buffer wrapper
        let mut clap_buffer = clap_host::AudioBuffer64 {
            inputs: buffer.inputs,
            outputs: buffer.outputs,
            num_samples: buffer.num_samples,
        };

        // Process
        let result = self
            .inner
            .process_f64(
                &mut clap_buffer,
                if midi_events.is_empty() {
                    None
                } else {
                    Some(&midi_events)
                },
                param_changes.as_ref(),
                note_expressions.as_deref(),
                transport.as_ref(),
            )
            .unwrap_or_default();

        // Convert output back to tutti types
        convert_process_output(result)
    }

    fn set_sample_rate(&mut self, rate: f64) {
        self.inner.set_sample_rate(rate);
    }

    fn get_parameter_count(&self) -> usize {
        self.inner.parameter_count()
    }

    fn get_parameter(&self, id: u32) -> f64 {
        self.inner.get_parameter(id).unwrap_or(0.0)
    }

    fn set_parameter(&mut self, _id: u32, _value: f64) {
        // CLAP parameters are set via process events
        // For immediate setting, would need flush or queue for next process
    }

    fn get_parameter_list(&mut self) -> Vec<ParameterInfo> {
        self.inner
            .get_all_parameters()
            .into_iter()
            .map(convert_param_info)
            .collect()
    }

    fn get_parameter_info(&mut self, id: u32) -> Option<ParameterInfo> {
        // clap-host uses index, need to find by id
        let count = self.inner.parameter_count() as u32;
        for i in 0..count {
            if let Some(info) = self.inner.get_parameter_info(i) {
                if info.id == id {
                    return Some(convert_param_info(info));
                }
            }
        }
        None
    }

    fn has_editor(&mut self) -> bool {
        self.inner.has_gui()
    }

    unsafe fn open_editor(&mut self, parent: *mut std::ffi::c_void) -> Result<(u32, u32)> {
        self.inner
            .open_gui(parent)
            .map_err(|e| BridgeError::LoadFailed {
                path: std::path::PathBuf::from("editor"),
                stage: LoadStage::Initialization,
                reason: e.to_string(),
            })
    }

    fn close_editor(&mut self) {
        self.inner.close_gui();
    }

    fn editor_idle(&mut self) {
        // CLAP doesn't need explicit idle
    }

    fn get_state(&mut self) -> Result<Vec<u8>> {
        self.inner
            .save_state()
            .map_err(|e| BridgeError::LoadFailed {
                path: std::path::PathBuf::from("state"),
                stage: LoadStage::Initialization,
                reason: e.to_string(),
            })
    }

    fn set_state(&mut self, data: &[u8]) -> Result<()> {
        self.inner
            .load_state(data)
            .map_err(|e| BridgeError::LoadFailed {
                path: std::path::PathBuf::from("state"),
                stage: LoadStage::Initialization,
                reason: e.to_string(),
            })
    }
}

#[cfg(feature = "clap")]
fn convert_midi_event(event: &tutti_plugin::protocol::MidiEvent) -> clap_host::MidiEvent {
    use tutti_midi_io::ChannelVoiceMsg;

    let sample_offset = event.frame_offset as u32;
    let channel = event.channel as u8;

    let data = match event.msg {
        ChannelVoiceMsg::NoteOn { note, velocity } => clap_host::MidiData::NoteOn {
            key: note,
            velocity: velocity as f64 / 127.0,
        },
        ChannelVoiceMsg::NoteOff { note, velocity } => clap_host::MidiData::NoteOff {
            key: note,
            velocity: velocity as f64 / 127.0,
        },
        ChannelVoiceMsg::ControlChange { control } => {
            use tutti_midi_io::ControlChange;
            match control {
                ControlChange::CC { control, value } => clap_host::MidiData::ControlChange {
                    controller: control,
                    value,
                },
                ControlChange::CCHighRes {
                    control1, value, ..
                } => clap_host::MidiData::ControlChange {
                    controller: control1,
                    value: (value >> 7) as u8,
                },
                _ => clap_host::MidiData::ControlChange {
                    controller: 0,
                    value: 0,
                },
            }
        }
        ChannelVoiceMsg::ProgramChange { program } => {
            clap_host::MidiData::ProgramChange { program }
        }
        ChannelVoiceMsg::ChannelPressure { pressure } => clap_host::MidiData::ChannelPressure {
            pressure,
        },
        ChannelVoiceMsg::PitchBend { bend } => clap_host::MidiData::PitchBend { value: bend },
        ChannelVoiceMsg::PolyPressure { note, pressure } => clap_host::MidiData::PolyPressure {
            key: note,
            pressure: pressure as f64 / 127.0,
        },
        _ => clap_host::MidiData::NoteOff {
            key: 0,
            velocity: 0.0,
        },
    };

    clap_host::MidiEvent {
        sample_offset,
        channel,
        data,
    }
}

#[cfg(feature = "clap")]
fn convert_param_changes(changes: &ParameterChanges) -> clap_host::ParameterChanges {
    let mut result = clap_host::ParameterChanges::new();
    for queue in &changes.queues {
        let mut clap_queue = clap_host::ParameterQueue::new(queue.param_id);
        for point in &queue.points {
            clap_queue.add_point(point.sample_offset as u32, point.value);
        }
        result.add_queue(clap_queue);
    }
    result
}

#[cfg(feature = "clap")]
fn convert_note_expressions(
    changes: &NoteExpressionChanges,
) -> Vec<clap_host::NoteExpressionValue> {
    changes
        .changes
        .iter()
        .map(|expr| clap_host::NoteExpressionValue {
            sample_offset: expr.sample_offset as u32,
            note_id: expr.note_id,
            port_index: 0,
            channel: -1,
            key: -1,
            expression_type: match expr.expression_type {
                tutti_plugin::protocol::NoteExpressionType::Volume => {
                    clap_host::NoteExpressionType::Volume
                }
                tutti_plugin::protocol::NoteExpressionType::Pan => clap_host::NoteExpressionType::Pan,
                tutti_plugin::protocol::NoteExpressionType::Tuning => {
                    clap_host::NoteExpressionType::Tuning
                }
                tutti_plugin::protocol::NoteExpressionType::Vibrato => {
                    clap_host::NoteExpressionType::Vibrato
                }
                tutti_plugin::protocol::NoteExpressionType::Brightness => {
                    clap_host::NoteExpressionType::Brightness
                }
            },
            value: expr.value,
        })
        .collect()
}

#[cfg(feature = "clap")]
fn convert_transport(transport: &tutti_plugin::protocol::TransportInfo) -> clap_host::TransportInfo {
    clap_host::TransportInfo {
        playing: transport.playing,
        recording: transport.recording,
        loop_active: transport.cycle_active,
        tempo: transport.tempo,
        time_sig_numerator: transport.time_sig_numerator as u16,
        time_sig_denominator: transport.time_sig_denominator as u16,
        song_pos_beats: transport.position_quarters,
        song_pos_seconds: transport.position_samples as f64 / 44100.0, // Approximate
        loop_start_beats: transport.cycle_start_quarters,
        loop_end_beats: transport.cycle_end_quarters,
        bar_start: transport.bar_position_quarters,
        bar_number: 0,
    }
}

#[cfg(feature = "clap")]
fn convert_process_output(output: clap_host::instance::ProcessOutput) -> ProcessOutput {
    use tutti_midi_io::{Channel, ChannelVoiceMsg, ControlChange};

    let midi_events: tutti_plugin::protocol::MidiEventVec = output
        .midi_events
        .iter()
        .map(|e| {
            let msg = match e.data {
                clap_host::MidiData::NoteOn { key, velocity } => ChannelVoiceMsg::NoteOn {
                    note: key,
                    velocity: (velocity * 127.0) as u8,
                },
                clap_host::MidiData::NoteOff { key, velocity } => ChannelVoiceMsg::NoteOff {
                    note: key,
                    velocity: (velocity * 127.0) as u8,
                },
                clap_host::MidiData::ControlChange { controller, value } => {
                    ChannelVoiceMsg::ControlChange {
                        control: ControlChange::CC {
                            control: controller,
                            value,
                        },
                    }
                }
                clap_host::MidiData::ProgramChange { program } => {
                    ChannelVoiceMsg::ProgramChange { program }
                }
                clap_host::MidiData::ChannelPressure { pressure } => {
                    ChannelVoiceMsg::ChannelPressure { pressure }
                }
                clap_host::MidiData::PitchBend { value } => {
                    ChannelVoiceMsg::PitchBend { bend: value }
                }
                clap_host::MidiData::PolyPressure { key, pressure } => {
                    ChannelVoiceMsg::PolyPressure {
                        note: key,
                        pressure: (pressure * 127.0) as u8,
                    }
                }
            };
            tutti_plugin::protocol::MidiEvent {
                frame_offset: e.sample_offset as usize,
                channel: Channel::from_u8(e.channel),
                msg,
            }
        })
        .collect();

    let mut param_changes = ParameterChanges::new();
    for queue in output.param_changes.queues {
        let mut tutti_queue = tutti_plugin::protocol::ParameterQueue::new(queue.param_id);
        for point in queue.points {
            tutti_queue.add_point(point.sample_offset as i32, point.value);
        }
        param_changes.add_queue(tutti_queue);
    }

    let mut note_expression = NoteExpressionChanges::new();
    for expr in output.note_expressions {
        note_expression.add_change(tutti_plugin::protocol::NoteExpressionValue {
            sample_offset: expr.sample_offset as i32,
            note_id: expr.note_id,
            expression_type: match expr.expression_type {
                clap_host::NoteExpressionType::Volume => {
                    tutti_plugin::protocol::NoteExpressionType::Volume
                }
                clap_host::NoteExpressionType::Pan => tutti_plugin::protocol::NoteExpressionType::Pan,
                clap_host::NoteExpressionType::Tuning => {
                    tutti_plugin::protocol::NoteExpressionType::Tuning
                }
                clap_host::NoteExpressionType::Vibrato => {
                    tutti_plugin::protocol::NoteExpressionType::Vibrato
                }
                clap_host::NoteExpressionType::Brightness => {
                    tutti_plugin::protocol::NoteExpressionType::Brightness
                }
                clap_host::NoteExpressionType::Pressure => {
                    tutti_plugin::protocol::NoteExpressionType::Volume
                } // Map to volume
                clap_host::NoteExpressionType::Expression => {
                    tutti_plugin::protocol::NoteExpressionType::Volume
                } // Map to volume
            },
            value: expr.value,
        });
    }

    ProcessOutput {
        midi_events,
        param_changes,
        note_expression,
    }
}

#[cfg(feature = "clap")]
fn convert_param_info(info: clap_host::ParameterInfo) -> ParameterInfo {
    ParameterInfo {
        id: info.id,
        name: info.name,
        unit: String::new(),
        min_value: info.min_value,
        max_value: info.max_value,
        default_value: info.default_value,
        step_count: 0,
        flags: ParameterFlags {
            automatable: info.flags.is_automatable,
            read_only: info.flags.is_readonly,
            wrap: info.flags.is_periodic,
            is_bypass: info.flags.is_bypass,
            hidden: info.flags.is_hidden,
        },
    }
}
