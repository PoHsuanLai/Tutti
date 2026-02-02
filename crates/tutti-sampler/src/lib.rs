//! Audio streaming, recording, and sample playback.
//!
//! Provides disk streaming, audio input recording, and time-stretching capabilities
//! for the Tutti audio engine.
//!
//! # Features
//!
//! - **Disk streaming**: Butler thread for asynchronous I/O with ring buffers
//! - **Audio input**: Hardware capture with lock-free MPMC channels
//! - **Recording**: MIDI, audio, and pattern recording with quantization
//! - **Time-stretching**: Real-time pitch and tempo manipulation via phase vocoder
//! - **Automation**: Parameter automation recording and playback
//!
//! # Example
//!
//! ```ignore
//! use tutti_sampler::{SamplerSystem, SamplerUnit, TimeStretchUnit};
//!
//! // High-level streaming API (most common)
//! let sampler = SamplerSystem::new(44100.0).build()?;
//! sampler.stream_file(0, "audio.wav").gain(0.8).start();
//!
//! // Low-level DSP nodes for FunDSP graph integration
//! let unit = SamplerUnit::new(wave);
//! let stretched = TimeStretchUnit::new(Box::new(unit), 44100.0);
//!
//! // Advanced: Recording and automation
//! use tutti_sampler::recording::{RecordingManager, AutomationManager};
//! let recording = RecordingManager::new(44100.0);
//! ```

// Error types
pub mod error;
pub use error::{Error, Result};

// Main high-level API (most common usage)
mod system;
pub use system::{CaptureSession, SamplerSystem, SamplerSystemBuilder};

// Fluent builders
mod stream_builder;
pub use stream_builder::{RecordBuilder, StreamBuilder};

// DSP nodes for FunDSP graph integration
pub use audio_input::{AudioInput, AudioInputBackend};
pub use sampler::{SamplerUnit, StreamingSamplerUnit};
pub use time_stretch::TimeStretchUnit;

// Modules - always compiled (SamplerSystem needs them internally)
pub mod audio_input;
pub mod butler;
pub mod recording;
pub mod sampler;
pub mod time_stretch;
