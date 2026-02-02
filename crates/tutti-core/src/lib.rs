//! Audio graph runtime with FunDSP, transport, and metering.
//!
//! ```ignore
//! let system = TuttiSystem::builder().build()?;
//! ```

// Always no_std + alloc (only enable std for CPAL audio I/O)
#![no_std]

#[macro_use]
extern crate alloc;

// Macros are defined in other modules and automatically available

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
pub use fundsp::sequencer::{EventId, Fade, ReplayMode, Sequencer};
pub use fundsp::signal::SignalFrame;
pub use fundsp::{Sample, F32, F64};

// FunDSP file I/O and DSP utilities (for tutti-sampler)
pub use fundsp::fft::{inverse_fft, real_fft};
pub use fundsp::math::Complex32;
pub use fundsp::setting::Setting;
pub use fundsp::wave::Wave;

// Essential types for Net usage
pub use fundsp::net::{NodeId, Source};
pub use fundsp::realnet::NetBackend;
pub use net_frontend::{Connection, NodeInfo, TuttiNet};

// Node registry for dynamic node creation
pub use registry::{
    get_param, get_param_or, NodeConstructor, NodeParamValue, NodeParams, NodeRegistry,
    NodeRegistryError, ParamConvert,
};

// MIDI support (requires "midi" feature)
#[cfg(feature = "midi")]
pub use midi::{AsMidiAudioUnit, MidiAudioUnit, MidiEvent, MidiRegistry};

// DSP units (PDC is the only one in tutti-core, other DSP nodes are in tutti-dsp)
pub use pdc::PdcDelayUnit;

// Transport types
pub use transport::{
    automation_curves, AutomationEnvelopeFn, AutomationReaderInput, Metronome, MetronomeHandle,
    MetronomeMode, MotionState, TempoMap, TimeSignature, TransportClock, TransportHandle,
    TransportManager, BBT,
};

// Metering types
pub use metering::{
    AtomicAmplitude, AtomicStereoAnalysis, CpuMeter, CpuMetrics, MeteringManager,
    StereoAnalysisSnapshot,
};

// PDC types
pub use pdc::{DelayBuffer, PdcManager, PdcState};

// Lock-free primitives
pub use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};
pub use lockfree::{AtomicDouble, AtomicFlag, AtomicFloat};

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
pub(crate) mod compat;
pub(crate) mod lockfree;
pub(crate) mod metering;
mod net_frontend;

#[cfg(feature = "std")]
pub(crate) mod output;

pub(crate) mod pdc;
pub mod registry;
pub(crate) mod transport;

#[cfg(feature = "midi")]
pub mod midi;

#[cfg(feature = "neural")]
pub mod neural;
