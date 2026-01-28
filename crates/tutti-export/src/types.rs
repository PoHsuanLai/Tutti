//! Render job types for instruction-driven offline rendering
//!
//! These types define what to render without any IR dependency.
//! The frontend converts high-level objects (tracks, clips, beats) to
//! these low-level instructions with pre-computed sample positions.

use std::collections::HashMap;

/// A complete render job specification
///
/// Contains all information needed to render audio offline.
/// All timing is in samples (pre-computed by frontend).
#[derive(Debug, Clone)]
pub struct RenderJob {
    /// Output sample rate
    pub sample_rate: u32,
    /// Total length to render in samples
    pub length_samples: usize,
    /// Tracks to render
    pub tracks: Vec<RenderTrack>,
    /// Master bus settings
    pub master: RenderMaster,
}

impl RenderJob {
    /// Create a new render job
    pub fn new(sample_rate: u32, length_samples: usize) -> Self {
        Self {
            sample_rate,
            length_samples,
            tracks: Vec::new(),
            master: RenderMaster::default(),
        }
    }

    /// Add a track to render
    pub fn with_track(mut self, track: RenderTrack) -> Self {
        self.tracks.push(track);
        self
    }

    /// Set master bus settings
    pub fn with_master(mut self, master: RenderMaster) -> Self {
        self.master = master;
        self
    }

    /// Get duration in seconds
    pub fn duration_seconds(&self) -> f64 {
        self.length_samples as f64 / self.sample_rate as f64
    }
}

/// A track to render
#[derive(Debug, Clone)]
pub struct RenderTrack {
    /// Track index (for identification)
    pub index: usize,
    /// Synth notes to render
    pub notes: Vec<RenderNote>,
    /// Audio clips to mix in
    pub audio_clips: Vec<RenderAudioClip>,
    /// Pattern/drum triggers
    pub pattern_triggers: Vec<RenderPatternTrigger>,
    /// Track volume (0.0 - 1.0+)
    pub volume: f32,
    /// Track pan (-1.0 = left, 0.0 = center, 1.0 = right)
    pub pan: f32,
    /// Whether track is muted
    pub muted: bool,
    /// Whether track is soloed
    pub soloed: bool,
    /// Include track effects in render
    pub include_effects: bool,
}

impl RenderTrack {
    /// Create a new render track
    pub fn new(index: usize) -> Self {
        Self {
            index,
            notes: Vec::new(),
            audio_clips: Vec::new(),
            pattern_triggers: Vec::new(),
            volume: 1.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            include_effects: true,
        }
    }

    /// Add a note to render
    pub fn with_note(mut self, note: RenderNote) -> Self {
        self.notes.push(note);
        self
    }

    /// Add an audio clip
    pub fn with_audio_clip(mut self, clip: RenderAudioClip) -> Self {
        self.audio_clips.push(clip);
        self
    }

    /// Add a pattern trigger
    pub fn with_pattern_trigger(mut self, trigger: RenderPatternTrigger) -> Self {
        self.pattern_triggers.push(trigger);
        self
    }

    /// Set track volume
    pub fn with_volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }

    /// Set track pan
    pub fn with_pan(mut self, pan: f32) -> Self {
        self.pan = pan;
        self
    }

    /// Set muted state
    pub fn with_muted(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }

    /// Set soloed state
    pub fn with_soloed(mut self, soloed: bool) -> Self {
        self.soloed = soloed;
        self
    }
}

/// A synth note to render
#[derive(Debug, Clone)]
pub struct RenderNote {
    /// Index of the synth builder to use
    pub synth_index: usize,
    /// MIDI note number (0-127)
    pub midi_note: u8,
    /// Velocity (0-127)
    pub velocity: u8,
    /// Start position in samples
    pub start_sample: usize,
    /// Duration in samples
    pub duration_samples: usize,
    /// Optional synth parameters
    pub params: Option<HashMap<String, f32>>,
}

/// An audio clip to mix in
#[derive(Debug, Clone)]
pub struct RenderAudioClip {
    /// Start position in samples (can be negative for pre-roll)
    pub start_sample: isize,
    /// Interleaved stereo audio data [L, R, L, R, ...]
    pub audio_data: Vec<f32>,
    /// Clip gain
    pub gain: f32,
    /// Whether the audio is mono (single channel)
    pub mono: bool,
}

impl RenderAudioClip {
    /// Create a new stereo audio clip
    pub fn stereo(start_sample: isize, audio_data: Vec<f32>, gain: f32) -> Self {
        Self {
            start_sample,
            audio_data,
            gain,
            mono: false,
        }
    }

    /// Create a new mono audio clip
    pub fn mono(start_sample: isize, audio_data: Vec<f32>, gain: f32) -> Self {
        Self {
            start_sample,
            audio_data,
            gain,
            mono: true,
        }
    }

    /// Get the length in samples (per channel)
    pub fn length_samples(&self) -> usize {
        if self.mono {
            self.audio_data.len()
        } else {
            self.audio_data.len() / 2
        }
    }
}

/// A pattern trigger (drum hit, sample trigger)
#[derive(Debug, Clone)]
pub struct RenderPatternTrigger {
    /// Trigger position in samples
    pub start_sample: usize,
    /// What to trigger
    pub source: PatternTriggerSource,
    /// Trigger velocity/gain
    pub velocity: f32,
}

/// Source for a pattern trigger
#[derive(Debug, Clone)]
pub enum PatternTriggerSource {
    /// Trigger a sample by path
    Sample {
        /// Path to sample file
        path: String,
    },
    /// Trigger a synth
    Synth {
        /// Synth builder index
        synth_index: usize,
        /// MIDI note to play
        midi_note: u8,
        /// Duration in samples
        duration_samples: usize,
        /// Optional parameters
        params: Option<HashMap<String, f32>>,
    },
}

/// Master bus settings
#[derive(Debug, Clone)]
pub struct RenderMaster {
    /// Master volume
    pub volume: f32,
    /// Include master effects
    pub include_effects: bool,
    /// Tail time in samples (for reverb decay)
    pub tail_samples: usize,
}

impl Default for RenderMaster {
    fn default() -> Self {
        Self {
            volume: 1.0,
            include_effects: true,
            tail_samples: 0,
        }
    }
}

impl RenderMaster {
    /// Set master volume
    pub fn with_volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }

    /// Set tail time in seconds
    pub fn with_tail_seconds(mut self, seconds: f64, sample_rate: u32) -> Self {
        self.tail_samples = (seconds * sample_rate as f64) as usize;
        self
    }

    /// Set tail time in samples
    pub fn with_tail_samples(mut self, samples: usize) -> Self {
        self.tail_samples = samples;
        self
    }
}
