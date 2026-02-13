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
    // RT-safe methods (audio thread)

    fn process(
        &self,
        num_samples: usize,
        midi_events: MidiEventVec,
        param_changes: ParameterChanges,
        note_expression: NoteExpressionChanges,
        transport: TransportInfo,
    ) -> bool;

    /// RT-safe, fire-and-forget.
    fn set_parameter_rt(&self, param_id: u32, value: f32) -> bool;

    /// RT-safe.
    fn set_sample_rate_rt(&self, rate: f64) -> bool;

    /// RT-safe.
    fn reset_rt(&self) -> bool;

    fn write_input_channel(&self, channel: usize, data: &[f32]) -> Result<()>;
    fn read_output_channel_into(&self, channel: usize, output: &mut [f32]) -> Result<usize>;
    fn write_input_channel_f64(&self, channel: usize, data: &[f64]) -> Result<()>;
    fn read_output_channel_into_f64(&self, channel: usize, output: &mut [f64]) -> Result<usize>;

    // Non-RT control methods (main thread)

    fn is_crashed(&self) -> bool;

    /// Returns (width, height) on success.
    fn open_editor(&self, parent_handle: u64) -> Option<(u32, u32)>;
    fn close_editor(&self) -> bool;

    /// Fire-and-forget.
    fn editor_idle(&self);

    fn save_state(&self) -> Option<Vec<u8>>;
    fn load_state(&self, data: &[u8]) -> bool;
    fn get_parameter_list(&self) -> Option<Vec<ParameterInfo>>;
    fn get_parameter(&self, param_id: u32) -> Option<f32>;
}
