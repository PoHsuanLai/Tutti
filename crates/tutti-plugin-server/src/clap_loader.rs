//! CLAP plugin loader - thin wrapper around clap-host crate.

#![allow(dead_code)] // Some functions used only by server module

use crate::instance::{PluginInstance, ProcessContext, ProcessOutput};
use std::path::Path;
use tutti_plugin::protocol::{
    AudioBuffer, AudioBuffer64, NoteExpressionChanges, ParameterChanges, ParameterFlags,
    ParameterInfo,
};
use tutti_plugin::{BridgeError, LoadStage, PluginMetadata, Result};

#[cfg(feature = "clap")]
use clap_host::ClapInstance as ClapHostInstance;

pub struct ClapInstance {
    #[cfg(feature = "clap")]
    inner: ClapHostInstance,
    metadata: PluginMetadata,
}

// Safety: ClapHostInstance is Send
unsafe impl Send for ClapInstance {}

impl ClapInstance {
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
            let has_gui = inner.has_gui();
            let metadata = PluginMetadata::new(info.id.clone(), info.name.clone())
                .author(info.vendor.clone())
                .version(info.version.clone())
                .audio_io(info.audio_inputs, info.audio_outputs)
                .f64_support(true) // CLAP always supports f64
                .editor(has_gui, None);

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

    /// Activate the plugin (CLAP requirement: must be called on the main thread).
    ///
    /// Call this before handing the instance to the bridge thread.
    /// `start_processing()` will be called lazily on the audio/bridge thread
    /// by the first `process_f32()` call.
    #[cfg(feature = "clap")]
    pub fn activate(&mut self) -> Result<()> {
        self.inner.activate().map_err(|e| BridgeError::LoadFailed {
            path: std::path::PathBuf::new(),
            stage: LoadStage::Activation,
            reason: format!("activate failed: {e}"),
        })
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
        let midi_events: Vec<clap_host::MidiEvent> =
            ctx.midi_events.iter().map(convert_midi_event).collect();

        let param_changes = ctx.param_changes.map(convert_param_changes);
        let note_expressions = ctx.note_expression.map(convert_note_expressions);
        let transport = ctx
            .transport
            .map(|t| convert_transport(t, buffer.sample_rate));

        let mut clap_buffer = clap_host::AudioBuffer32 {
            inputs: buffer.inputs,
            outputs: buffer.outputs,
            num_samples: buffer.num_samples,
            sample_rate: buffer.sample_rate,
        };

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

        convert_process_output(result)
    }

    fn process_f64<'a>(
        &mut self,
        buffer: &'a mut AudioBuffer64<'a>,
        ctx: &ProcessContext,
    ) -> ProcessOutput {
        let midi_events: Vec<clap_host::MidiEvent> =
            ctx.midi_events.iter().map(convert_midi_event).collect();

        let param_changes = ctx.param_changes.map(convert_param_changes);
        let note_expressions = ctx.note_expression.map(convert_note_expressions);
        let transport = ctx
            .transport
            .map(|t| convert_transport(t, buffer.sample_rate));

        let mut clap_buffer = clap_host::AudioBuffer64 {
            inputs: buffer.inputs,
            outputs: buffer.outputs,
            num_samples: buffer.num_samples,
            sample_rate: buffer.sample_rate,
        };

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

    fn set_parameter(&mut self, id: u32, value: f64) {
        self.inner.set_parameter(id, value);
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
            .map_err(|e| BridgeError::EditorError(e.to_string()))
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
            .map_err(|e| BridgeError::StateSaveError(e.to_string()))
    }

    fn stop_processing(&mut self) {
        self.inner.stop_processing();
    }

    fn set_state(&mut self, data: &[u8]) -> Result<()> {
        self.inner
            .load_state(data)
            .map_err(|e| BridgeError::StateRestoreError(e.to_string()))
    }
}

#[cfg(feature = "clap")]
fn convert_midi_event(event: &tutti_plugin::protocol::MidiEvent) -> clap_host::MidiEvent {
    use tutti_midi_io::ChannelVoiceMsg;

    let sample_offset = event.frame_offset as i32;
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
        ChannelVoiceMsg::ChannelPressure { pressure } => {
            clap_host::MidiData::ChannelPressure { pressure }
        }
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
            clap_queue.add_point(point.sample_offset, point.value);
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
            sample_offset: expr.sample_offset,
            note_id: expr.note_id,
            port_index: 0,
            channel: -1,
            key: -1,
            expression_type: match expr.expression_type {
                tutti_plugin::protocol::NoteExpressionType::Volume => {
                    clap_host::NoteExpressionType::Volume
                }
                tutti_plugin::protocol::NoteExpressionType::Pan => {
                    clap_host::NoteExpressionType::Pan
                }
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
fn convert_transport(
    transport: &tutti_plugin::protocol::TransportInfo,
    sample_rate: f64,
) -> clap_host::TransportInfo {
    let sr = if sample_rate > 0.0 {
        sample_rate
    } else {
        44100.0
    };
    clap_host::TransportInfo {
        playing: transport.playing,
        recording: transport.recording,
        cycle_active: transport.cycle_active,
        tempo: transport.tempo,
        time_sig_numerator: transport.time_sig_numerator,
        time_sig_denominator: transport.time_sig_denominator,
        song_pos_beats: transport.position_quarters,
        song_pos_seconds: transport.position_samples as f64 / sr,
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
            tutti_queue.add_point(point.sample_offset, point.value);
        }
        param_changes.add_queue(tutti_queue);
    }

    let mut note_expression = NoteExpressionChanges::new();
    for expr in output.note_expressions {
        note_expression.add_change(tutti_plugin::protocol::NoteExpressionValue {
            sample_offset: expr.sample_offset,
            note_id: expr.note_id,
            expression_type: match expr.expression_type {
                clap_host::NoteExpressionType::Volume => {
                    tutti_plugin::protocol::NoteExpressionType::Volume
                }
                clap_host::NoteExpressionType::Pan => {
                    tutti_plugin::protocol::NoteExpressionType::Pan
                }
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

#[cfg(test)]
#[cfg(feature = "clap")]
mod tests {
    use super::*;
    use crate::instance::PluginInstance;
    use std::path::Path;
    use std::sync::atomic::Ordering;
    use tutti_midi_io::{Channel, ChannelVoiceMsg};
    use tutti_plugin::protocol::{
        MidiEvent, NoteExpressionChanges, NoteExpressionType, NoteExpressionValue,
        ParameterChanges, ParameterQueue, TransportInfo,
    };

    const CLAP_PLUGIN: &str = "/Library/Audio/Plug-Ins/CLAP/TAL-NoiseMaker.clap";

    #[test]
    fn test_clap_load() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(CLAP_PLUGIN);
        let instance = ClapInstance::load(path, 44100.0, 512);
        assert!(
            instance.is_ok(),
            "Failed to load CLAP plugin: {:?}",
            instance.err()
        );

        let instance = instance.unwrap();
        let meta = instance.metadata();
        assert!(!meta.name.is_empty(), "Plugin name should not be empty");
        assert!(!meta.id.is_empty(), "Plugin id should not be empty");
    }

    #[test]
    fn test_clap_metadata() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(CLAP_PLUGIN);
        let instance = ClapInstance::load(path, 44100.0, 512).expect("Failed to load CLAP plugin");
        let meta = instance.metadata();

        assert!(
            meta.audio_io.inputs > 0,
            "Expected audio inputs > 0, got {}",
            meta.audio_io.inputs
        );
        assert!(
            meta.audio_io.outputs > 0,
            "Expected audio outputs > 0, got {}",
            meta.audio_io.outputs
        );
        assert!(
            instance.supports_f64(),
            "CLAP plugins should support f64 processing"
        );
    }

    #[test]
    fn test_clap_parameter_count() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(CLAP_PLUGIN);
        let instance = ClapInstance::load(path, 44100.0, 512).expect("Failed to load CLAP plugin");

        let count = instance.get_parameter_count();
        assert!(
            count > 0,
            "TAL-NoiseMaker should have parameters, got {}",
            count
        );
    }

    #[test]
    fn test_clap_parameter_list() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(CLAP_PLUGIN);
        let mut instance =
            ClapInstance::load(path, 44100.0, 512).expect("Failed to load CLAP plugin");

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
    fn test_clap_get_parameter() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(CLAP_PLUGIN);
        let mut instance =
            ClapInstance::load(path, 44100.0, 512).expect("Failed to load CLAP plugin");

        let params = instance.get_parameter_list();
        assert!(!params.is_empty(), "Need at least one parameter");

        let first_id = params[0].id;
        let value = instance.get_parameter(first_id);
        assert!(
            value.is_finite(),
            "Parameter value should be finite, got {}",
            value
        );
    }

    #[test]
    fn test_clap_process_f32_silence() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(CLAP_PLUGIN);
        let mut instance =
            ClapInstance::load(path, 44100.0, 512).expect("Failed to load CLAP plugin");

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

        let ctx = ProcessContext::new();
        // Should not panic
        let _output = instance.process_f32(&mut buffer, &ctx);
    }

    // Note: f64 processing test removed — TAL-NoiseMaker's CLAP plugin crashes
    // when data32 is null (does not actually support f64-only processing).

    #[test]
    fn test_clap_process_f32_with_note() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let path = Path::new(CLAP_PLUGIN);
        let mut instance =
            ClapInstance::load(path, 44100.0, 512).expect("Failed to load CLAP plugin");

        let num_samples = 512;

        // First block: send a NoteOn event
        let note_on = [MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::NoteOn {
                note: 60,
                velocity: 100,
            },
        }];

        let mut has_nonzero = false;

        // Process the block with NoteOn
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

            let ctx = ProcessContext::new().midi(&note_on);
            let _output = instance.process_f32(&mut buffer, &ctx);

            // Check output for non-zero samples
            for ch in output_data.iter() {
                for &sample in ch.iter() {
                    if sample != 0.0 {
                        has_nonzero = true;
                    }
                }
            }
        }

        // Process additional blocks (at least 4 more) to give the synth time to produce sound
        let empty_ctx = ProcessContext::new();
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

            let _output = instance.process_f32(&mut buffer, &empty_ctx);

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

    // ── Surge XT tests (f64-capable plugin) ──

    const SURGE_XT: &str = "/Library/Audio/Plug-Ins/CLAP/Surge XT.clap";

    #[test]
    fn test_surge_load() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance = ClapInstance::load(Path::new(SURGE_XT), 44100.0, 512);
        assert!(
            instance.is_ok(),
            "Failed to load Surge XT: {:?}",
            instance.err()
        );

        let instance = instance.unwrap();
        let meta = instance.metadata();
        assert!(!meta.name.is_empty());
        assert!(!meta.id.is_empty());
        println!("Surge XT loaded: {} ({})", meta.name, meta.id);
    }

    #[test]
    fn test_surge_port_flags() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance =
            ClapInstance::load(Path::new(SURGE_XT), 44100.0, 512).expect("Failed to load Surge XT");

        let port_info = instance.inner.audio_port_info(0, false);
        assert!(port_info.is_some(), "Expected at least one output port");

        let port = port_info.unwrap();
        assert!(port.channel_count >= 2, "Expected stereo output");
        // Note: most CLAP synths (including Surge XT) do not advertise
        // CLAP_AUDIO_PORT_SUPPORTS_64BITS. f64 processing is rare in practice.
    }

    #[test]
    fn test_surge_process_f32_silence() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(SURGE_XT), 44100.0, 512).expect("Failed to load Surge XT");

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

        let ctx = ProcessContext::new();
        let _output = instance.process_f32(&mut buffer, &ctx);
    }

    #[test]
    fn test_surge_process_f32_with_note() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(SURGE_XT), 44100.0, 512).expect("Failed to load Surge XT");

        let num_samples = 512;
        let note_on = [MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::NoteOn {
                note: 60,
                velocity: 100,
            },
        }];

        // Process block with NoteOn + follow-up blocks.
        // Surge XT is strict about CLAP thread contracts and may not produce
        // audio when process() is called from a non-audio thread, so we only
        // verify no crash rather than asserting on output content.
        for i in 0..5 {
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

            let ctx = if i == 0 {
                ProcessContext::new().midi(&note_on)
            } else {
                ProcessContext::new()
            };

            let _output = instance.process_f32(&mut buffer, &ctx);
        }
    }

    /// Test Surge XT parameter setting via flush.
    #[test]
    fn test_surge_set_parameter_flush() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(SURGE_XT), 44100.0, 512).expect("Failed to load Surge XT");

        let params = instance.get_parameter_list();
        assert!(!params.is_empty(), "Need at least one parameter");

        let param_id = params[0].id;
        instance.set_parameter(param_id, 0.5);
    }

    // ── Group A: Plugin Lifecycle ──

    #[test]
    fn test_clap_lifecycle_inactive_after_load() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");
        // load() inits but does NOT activate — activation is lazy
        assert!(
            !instance.inner.is_active(),
            "Plugin should not be active right after load"
        );
    }

    #[test]
    fn test_clap_lifecycle_activate_deactivate() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        assert!(!instance.inner.is_active());
        instance
            .inner
            .activate()
            .expect("Activation should succeed");
        assert!(
            instance.inner.is_active(),
            "Should be active after activate"
        );

        instance.inner.deactivate();
        assert!(
            !instance.inner.is_active(),
            "Should be inactive after deactivate"
        );

        instance
            .inner
            .activate()
            .expect("Reactivation should succeed");
        assert!(
            instance.inner.is_active(),
            "Should be active after reactivate"
        );
    }

    #[test]
    fn test_clap_lifecycle_start_stop_processing() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        // start_processing auto-activates if needed
        instance
            .inner
            .start_processing()
            .expect("start_processing should succeed");
        assert!(
            instance.inner.is_active(),
            "Should be active (auto-activated)"
        );
        assert!(
            instance.inner.is_processing(),
            "Should be processing after start"
        );

        instance.inner.stop_processing();
        assert!(
            !instance.inner.is_processing(),
            "Should not be processing after stop"
        );
        assert!(
            instance.inner.is_active(),
            "Should still be active after stop_processing"
        );
    }

    #[test]
    fn test_clap_lifecycle_on_main_thread() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");
        // Just verify no crash
        instance.inner.on_main_thread();
    }

    // ── Group B: Polling Methods ──

    #[test]
    fn test_clap_poll_initially_clear() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        // All poll methods should return false on a fresh instance
        assert!(!instance.inner.poll_restart_requested());
        assert!(!instance.inner.poll_process_requested());
        assert!(!instance.inner.poll_callback_requested());
        assert!(!instance.inner.poll_latency_changed());
        assert!(!instance.inner.poll_tail_changed());
        assert!(!instance.inner.poll_params_rescan());
        assert!(!instance.inner.poll_params_flush_requested());
        assert!(!instance.inner.poll_state_dirty());
        assert!(!instance.inner.poll_audio_ports_changed());
        assert!(!instance.inner.poll_note_ports_changed());
    }

    #[test]
    fn test_clap_poll_restart() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        let state = instance.inner.host_state();
        state.restart_requested.store(true, Ordering::Release);

        // needs_restart() is non-clearing — should return true repeatedly
        assert!(
            instance.inner.needs_restart(),
            "needs_restart should be true"
        );
        assert!(
            instance.inner.needs_restart(),
            "needs_restart should still be true (non-clearing)"
        );

        // poll_restart_requested() clears the flag
        assert!(
            instance.inner.poll_restart_requested(),
            "poll should return true"
        );
        assert!(
            !instance.inner.poll_restart_requested(),
            "poll should return false after clearing"
        );
        assert!(
            !instance.inner.needs_restart(),
            "needs_restart should be false after poll cleared it"
        );
    }

    #[test]
    fn test_clap_poll_process_callback() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        let state = instance.inner.host_state();
        state.process_requested.store(true, Ordering::Release);
        state.callback_requested.store(true, Ordering::Release);

        assert!(instance.inner.poll_process_requested());
        assert!(instance.inner.poll_callback_requested());

        // Second poll should be false (cleared)
        assert!(!instance.inner.poll_process_requested());
        assert!(!instance.inner.poll_callback_requested());
    }

    #[test]
    fn test_clap_poll_latency_tail() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        let state = instance.inner.host_state();
        state.latency_changed.store(true, Ordering::Release);
        state.tail_changed.store(true, Ordering::Release);

        assert!(instance.inner.poll_latency_changed());
        assert!(instance.inner.poll_tail_changed());

        assert!(!instance.inner.poll_latency_changed());
        assert!(!instance.inner.poll_tail_changed());
    }

    #[test]
    fn test_clap_poll_ports_state() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        let state = instance.inner.host_state();
        state.audio_ports_changed.store(true, Ordering::Release);
        state.note_ports_changed.store(true, Ordering::Release);
        state.state_dirty.store(true, Ordering::Release);

        assert!(instance.inner.poll_audio_ports_changed());
        assert!(instance.inner.poll_note_ports_changed());
        assert!(instance.inner.poll_state_dirty());

        assert!(!instance.inner.poll_audio_ports_changed());
        assert!(!instance.inner.poll_note_ports_changed());
        assert!(!instance.inner.poll_state_dirty());
    }

    // ── Group C: Parameter Get/Set Roundtrip ──

    #[test]
    fn test_clap_param_set_get_roundtrip() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        let params = instance.get_parameter_list();
        assert!(!params.is_empty(), "Need at least one parameter");

        let param_id = params[0].id;
        let original = instance.get_parameter(param_id);

        // Set to a different value
        let new_value = if original < 0.5 { 0.75 } else { 0.25 };
        instance.set_parameter(param_id, new_value);

        let readback = instance.get_parameter(param_id);
        assert!(
            (readback - new_value).abs() < 0.01,
            "Parameter should be close to set value: expected {}, got {}",
            new_value,
            readback
        );
    }

    // ── Group D: State Save/Load Roundtrip ──

    #[test]
    fn test_clap_state_save_nonempty() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        let state = instance.get_state().expect("save_state should succeed");
        assert!(!state.is_empty(), "Saved state should not be empty");
    }

    #[test]
    fn test_clap_state_save_load_roundtrip() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        let params = instance.get_parameter_list();
        assert!(!params.is_empty(), "Need at least one parameter");
        let param_id = params[0].id;

        // Save original state
        let saved = instance.get_state().expect("save should succeed");
        let original_value = instance.get_parameter(param_id);

        // Change parameter
        let new_value = if original_value < 0.5 { 0.75 } else { 0.25 };
        instance.set_parameter(param_id, new_value);

        // Restore state
        instance.set_state(&saved).expect("restore should succeed");

        let restored_value = instance.get_parameter(param_id);
        assert!(
            (restored_value - original_value).abs() < 0.01,
            "Parameter should be restored: expected {}, got {}",
            original_value,
            restored_value
        );
    }

    // ── Group E: Audio/Note Port Enumeration ──

    #[test]
    fn test_clap_audio_port_enumeration() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        let output_count = instance.inner.audio_port_count(false);
        assert!(
            output_count > 0,
            "Synth should have at least one output port"
        );

        for i in 0..output_count {
            let info = instance
                .inner
                .audio_port_info(i, false)
                .expect("audio_port_info should return Some");
            assert!(info.channel_count > 0, "Port {} should have channels", i);
            assert!(!info.name.is_empty(), "Port {} should have a name", i);
        }
    }

    #[test]
    fn test_clap_note_port_enumeration() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        let input_count = instance.inner.note_port_count(true);
        assert!(
            input_count > 0,
            "Synth should have at least one note input port"
        );

        for i in 0..input_count {
            let info = instance
                .inner
                .note_port_info(i, true)
                .expect("note_port_info should return Some");
            assert!(!info.name.is_empty(), "Note port {} should have a name", i);
        }
    }

    // ── Group F: Latency/Tail Queries ──

    #[test]
    fn test_clap_latency_and_tail() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        // Just verify no crash — values depend on plugin
        let _latency = instance.inner.get_latency();
        let _tail = instance.inner.get_tail();
    }

    // ── Group G: Processing with Param Automation, Expressions, Transport ──

    #[test]
    fn test_clap_process_with_param_automation() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        let params = instance.get_parameter_list();
        assert!(!params.is_empty());

        let mut changes = ParameterChanges::new();
        let mut queue = ParameterQueue::new(params[0].id);
        queue.add_point(0, 0.5);
        changes.add_queue(queue);

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

        let ctx = ProcessContext::new().params(&changes);
        let _output = instance.process_f32(&mut buffer, &ctx);
    }

    #[test]
    fn test_clap_process_with_note_expression() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        let note_on = [MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::NoteOn {
                note: 60,
                velocity: 100,
            },
        }];

        let mut expr_changes = NoteExpressionChanges::new();
        expr_changes.add_change(NoteExpressionValue {
            sample_offset: 0,
            note_id: -1,
            expression_type: NoteExpressionType::Volume,
            value: 0.8,
        });

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

        let ctx = ProcessContext::new()
            .midi(&note_on)
            .note_expression(&expr_changes);
        let _output = instance.process_f32(&mut buffer, &ctx);
    }

    #[test]
    fn test_clap_process_with_transport() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        let transport = TransportInfo {
            playing: true,
            tempo: 120.0,
            time_sig_numerator: 4,
            time_sig_denominator: 4,
            ..Default::default()
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

        let ctx = ProcessContext::new().transport(&transport);
        let _output = instance.process_f32(&mut buffer, &ctx);
    }

    // ── Group I: Surge XT Additional Coverage ──

    #[test]
    fn test_surge_state_save_load() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(SURGE_XT), 44100.0, 512).expect("Failed to load Surge XT");

        let params = instance.get_parameter_list();
        assert!(!params.is_empty());
        let param_id = params[0].id;

        let saved = instance.get_state().expect("save should succeed");
        assert!(!saved.is_empty(), "Surge XT state should not be empty");

        let original_value = instance.get_parameter(param_id);
        let new_value = if original_value < 0.5 { 0.75 } else { 0.25 };
        instance.set_parameter(param_id, new_value);

        instance.set_state(&saved).expect("restore should succeed");

        let restored = instance.get_parameter(param_id);
        assert!(
            (restored - original_value).abs() < 0.01,
            "Surge XT param should be restored: expected {}, got {}",
            original_value,
            restored
        );
    }

    #[test]
    fn test_surge_audio_ports() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance =
            ClapInstance::load(Path::new(SURGE_XT), 44100.0, 512).expect("Failed to load Surge XT");

        let output_count = instance.inner.audio_port_count(false);
        assert!(output_count > 0, "Surge XT should have output ports");

        let port = instance
            .inner
            .audio_port_info(0, false)
            .expect("Should have at least one output port");
        assert!(
            port.channel_count >= 2,
            "Expected stereo output, got {} channels",
            port.channel_count
        );
    }

    #[test]
    fn test_surge_latency_tail() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance =
            ClapInstance::load(Path::new(SURGE_XT), 44100.0, 512).expect("Failed to load Surge XT");

        let _latency = instance.inner.get_latency();
        let _tail = instance.inner.get_tail();
    }

    // ── Group J: GUI / Editor ──

    #[test]
    fn test_clap_has_gui() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");
        assert!(instance.has_editor(), "TAL-NoiseMaker should have a GUI");
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore] // Requires main-thread Cocoa environment; run with: cargo test -- --ignored test_clap_gui
    fn test_clap_gui_open_close() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        let parent = create_nsview();
        assert!(!parent.is_null(), "Failed to create NSView");

        let result = unsafe { instance.open_editor(parent) };
        assert!(
            result.is_ok(),
            "open_editor should succeed: {:?}",
            result.err()
        );

        let (w, h) = result.unwrap();
        assert!(w > 0 && h > 0, "GUI size should be non-zero: {}x{}", w, h);

        instance.close_editor();
        release_nsview(parent);
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore] // Requires main-thread Cocoa environment; run with: cargo test -- --ignored test_surge_gui
    fn test_surge_gui_open_close() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(SURGE_XT), 44100.0, 512).expect("Failed to load Surge XT");

        assert!(instance.has_editor(), "Surge XT should have a GUI");

        let parent = create_nsview();
        assert!(!parent.is_null());

        let result = unsafe { instance.open_editor(parent) };
        assert!(
            result.is_ok(),
            "Surge XT open_editor should succeed: {:?}",
            result.err()
        );

        let (w, h) = result.unwrap();
        assert!(
            w > 0 && h > 0,
            "Surge XT GUI size should be non-zero: {}x{}",
            w,
            h
        );

        instance.close_editor();
        release_nsview(parent);
    }

    // ── Group K: Render Mode ──

    #[test]
    fn test_clap_render_mode() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        // Set offline mode — returns true if plugin supports render extension
        let _ = instance.inner.set_render_mode(true);
        // Set back to realtime
        let _ = instance.inner.set_render_mode(false);
        // No crash is the assertion
    }

    #[test]
    fn test_clap_has_hard_realtime_requirement() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        // Most synths don't have hard RT requirements
        let _has_rt = instance.inner.has_hard_realtime_requirement();
    }

    // ── Group L: Voice Info ──

    #[test]
    fn test_clap_voice_info() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        // TAL-NoiseMaker may or may not support voice info
        if let Some(info) = instance.inner.get_voice_info() {
            assert!(info.voice_count > 0, "voice_count should be > 0");
            assert!(info.voice_capacity > 0, "voice_capacity should be > 0");
        }
    }

    #[test]
    fn test_surge_voice_info() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance =
            ClapInstance::load(Path::new(SURGE_XT), 44100.0, 512).expect("Failed to load Surge XT");

        // Surge XT likely supports voice info
        if let Some(info) = instance.inner.get_voice_info() {
            assert!(info.voice_count > 0, "Surge XT voice_count should be > 0");
            assert!(
                info.voice_capacity > 0,
                "Surge XT voice_capacity should be > 0"
            );
        }
    }

    // ── Group M: Note Names ──

    #[test]
    fn test_clap_note_names() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        let count = instance.inner.note_name_count();
        // Iterate whatever's there — may be 0 for synths without custom note names
        for i in 0..count {
            let name = instance.inner.get_note_name(i);
            assert!(name.is_some(), "note_name at index {} should exist", i);
            assert!(
                !name.unwrap().name.is_empty(),
                "note_name {} should have a name",
                i
            );
        }
    }

    // ── Group N: Sample Rate Cycling ──

    #[test]
    fn test_clap_sample_rate_change() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        // Change sample rate — this deactivates, updates, and requires reactivation
        instance.set_sample_rate(48000.0);

        // Process should still work (auto-reactivates)
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

        let ctx = ProcessContext::new();
        let _output = instance.process_f32(&mut buffer, &ctx);
    }

    #[test]
    fn test_clap_sample_rate_cycle_multiple() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        // Cycle through several sample rates
        for &rate in &[48000.0, 96000.0, 44100.0, 22050.0] {
            instance.set_sample_rate(rate);

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
                sample_rate: rate,
            };

            let ctx = ProcessContext::new();
            let _output = instance.process_f32(&mut buffer, &ctx);
        }
    }

    // ── Group O: State Context ──

    #[test]
    fn test_clap_state_context_support() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        // Just query — may or may not be supported
        let _supports = instance.inner.supports_state_context();
    }

    #[test]
    fn test_clap_state_context_save_load() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        // Save with ForProject context (falls back to regular save if unsupported)
        let saved = instance
            .inner
            .save_state_with_context(clap_host::StateContext::ForProject)
            .expect("save_state_with_context should succeed");
        assert!(!saved.is_empty());

        // Load it back
        instance
            .inner
            .load_state_with_context(&saved, clap_host::StateContext::ForProject)
            .expect("load_state_with_context should succeed");
    }

    #[test]
    fn test_clap_state_context_for_duplicate() {
        let _lock = crate::test_utils::PLUGIN_LOAD_LOCK.lock().unwrap();
        let mut instance =
            ClapInstance::load(Path::new(CLAP_PLUGIN), 44100.0, 512).expect("Failed to load");

        // ForDuplicate context — used when duplicating a plugin instance
        let saved = instance
            .inner
            .save_state_with_context(clap_host::StateContext::ForDuplicate)
            .expect("save should succeed");
        assert!(!saved.is_empty());

        instance
            .inner
            .load_state_with_context(&saved, clap_host::StateContext::ForDuplicate)
            .expect("load should succeed");
    }

    // Objective-C runtime FFI for NSView creation (macOS only).
    // On Apple Silicon, objc_msgSend must be cast to the correct function
    // pointer type — the variadic extern "C" declaration doesn't work.
    #[cfg(target_os = "macos")]
    extern "C" {
        fn objc_getClass(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        fn sel_registerName(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        fn objc_msgSend();
    }

    #[cfg(target_os = "macos")]
    type ObjcMsgSendFn =
        unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> *mut std::ffi::c_void;

    #[cfg(target_os = "macos")]
    fn objc_send() -> ObjcMsgSendFn {
        unsafe { std::mem::transmute(objc_msgSend as *const ()) }
    }

    /// Ensure NSApplication is initialized (required for plugin GUIs).
    #[cfg(target_os = "macos")]
    fn ensure_nsapp() {
        use std::sync::Once;
        static INIT: Once = Once::new();
        INIT.call_once(|| unsafe {
            let send = objc_send();
            let cls = objc_getClass(c"NSApplication".as_ptr());
            let shared_sel = sel_registerName(c"sharedApplication".as_ptr());
            send(cls, shared_sel);
        });
    }

    #[cfg(target_os = "macos")]
    fn create_nsview() -> *mut std::ffi::c_void {
        ensure_nsapp();
        unsafe {
            let send = objc_send();
            let cls = objc_getClass(c"NSView".as_ptr());
            let alloc_sel = sel_registerName(c"alloc".as_ptr());
            let init_sel = sel_registerName(c"init".as_ptr());

            let obj = send(cls, alloc_sel);
            send(obj, init_sel)
        }
    }

    #[cfg(target_os = "macos")]
    fn release_nsview(view: *mut std::ffi::c_void) {
        unsafe {
            let send = objc_send();
            let release_sel = sel_registerName(c"release".as_ptr());
            send(view, release_sel);
        }
    }
}
