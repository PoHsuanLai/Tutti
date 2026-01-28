//! Offline renderer for audio export
//!
//! The `OfflineRenderer` processes `RenderJob` instructions to produce
//! audio buffers without any real-time constraints.

use crate::error::{ExportError, Result};
use crate::types::{RenderAudioClip, RenderJob, RenderNote, RenderPatternTrigger, RenderTrack};
use tutti_core::AudioUnit;

/// Progress callback for render operations
pub type RenderProgressCallback = Box<dyn Fn(f32) + Send>;

/// Result of a render operation
#[derive(Debug, Clone)]
pub struct RenderResult {
    /// Left channel audio data
    pub left: Vec<f32>,
    /// Right channel audio data
    pub right: Vec<f32>,
    /// Sample rate of the rendered audio
    pub sample_rate: u32,
    /// Peak level (linear)
    pub peak_level: f32,
    /// Number of samples rendered
    pub length_samples: usize,
}

impl RenderResult {
    /// Get duration in seconds
    pub fn duration_seconds(&self) -> f64 {
        self.length_samples as f64 / self.sample_rate as f64
    }

    /// Get interleaved stereo data [L, R, L, R, ...]
    pub fn interleaved(&self) -> Vec<f32> {
        let mut result = Vec::with_capacity(self.left.len() * 2);
        for i in 0..self.left.len() {
            result.push(self.left[i]);
            result.push(self.right[i]);
        }
        result
    }
}

/// Synth builder function type
///
/// Takes MIDI note, velocity, and optional parameters.
/// Returns a boxed AudioUnit that generates f32 audio.
pub type SynthBuilderFn = Box<
    dyn Fn(u8, u8, Option<&std::collections::HashMap<String, f32>>) -> Box<dyn AudioUnit>
        + Send
        + Sync,
>;

/// Effect builder function type
pub type EffectBuilderFn = Box<dyn Fn() -> Box<dyn AudioUnit> + Send + Sync>;

/// Sample loader function type
///
/// Takes a sample path and returns stereo audio data.
pub type SampleLoaderFn = Box<dyn Fn(&str) -> Result<(Vec<f32>, Vec<f32>)> + Send + Sync>;

/// Offline audio renderer
///
/// Renders `RenderJob` instructions to audio buffers.
/// Framework-agnostic: receives pre-computed sample positions.
pub struct OfflineRenderer {
    sample_rate: u32,
    synth_builders: Vec<SynthBuilderFn>,
    effect_builders: Vec<EffectBuilderFn>,
    sample_loader: Option<SampleLoaderFn>,
}

impl OfflineRenderer {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            synth_builders: Vec::new(),
            effect_builders: Vec::new(),
            sample_loader: None,
        }
    }

    pub fn register_synth(&mut self, builder: SynthBuilderFn) -> usize {
        let index = self.synth_builders.len();
        self.synth_builders.push(builder);
        index
    }

    pub fn register_effect(&mut self, builder: EffectBuilderFn) -> usize {
        let index = self.effect_builders.len();
        self.effect_builders.push(builder);
        index
    }

    /// Set the sample loader function
    pub fn set_sample_loader(&mut self, loader: SampleLoaderFn) {
        self.sample_loader = Some(loader);
    }

    /// Render a job to audio buffers
    ///
    /// # Arguments
    /// * `job` - The render job specification
    /// * `progress` - Optional progress callback (0.0 to 1.0)
    pub fn render(
        &self,
        job: RenderJob,
        progress: Option<RenderProgressCallback>,
    ) -> Result<RenderResult> {
        let total_samples = job.length_samples + job.master.tail_samples;

        // Allocate output buffers
        let mut left = vec![0.0f32; total_samples];
        let mut right = vec![0.0f32; total_samples];

        // Determine which tracks to render (handle solo)
        let has_solo = job.tracks.iter().any(|t| t.soloed);
        let tracks_to_render: Vec<&RenderTrack> = job
            .tracks
            .iter()
            .filter(|t| {
                if t.muted {
                    return false;
                }
                if has_solo && !t.soloed {
                    return false;
                }
                true
            })
            .collect();

        let total_tracks = tracks_to_render.len();

        // Render each track
        for (track_idx, track) in tracks_to_render.iter().enumerate() {
            // Report progress
            if let Some(ref callback) = progress {
                callback(track_idx as f32 / total_tracks as f32);
            }

            // Render track to temporary buffers
            let (track_left, track_right) = self.render_track(track, total_samples)?;

            // Apply track volume and pan, mix into output
            let (left_gain, right_gain) = Self::pan_gains(track.pan, track.volume);

            for i in 0..total_samples {
                left[i] += track_left[i] * left_gain;
                right[i] += track_right[i] * right_gain;
            }
        }

        // Apply master volume
        let master_vol = job.master.volume;
        for i in 0..total_samples {
            left[i] *= master_vol;
            right[i] *= master_vol;
        }

        // Calculate peak level
        let peak_level = left
            .iter()
            .chain(right.iter())
            .map(|s| s.abs())
            .fold(0.0f32, f32::max);

        // Final progress
        if let Some(callback) = progress {
            callback(1.0);
        }

        Ok(RenderResult {
            left,
            right,
            sample_rate: job.sample_rate,
            peak_level,
            length_samples: total_samples,
        })
    }

    /// Render a single track
    fn render_track(
        &self,
        track: &RenderTrack,
        total_samples: usize,
    ) -> Result<(Vec<f32>, Vec<f32>)> {
        let mut left = vec![0.0f32; total_samples];
        let mut right = vec![0.0f32; total_samples];

        // Render synth notes
        for note in &track.notes {
            self.render_note(note, &mut left, &mut right)?;
        }

        // Mix audio clips
        for clip in &track.audio_clips {
            self.mix_audio_clip(clip, &mut left, &mut right);
        }

        // Render pattern triggers
        for trigger in &track.pattern_triggers {
            self.render_pattern_trigger(trigger, &mut left, &mut right)?;
        }

        Ok((left, right))
    }

    fn render_note(&self, note: &RenderNote, left: &mut [f32], right: &mut [f32]) -> Result<()> {
        if note.synth_index >= self.synth_builders.len() {
            return Err(ExportError::Render(format!(
                "Synth index {} not registered",
                note.synth_index
            )));
        }

        let builder = &self.synth_builders[note.synth_index];
        let mut synth = builder(note.midi_note, note.velocity, note.params.as_ref());
        synth.set_sample_rate(self.sample_rate as f64);
        synth.allocate();

        let end_sample = (note.start_sample + note.duration_samples).min(left.len());
        let mut output = [0.0f32; 2];

        for i in note.start_sample..end_sample {
            synth.tick(&[], &mut output);

            if synth.outputs() >= 2 {
                left[i] += output[0];
                right[i] += output[1];
            } else {
                left[i] += output[0];
                right[i] += output[0];
            }
        }

        Ok(())
    }

    /// Mix an audio clip into the output
    fn mix_audio_clip(&self, clip: &RenderAudioClip, left: &mut [f32], right: &mut [f32]) {
        let clip_len = clip.length_samples();

        for i in 0..clip_len {
            let out_idx = clip.start_sample + i as isize;
            if out_idx < 0 || out_idx >= left.len() as isize {
                continue;
            }
            let out_idx = out_idx as usize;

            if clip.mono {
                let sample = clip.audio_data[i] * clip.gain;
                left[out_idx] += sample;
                right[out_idx] += sample;
            } else {
                left[out_idx] += clip.audio_data[i * 2] * clip.gain;
                right[out_idx] += clip.audio_data[i * 2 + 1] * clip.gain;
            }
        }
    }

    /// Render a pattern trigger
    fn render_pattern_trigger(
        &self,
        trigger: &RenderPatternTrigger,
        left: &mut [f32],
        right: &mut [f32],
    ) -> Result<()> {
        match &trigger.source {
            crate::types::PatternTriggerSource::Sample { path } => {
                // Load and mix sample
                if let Some(ref loader) = self.sample_loader {
                    let (sample_left, sample_right) = loader(path)?;
                    let clip = RenderAudioClip {
                        start_sample: trigger.start_sample as isize,
                        audio_data: sample_left
                            .iter()
                            .zip(sample_right.iter())
                            .flat_map(|(l, r)| [*l, *r])
                            .collect(),
                        gain: trigger.velocity,
                        mono: false,
                    };
                    self.mix_audio_clip(&clip, left, right);
                }
            }
            crate::types::PatternTriggerSource::Synth {
                synth_index,
                midi_note,
                duration_samples,
                params,
            } => {
                let note = RenderNote {
                    synth_index: *synth_index,
                    midi_note: *midi_note,
                    velocity: (trigger.velocity * 127.0) as u8,
                    start_sample: trigger.start_sample,
                    duration_samples: *duration_samples,
                    params: params.clone(),
                };
                self.render_note(&note, left, right)?;
            }
        }

        Ok(())
    }

    /// Calculate left/right gains from pan and volume
    ///
    /// Uses equal power panning law.
    fn pan_gains(pan: f32, volume: f32) -> (f32, f32) {
        // pan: -1.0 = full left, 0.0 = center, 1.0 = full right
        let pan_normalized = (pan + 1.0) * 0.5; // 0.0 to 1.0
        let angle = pan_normalized * std::f32::consts::FRAC_PI_2;
        let left_gain = angle.cos() * volume;
        let right_gain = angle.sin() * volume;
        (left_gain, right_gain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_empty_job() {
        let renderer = OfflineRenderer::new(44100);
        let job = RenderJob::new(44100, 44100);

        let result = renderer.render(job, None).unwrap();

        assert_eq!(result.left.len(), 44100);
        assert_eq!(result.right.len(), 44100);
        assert_eq!(result.peak_level, 0.0);
    }

    #[test]
    fn test_pan_gains() {
        let (l, r) = OfflineRenderer::pan_gains(0.0, 1.0);
        assert!((l - r).abs() < 0.01);

        let (l, r) = OfflineRenderer::pan_gains(-1.0, 1.0);
        assert!(l > 0.9);
        assert!(r < 0.1);

        // Full right
        let (l, r) = OfflineRenderer::pan_gains(1.0, 1.0);
        assert!(l < 0.1);
        assert!(r > 0.9);
    }

    #[test]
    fn test_render_result_interleaved() {
        let result = RenderResult {
            left: vec![1.0, 2.0, 3.0],
            right: vec![4.0, 5.0, 6.0],
            sample_rate: 44100,
            peak_level: 6.0,
            length_samples: 3,
        };

        let interleaved = result.interleaved();
        assert_eq!(interleaved, vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
    }

    #[test]
    fn test_render_audio_clip() {
        let renderer = OfflineRenderer::new(44100);

        // Create a job with an audio clip
        // Note: equal power panning at center gives ~0.707 gain per channel
        let clip = RenderAudioClip::stereo(0, vec![0.5, 0.5, 0.3, 0.3], 1.0);
        let track = RenderTrack::new(0).with_audio_clip(clip);
        let job = RenderJob::new(44100, 4).with_track(track);

        let result = renderer.render(job, None).unwrap();

        // With equal power pan at center: output = input * cos(π/4) ≈ input * 0.707
        let expected_l0 = 0.5 * std::f32::consts::FRAC_PI_4.cos();
        let expected_l1 = 0.3 * std::f32::consts::FRAC_PI_4.cos();

        assert!(
            (result.left[0] - expected_l0).abs() < 0.01,
            "left[0]: {} vs {}",
            result.left[0],
            expected_l0
        );
        assert!((result.right[0] - expected_l0).abs() < 0.01);
        assert!((result.left[1] - expected_l1).abs() < 0.01);
    }

    #[test]
    fn test_muted_track() {
        let renderer = OfflineRenderer::new(44100);

        let clip = RenderAudioClip::stereo(0, vec![1.0, 1.0], 1.0);
        let track = RenderTrack::new(0).with_audio_clip(clip).with_muted(true);
        let job = RenderJob::new(44100, 1).with_track(track);

        let result = renderer.render(job, None).unwrap();

        // Muted track should not contribute
        assert_eq!(result.peak_level, 0.0);
    }
}
