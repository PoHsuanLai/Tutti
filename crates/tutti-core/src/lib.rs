//! Real-time audio engine with DSP graph, transport, and metering.
//!
//! # Primary API
//!
//! - [`TuttiSystem`] / [`TuttiSystemBuilder`]: Main entry point
//! - [`TuttiNet`]: DSP graph manipulation
//! - [`TransportHandle`]: Playback control (play/stop/seek/loop)
//! - [`MeteringManager`]: Audio level monitoring
//! - [`PdcManager`]: Plugin delay compensation
//!
//! # Feature-gated APIs
//!
//! - `"neural"`: [`NeuralNodeManager`], [`BatchingStrategy`] for GPU-accelerated audio
//! - `"midi"`: [`MidiRegistry`], [`MidiEvent`] for MIDI routing
//! - `"std"`: CPAL audio I/O (enabled by default)
//!
//! # Example
//!
//! ```ignore
//! use tutti_core::prelude::*;
//!
//! let system = TuttiSystem::builder().build()?;
//!
//! system.graph(|net| {
//!     let osc = net.add(Box::new(sine_hz(440.0)));
//!     net.pipe_output(osc);
//! });
//!
//! system.transport().play();
//! ```

#![no_std]

#[cfg(feature = "std")]
extern crate std;

#[macro_use]
extern crate alloc;

// Error types
pub mod error;
pub use error::{Error, NodeRegistryError, Result};

// Main entry point
mod system;
pub use system::{TuttiSystem, TuttiSystemBuilder};

// DSP graph
mod net_frontend;
pub use net_frontend::{NodeInfo, TuttiNet};

// Transport
pub(crate) mod transport;
pub use transport::{
    automation_curves, AutomationEnvelopeFn, AutomationReaderInput, Metronome, MetronomeHandle,
    MetronomeMode, MotionState, SmpteFrameRate, SyncSnapshot, SyncSource, SyncState, SyncStatus,
    TempoMap, TimeSignature, TransportClock, TransportHandle, TransportManager, BBT,
};

// Metering
pub(crate) mod metering;
pub use metering::{
    analyze_loudness, analyze_true_peak, AtomicAmplitude, AtomicStereoAnalysis, CpuMeter,
    CpuMetrics, LoudnessResult, MeteringManager, StereoAnalysisSnapshot,
};

// Plugin delay compensation
pub(crate) mod pdc;
pub use pdc::{DelayBuffer, PdcDelayUnit, PdcManager, PdcState};

// Node registry
pub mod registry;
pub use registry::{
    get_param, get_param_or, NodeConstructor, NodeParamValue, NodeParams, NodeRegistry,
    ParamConvert,
};

// Lock-free primitives
pub(crate) mod lockfree;
pub use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};
pub use lockfree::{AtomicDouble, AtomicFlag, AtomicFloat};

// FunDSP re-exports
pub mod dsp {
    //! Re-export of fundsp::prelude for DSP building blocks.
    pub use fundsp::prelude::*;
}

pub use fundsp::biquad::{
    LinkwitzRileyCrossover, LinkwitzRileyHighpass, LinkwitzRileyLowpass, LrOrder,
};
pub use fundsp::fft::{inverse_fft, real_fft};
pub use fundsp::math::Complex32;
pub use fundsp::net::{NodeId, Source};
pub use fundsp::prelude::{
    lr_crossover, lr_crossover_hz, lr_highpass, lr_highpass_hz, lr_lowpass, lr_lowpass_hz, shared,
    AudioUnit, BufferMut, BufferRef, Shared,
};
pub use fundsp::realnet::NetBackend;
pub use fundsp::sequencer::{EventId, Fade, ReplayMode, Sequencer};
pub use fundsp::setting::Setting;
pub use fundsp::signal::SignalFrame;
pub use fundsp::wave::Wave;
pub use fundsp::{Sample, F32, F64};

/// Voice identifier for polyphonic synths.
pub type VoiceId = u64;

// Internal modules
pub(crate) mod compat;

#[cfg(feature = "std")]
pub(crate) mod callback;

#[cfg(feature = "std")]
pub(crate) mod output;

// Feature-gated: MIDI
#[cfg(feature = "midi")]
pub mod midi;

#[cfg(feature = "midi")]
pub use midi::{
    AsMidiAudioUnit, MidiAudioUnit, MidiEvent, MidiInputSource, MidiRegistry, MidiRoute,
    MidiRoutingSnapshot, MidiRoutingTable, NoMidiInput,
};

// Feature-gated: Neural audio
#[cfg(feature = "neural")]
pub mod neural;

#[cfg(feature = "neural")]
pub use neural::{
    ArcNeuralEffectBuilder, ArcNeuralSynthBuilder, BatchingStrategy, NeuralEffectBuilder,
    NeuralModelId, NeuralNodeManager, NeuralSynthBuilder, SharedNeuralNodeManager,
};
