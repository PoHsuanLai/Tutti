//! Audio metering and analysis.
//!
//! - Real-time metering: `MeteringManager` for live amplitude, LUFS, CPU tracking
//! - Batch analysis: `analyze_loudness`, `analyze_true_peak` for offline processing

mod amplitude;
mod cpu;
mod handle;
mod loudness;
mod manager;
mod math;
mod rt;
mod stereo;

pub use amplitude::AtomicAmplitude;
pub use cpu::{CpuMeter, CpuMetrics};
pub use handle::MeteringHandle;
pub use loudness::{analyze_loudness, analyze_true_peak, LoudnessResult};
pub use manager::MeteringManager;
#[cfg(feature = "std")]
pub(crate) use rt::MeteringContext;
pub use stereo::{AtomicStereoAnalysis, StereoAnalysisSnapshot};
