//! Plugin instance trait and processing types.
//!
//! This module defines a unified interface for all plugin formats (VST2, VST3, CLAP).

use crate::error::Result;
use crate::protocol::{
    AudioBuffer, AudioBuffer64, MidiEvent, MidiEventVec, NoteExpressionChanges, ParameterChanges,
    ParameterInfo, PluginMetadata, TransportInfo,
};

/// Processing context passed to plugins.
///
/// Contains all optional data that can be sent to a plugin during processing:
/// MIDI events, parameter automation, note expression, and transport state.
#[derive(Default)]
pub struct ProcessContext<'a> {
    /// MIDI events to send to the plugin.
    pub midi_events: &'a [MidiEvent],
    /// Parameter automation changes (VST3/CLAP only, ignored by VST2).
    pub param_changes: Option<&'a ParameterChanges>,
    /// Note expression changes (VST3/CLAP only, ignored by VST2).
    pub note_expression: Option<&'a NoteExpressionChanges>,
    /// Transport state (tempo, position, play state).
    pub transport: Option<&'a TransportInfo>,
}

impl<'a> ProcessContext<'a> {
    /// Create a new empty process context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set MIDI events.
    pub fn midi(mut self, events: &'a [MidiEvent]) -> Self {
        self.midi_events = events;
        self
    }

    /// Set parameter automation changes.
    pub fn params(mut self, changes: &'a ParameterChanges) -> Self {
        self.param_changes = Some(changes);
        self
    }

    /// Set note expression changes.
    pub fn note_expression(mut self, changes: &'a NoteExpressionChanges) -> Self {
        self.note_expression = Some(changes);
        self
    }

    /// Set transport state.
    pub fn transport(mut self, info: &'a TransportInfo) -> Self {
        self.transport = Some(info);
        self
    }
}

/// Output from plugin processing.
#[derive(Default)]
pub struct ProcessOutput {
    /// MIDI events output by the plugin.
    pub midi_events: MidiEventVec,
    /// Parameter changes output by the plugin (gestures, etc.).
    pub param_changes: ParameterChanges,
    /// Note expression changes output by the plugin.
    pub note_expression: NoteExpressionChanges,
}

/// Unified trait for all plugin instance types.
///
/// This trait abstracts over VST2, VST3, and CLAP plugins, providing a common
/// interface for the plugin server to use regardless of format.
pub trait PluginInstance: Send {
    /// Get plugin metadata.
    fn metadata(&self) -> &PluginMetadata;

    /// Check if this plugin supports 64-bit (f64) audio processing.
    fn supports_f64(&self) -> bool;

    /// Process audio with f32 buffers.
    fn process_f32<'a>(
        &mut self,
        buffer: &'a mut AudioBuffer<'a>,
        ctx: &ProcessContext,
    ) -> ProcessOutput;

    /// Process audio with f64 buffers.
    fn process_f64<'a>(
        &mut self,
        buffer: &'a mut AudioBuffer64<'a>,
        ctx: &ProcessContext,
    ) -> ProcessOutput;

    /// Set sample rate.
    fn set_sample_rate(&mut self, rate: f64);

    /// Get parameter count.
    fn get_parameter_count(&self) -> usize;

    /// Get parameter value (normalized 0-1).
    fn get_parameter(&self, id: u32) -> f64;

    /// Set parameter value (normalized 0-1).
    fn set_parameter(&mut self, id: u32, value: f64);

    /// Get list of all parameters.
    fn get_parameter_list(&mut self) -> Vec<ParameterInfo>;

    /// Get info for a specific parameter.
    fn get_parameter_info(&mut self, id: u32) -> Option<ParameterInfo>;

    /// Check if plugin has an editor GUI.
    fn has_editor(&mut self) -> bool;

    /// Open the plugin editor.
    ///
    /// # Safety
    /// The parent pointer must be a valid window handle for the platform.
    unsafe fn open_editor(&mut self, parent: *mut std::ffi::c_void) -> Result<(u32, u32)>;

    /// Close the plugin editor.
    fn close_editor(&mut self);

    /// Idle callback for editor (call periodically to update GUI).
    fn editor_idle(&mut self);

    /// Save plugin state to bytes.
    fn get_state(&mut self) -> Result<Vec<u8>>;

    /// Load plugin state from bytes.
    fn set_state(&mut self, data: &[u8]) -> Result<()>;
}
