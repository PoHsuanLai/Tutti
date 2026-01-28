pub(crate) mod fsm;
pub(crate) mod manager;
pub(crate) mod metronome;
pub(crate) mod position;
pub(crate) mod tempo_map;
mod clock;

// Re-export essential types
pub use manager::{MotionState, TransportManager};
pub use metronome::{Metronome, MetronomeMode};
pub use tempo_map::{TempoMap, TimeSignature, BBT};
pub use clock::{
    TransportClock,
    AutomationReaderInput,
    AutomationEnvelopeFn,
    curves as automation_curves,
};
