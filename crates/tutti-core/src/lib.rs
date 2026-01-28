//! Audio graph runtime with FunDSP, transport, and metering.
//!
//! ```ignore
//! let system = TuttiSystem::builder().build()?;
//! ```

pub mod error;
pub use error::{Error, Result};

pub mod config;

// Main entry point
mod system;
pub use system::{TuttiSystem, TuttiSystemBuilder};

// FunDSP DSP toolkit
pub mod dsp {
    pub use fundsp::prelude::*;
}

// Essential FunDSP types
pub use fundsp::biquad::{
    LinkwitzRileyCrossover, LinkwitzRileyHighpass, LinkwitzRileyLowpass, LrOrder,
};
pub use fundsp::prelude::{
    lr_crossover, lr_crossover_hz, lr_highpass, lr_highpass_hz, lr_lowpass, lr_lowpass_hz,
};
pub use fundsp::prelude::{shared, AudioUnit, BufferMut, BufferRef, Shared};
pub use fundsp::{Sample, F32, F64};
pub use fundsp::sequencer::{EventId, Fade, ReplayMode, Sequencer};
pub use fundsp::signal::SignalFrame;

// FunDSP file I/O and DSP utilities (for tutti-sampler)
pub use fundsp::fft::{inverse_fft, real_fft};
pub use fundsp::math::Complex32;
pub use fundsp::setting::Setting;
pub use fundsp::wave::Wave;

// Essential types for Net usage
pub use fundsp::net::{NodeId, Source};
pub use fundsp::realnet::NetBackend;
pub use net_frontend::TuttiNet;

// MIDI support (requires "midi" feature)
#[cfg(feature = "midi")]
pub use midi::{AsMidiAudioUnit, MidiAudioUnit, MidiEvent};

// DSP units (PDC is the only one in tutti-core, other DSP nodes are in tutti-dsp)
pub use pdc::PdcDelayUnit;

// Transport types
pub use transport::{
    automation_curves, AutomationEnvelopeFn, AutomationReaderInput, Metronome, MetronomeMode,
    MotionState, TempoMap, TimeSignature, TransportClock, TransportManager, BBT,
};

// Metering types
pub use metering::{
    AtomicAmplitude, AtomicStereoAnalysis, CpuMeter, CpuMetrics, MeteringManager,
    StereoAnalysisSnapshot,
};

// PDC types
pub use pdc::{DelayBuffer, PdcManager, PdcState};

// Lock-free primitives
pub use lockfree::{AtomicDouble, AtomicFlag, AtomicFloat};
pub use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};

// Neural audio integration (graph analysis & traits)
#[cfg(feature = "neural")]
pub use neural::{
    ArcNeuralEffectBuilder,
    ArcNeuralSynthBuilder,
    // Graph optimization
    BatchingStrategy,
    GraphAnalyzer,
    NeuralEffectBuilder,
    NeuralModelId,
    // Node manager
    NeuralNodeInfo,
    NeuralNodeManager,
    // Builder traits
    NeuralSynthBuilder,
    SharedNeuralNodeManager,
};

pub type VoiceId = u64;

// Module declarations
pub(crate) mod callback;
pub(crate) mod lockfree;
pub(crate) mod metering;
mod net_frontend;
pub(crate) mod output;
pub(crate) mod pdc;
pub(crate) mod transport;

#[cfg(feature = "midi")]
pub mod midi;

#[cfg(feature = "neural")]
pub mod neural;
