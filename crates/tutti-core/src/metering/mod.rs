//! Audio metering and analysis.
//!
//! - Real-time metering: `MeteringManager` for live amplitude, LUFS, CPU tracking
//! - Batch analysis: `analyze_loudness`, `analyze_true_peak` for offline processing

mod amplitude;
mod cpu;
mod loudness;
mod manager;
mod math;
mod stereo;

pub use amplitude::AtomicAmplitude;
pub use cpu::{CpuMeter, CpuMetrics};
pub use loudness::{analyze_loudness, analyze_true_peak, LoudnessResult};
pub use manager::MeteringManager;
pub use stereo::{AtomicStereoAnalysis, StereoAnalysisSnapshot};
