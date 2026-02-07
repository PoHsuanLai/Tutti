mod click;
mod clock;
mod export_timeline;
pub(crate) mod fsm;
mod handle;
pub(crate) mod manager;
pub(crate) mod position;
pub mod sync;
pub(crate) mod tempo_map;

pub use click::{click, ClickNode, ClickState, MetronomeMode};
pub use clock::{
    curves as automation_curves, AutomationEnvelopeFn, AutomationReaderInput, TransportClock,
};
pub use export_timeline::{ExportConfig, ExportTimeline};
pub use handle::{MetronomeHandle, TransportHandle};
pub use manager::{Direction, MotionState, TransportManager};
pub use sync::{SmpteFrameRate, SyncSnapshot, SyncSource, SyncState, SyncStatus};
pub use tempo_map::{TempoMap, TimeSignature, BBT};

/// Trait for reading transport state.
///
/// This abstraction allows both live transport (`TransportHandle`) and
/// export timeline (`ExportTimeline`) to be used interchangeably by
/// nodes like `AutomationLane` that need beat position information.
pub trait TransportReader: Send + Sync {
    /// Get current beat position.
    fn current_beat(&self) -> f64;

    /// Check if loop is enabled.
    fn is_loop_enabled(&self) -> bool;

    /// Get loop range (start, end) in beats, if enabled.
    fn get_loop_range(&self) -> Option<(f64, f64)>;

    /// Check if transport is playing.
    fn is_playing(&self) -> bool;

    /// Check if recording is active.
    fn is_recording(&self) -> bool;

    /// Check if in preroll count-in.
    fn is_in_preroll(&self) -> bool;
}
