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
//! let sampler = SamplerSystem::builder(44100.0).build()?;
//! sampler.stream_file(0, "audio.wav").offset_samples(0).start();
//!
//! // Low-level DSP nodes for FunDSP graph integration
//! let unit = SamplerUnit::new(wave);
//! let stretched = TimeStretchUnit::new(Box::new(unit), 44100.0);
//!
//! // Recording via SamplerSystem
//! let session = sampler.record("output.wav").channels(2).start();
//! ```

pub mod error;
pub use error::{Error, Result};

mod system;
pub use system::{CaptureSession, SamplerSystem, SamplerSystemBuilder};

mod stream_builder;
pub use stream_builder::{RecordBuilder, StreamBuilder};

mod handle;
pub use handle::SamplerHandle;

mod auditioner;
pub use auditioner::Auditioner;

pub use audio_input::{AudioInput, AudioInputBackend};
pub use sampler::{SamplerUnit, StreamingSamplerUnit};
pub use time_stretch::{
    FftSize, GrainSize, TimeStretchAlgorithm, TimeStretchParams, TimeStretchUnit,
};

pub use butler::{
    BufferConfig, CacheStats, CaptureBufferProducer, CaptureId, ChannelStreamState, IOMetrics,
    IOMetricsSnapshot, LruCache, PlayDirection, RegionBufferConsumer, SharedStreamState, Varispeed,
};

mod audio_input;
pub(crate) mod butler;
pub(crate) mod recording;
mod sampler;
mod time_stretch;
