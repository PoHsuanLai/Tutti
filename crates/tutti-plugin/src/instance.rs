//! Plugin instance trait and processing types.
//!
//! This module defines a unified interface for all plugin formats (VST2, VST3, CLAP).

use crate::Result;
use crate::protocol::{
    AudioBuffer, AudioBuffer64, MidiEvent, MidiEventVec, NoteExpressionChanges, ParameterChanges,
    ParameterInfo, TransportInfo,
};
use crate::PluginMetadata;

#[derive(Default)]
pub struct ProcessContext<'a> {
    pub midi_events: &'a [MidiEvent],
    /// Parameter automation changes (VST3/CLAP only, ignored by VST2).
    pub param_changes: Option<&'a ParameterChanges>,
    /// Note expression changes (VST3/CLAP only, ignored by VST2).
    pub note_expression: Option<&'a NoteExpressionChanges>,
    /// Transport state (tempo, position, play state).
    pub transport: Option<&'a TransportInfo>,
}

impl<'a> ProcessContext<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn midi(mut self, events: &'a [MidiEvent]) -> Self {
        self.midi_events = events;
        self
    }

    pub fn params(mut self, changes: &'a ParameterChanges) -> Self {
        self.param_changes = Some(changes);
        self
    }

    pub fn note_expression(mut self, changes: &'a NoteExpressionChanges) -> Self {
        self.note_expression = Some(changes);
        self
    }

    pub fn transport(mut self, info: &'a TransportInfo) -> Self {
        self.transport = Some(info);
        self
    }
}

#[derive(Default)]
pub struct ProcessOutput {
    pub midi_events: MidiEventVec,
    pub param_changes: ParameterChanges,
    pub note_expression: NoteExpressionChanges,
}

/// Unified trait for all plugin instance types.
///
/// This trait abstracts over VST2, VST3, and CLAP plugins, providing a common
/// interface for the plugin server to use regardless of format.
pub trait PluginInstance: Send {
    fn metadata(&self) -> &PluginMetadata;

    fn supports_f64(&self) -> bool;

    fn process_f32<'a>(
        &mut self,
        buffer: &'a mut AudioBuffer<'a>,
        ctx: &ProcessContext,
    ) -> ProcessOutput;

    fn process_f64<'a>(
        &mut self,
        buffer: &'a mut AudioBuffer64<'a>,
        ctx: &ProcessContext,
    ) -> ProcessOutput;

    fn set_sample_rate(&mut self, rate: f64);

    fn get_parameter_count(&self) -> usize;

    /// Get parameter value (normalized 0-1).
    fn get_parameter(&self, id: u32) -> f64;

    /// Set parameter value (normalized 0-1).
    fn set_parameter(&mut self, id: u32, value: f64);

    fn get_parameter_list(&mut self) -> Vec<ParameterInfo>;

    fn get_parameter_info(&mut self, id: u32) -> Option<ParameterInfo>;

    fn has_editor(&mut self) -> bool;

    /// Open the plugin editor.
    ///
    /// # Safety
    /// The parent pointer must be a valid window handle for the platform.
    unsafe fn open_editor(&mut self, parent: *mut std::ffi::c_void) -> Result<(u32, u32)>;

    fn close_editor(&mut self);

    /// Idle callback for editor (call periodically to update GUI).
    fn editor_idle(&mut self);

    fn get_state(&mut self) -> Result<Vec<u8>>;

    fn set_state(&mut self, data: &[u8]) -> Result<()>;

    /// Stop audio processing. Called on the audio thread before shutdown.
    /// Default is a no-op; CLAP plugins override to call clap_plugin.stop_processing().
    fn stop_processing(&mut self) {}
}
