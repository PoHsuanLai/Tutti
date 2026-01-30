//! GPU-accelerated neural audio synthesis and effects
//!
//! This crate provides neural audio processing using GPU inference (Burn ML framework).
//! All neural processors implement `AudioUnit` and use lock-free queues for RT-safety.
//!
//! ## Architecture
//!
//! - **Synthesis** - Neural synthesizers (DDSP, WaveRNN, neural vocoders)
//!   - GPU inference generates control parameters (pitch, amplitude, filters)
//!   - Lock-free queues transfer from inference thread → audio thread
//!   - Audio thread renders DSP using FunDSP (no GPU blocking in RT path)
//!
//! - **Effects** - Neural audio effects (amp sims, compressors, reverbs)
//!   - Direct audio processing on audio thread (must be RT-safe)
//!   - GPU batching for multiple instances
//!
//! ## Available Models
//!
//! - **DDSP** - Differentiable Digital Signal Processing
//!   - Harmonic plus noise synthesis
//!   - Learned synthesis parameters
//!
//! - **Neural Vocoder** - Voice synthesis from mel-spectrograms
//!   - WaveRNN architecture
//!   - Real-time capable
//!
//! ## Usage
//!
//! ```rust,ignore
//! use tutti_neural::NeuralSystem;
//!
//! let neural = NeuralSystem::builder()
//!     .sample_rate(44100.0)
//!     .buffer_size(512)
//!     .build()?;
//!
//! // Load ONNX model
//! let model = neural.load_synth_model("violin.onnx")?;
//!
//! // Create voice (returns AudioUnit)
//! let voice = neural.synth().build_voice(&model)?;
//!
//! // Add to audio graph
//! engine.graph(|net| {
//!     let node = net.add(voice);
//!     net.pipe_output(node);
//! });
//! ```
//!
//! ## RT-Safety
//!
//! Neural inference runs on a separate thread and never blocks the audio thread.
//! Results are transferred via lock-free SPSC queues with bounded latency.

pub mod error;
pub use error::{Error, Result};

mod system;
pub use system::{
    EffectHandle, GpuInfo, NeuralModel, NeuralSystem, NeuralSystemBuilder, SynthHandle,
};

pub use gpu::{InferenceConfig, ModelType, VoiceId};
pub use tutti_core::neural::{BatchingStrategy, NeuralNodeManager};
pub use tutti_core::AudioUnit;

pub mod model;

pub(crate) mod backend;
pub(crate) mod effects;
pub(crate) mod gpu;
pub(crate) mod synthesis;
