//! Plugin bridge trait — abstracts over in-process and out-of-process communication.

use crate::error::Result;
use crate::protocol::{
    MidiEventVec, NoteExpressionChanges, ParameterChanges, ParameterInfo, TransportInfo,
};

/// Trait abstracting the communication bridge between audio/control threads and a plugin.
///
/// Two implementations exist:
/// - `LockFreeBridge` — out-of-process (IPC via socket to plugin-server child process)
/// - `InProcessBridge` — in-process (direct calls to `PluginInstance` on a dedicated thread)
///
/// Both use the same lock-free ArrayQueue command/response pattern. The difference is
/// what the bridge thread does: serialize to IPC vs. call methods directly.
pub trait PluginBridge: Send + Sync {
    // =========================================================================
    // RT-safe methods (audio thread)
    // =========================================================================

    /// Process audio. Returns true on success.
    fn process(
        &self,
        num_samples: usize,
        midi_events: MidiEventVec,
        param_changes: ParameterChanges,
        note_expression: NoteExpressionChanges,
        transport: TransportInfo,
    ) -> bool;

    /// Set a parameter value. RT-safe, fire-and-forget.
    fn set_parameter_rt(&self, param_id: u32, value: f32) -> bool;

    /// Set sample rate. RT-safe.
    fn set_sample_rate_rt(&self, rate: f64) -> bool;

    /// Reset the plugin. RT-safe.
    fn reset_rt(&self) -> bool;

    /// Write input audio data for a channel (f32).
    fn write_input_channel(&self, channel: usize, data: &[f32]) -> Result<()>;

    /// Read output audio data for a channel into a buffer (f32).
    fn read_output_channel_into(&self, channel: usize, output: &mut [f32]) -> Result<usize>;

    /// Write input audio data for a channel (f64).
    fn write_input_channel_f64(&self, channel: usize, data: &[f64]) -> Result<()>;

    /// Read output audio data for a channel into a buffer (f64).
    fn read_output_channel_into_f64(&self, channel: usize, output: &mut [f64]) -> Result<usize>;

    // =========================================================================
    // Non-RT control methods (main thread)
    // =========================================================================

    /// Whether the plugin server/instance has crashed or disconnected.
    fn is_crashed(&self) -> bool;

    /// Open the plugin editor GUI. Returns (width, height) on success.
    fn open_editor(&self, parent_handle: u64) -> Option<(u32, u32)>;

    /// Close the plugin editor GUI.
    fn close_editor(&self) -> bool;

    /// Tick the plugin editor idle loop. Fire-and-forget.
    fn editor_idle(&self);

    /// Save the plugin state. Returns the state bytes on success.
    fn save_state(&self) -> Option<Vec<u8>>;

    /// Load plugin state from bytes.
    fn load_state(&self, data: &[u8]) -> bool;

    /// Get the full parameter list.
    fn get_parameter_list(&self) -> Option<Vec<ParameterInfo>>;

    /// Get a single parameter value.
    fn get_parameter(&self, param_id: u32) -> Option<f32>;
}
