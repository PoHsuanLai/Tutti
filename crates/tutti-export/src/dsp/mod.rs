//! DSP utilities for audio export
//!
//! - Resampling (via rubato)
//! - Dithering
//! - Loudness metering (EBU R128)

mod dither;
mod loudness;
mod resample;

pub use dither::{apply_dither, quantize, DitherState};
pub use loudness::{
    calculate_loudness, calculate_peak, normalize_loudness, normalize_peak, LoudnessResult,
};
pub use resample::{resample_stereo, ResampleQuality};
