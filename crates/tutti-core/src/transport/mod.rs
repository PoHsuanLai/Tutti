mod clock;
pub(crate) mod fsm;
mod handle;
pub(crate) mod manager;
pub(crate) mod metronome;
pub(crate) mod position;
pub mod sync;
pub(crate) mod tempo_map;

// Re-export essential types
pub use clock::{
    curves as automation_curves, AutomationEnvelopeFn, AutomationReaderInput, TransportClock,
};
pub use handle::{MetronomeHandle, TransportHandle};
pub use manager::{MotionState, TransportManager};
pub use metronome::{Metronome, MetronomeMode};
pub use sync::{SmpteFrameRate, SyncSnapshot, SyncSource, SyncState, SyncStatus};
pub use tempo_map::{TempoMap, TimeSignature, BBT};
