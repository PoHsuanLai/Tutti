mod automation_reader;
mod click;
mod clock;
mod export_timeline;
pub(crate) mod fsm;
mod handle;
pub(crate) mod manager;
pub(crate) mod position;
pub mod sync;
pub(crate) mod tempo_map;

pub use automation_reader::{AutomationEnvelopeFn, AutomationReaderInput};
pub use click::{click, ClickNode, ClickSettings, ClickState, MetronomeMode};
pub use clock::TransportClock;
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
    fn current_beat(&self) -> f64;
    fn is_loop_enabled(&self) -> bool;
    fn get_loop_range(&self) -> Option<(f64, f64)>;
    fn is_playing(&self) -> bool;
    fn is_recording(&self) -> bool;
    fn is_in_preroll(&self) -> bool;
    fn tempo(&self) -> f32;
}
