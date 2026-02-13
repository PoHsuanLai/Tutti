#[cfg(any(feature = "wav", feature = "flac"))]
mod dither;
#[cfg(any(feature = "wav", feature = "flac"))]
mod loudness;
mod resample;

#[cfg(any(feature = "wav", feature = "flac"))]
pub(crate) use dither::{apply_dither, DitherState};
#[cfg(any(feature = "wav", feature = "flac"))]
pub(crate) use loudness::{normalize_loudness, normalize_peak};
#[cfg(any(feature = "wav", feature = "flac"))]
pub(crate) use resample::resample_stereo;
pub use resample::ResampleQuality;

#[cfg(any(feature = "wav", feature = "flac"))]
pub(crate) use tutti_core::analyze_loudness;

#[cfg(any(feature = "wav", feature = "flac"))]
use crate::options::{ExportOptions, NormalizationMode};
#[cfg(any(feature = "wav", feature = "flac"))]
use crate::Result;

/// Pipeline order: resample -> normalize -> dither.
#[cfg(any(feature = "wav", feature = "flac"))]
pub(crate) fn process_audio(
    left: &[f32],
    right: &[f32],
    options: &ExportOptions,
    output_sample_rate: u32,
) -> Result<(Vec<f32>, Vec<f32>)> {
    let mut left_proc = left.to_vec();
    let mut right_proc = right.to_vec();

    if let Some(target_rate) = options.sample_rate {
        if target_rate != options.source_sample_rate {
            let (l, r) = resample_stereo(
                &left_proc,
                &right_proc,
                options.source_sample_rate,
                target_rate,
                options.resample_quality,
            )?;
            left_proc = l;
            right_proc = r;
        }
    }

    match options.normalization {
        NormalizationMode::None => {}
        NormalizationMode::Peak(target_db) => {
            normalize_peak(&mut left_proc, &mut right_proc, target_db);
        }
        NormalizationMode::Loudness {
            target_lufs,
            true_peak_dbtp,
        } => {
            let current = analyze_loudness(&left_proc, &right_proc, output_sample_rate);
            normalize_loudness(
                &mut left_proc,
                &mut right_proc,
                current.integrated_lufs,
                target_lufs,
                true_peak_dbtp,
            );
        }
    }

    if options.dither != crate::options::DitherType::None {
        let mut state = DitherState::new(options.dither);
        apply_dither(
            &mut left_proc,
            &mut right_proc,
            options.bit_depth.bits(),
            &mut state,
        );
    }

    Ok((left_proc, right_proc))
}

/// Convert stereo to mono by averaging channels.
#[cfg(any(feature = "wav", feature = "flac"))]
pub(crate) fn stereo_to_mono(left: &[f32], right: &[f32]) -> Vec<f32> {
    left.iter()
        .zip(right.iter())
        .map(|(l, r)| (l + r) * 0.5)
        .collect()
}
