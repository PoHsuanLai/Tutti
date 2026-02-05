//! Metronome click AudioUnit - generates click sounds synced to transport.
//!
//! This unit should be added to the audio graph and will output click sounds
//! when enabled. It reads transport state via shared atomics.

use crate::compat::{Arc, Vec};
use crate::{AtomicFloat, AtomicU32, AtomicU8, Ordering};
use fundsp::audionode::AudioNode;
use fundsp::prelude::*;

/// Metronome operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MetronomeMode {
    /// Metronome is disabled.
    #[default]
    Off,
    /// Only play during preroll count-in.
    PrerollOnly,
    /// Only play while recording.
    RecordingOnly,
    /// Always play when transport is running.
    Always,
}

impl From<u8> for MetronomeMode {
    fn from(value: u8) -> Self {
        match value {
            0 => MetronomeMode::Off,
            1 => MetronomeMode::PrerollOnly,
            2 => MetronomeMode::RecordingOnly,
            3 => MetronomeMode::Always,
            _ => MetronomeMode::Off,
        }
    }
}

/// Shared state for the click generator, readable from the audio thread.
/// Updated by the UI thread via atomic operations.
pub struct ClickState {
    /// Current beat position (read from transport)
    pub current_beat: Arc<crate::AtomicDouble>,
    /// Whether transport is paused
    pub paused: Arc<crate::AtomicFlag>,
    /// Whether recording is active
    pub recording: Arc<crate::AtomicFlag>,
    /// Whether in preroll count-in
    pub in_preroll: Arc<crate::AtomicFlag>,
    /// Click volume (0.0 - 1.0)
    pub volume: AtomicFloat,
    /// Accent every N beats (0 = no accent)
    pub accent_every: AtomicU32,
    /// Metronome mode
    pub mode: AtomicU8,
}

impl ClickState {
    pub fn new(
        current_beat: Arc<crate::AtomicDouble>,
        paused: Arc<crate::AtomicFlag>,
        recording: Arc<crate::AtomicFlag>,
        in_preroll: Arc<crate::AtomicFlag>,
    ) -> Self {
        Self {
            current_beat,
            paused,
            recording,
            in_preroll,
            volume: AtomicFloat::new(0.5),
            accent_every: AtomicU32::new(4),
            mode: AtomicU8::new(MetronomeMode::Off as u8),
        }
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume.set(volume.clamp(0.0, 1.0));
    }

    pub fn volume(&self) -> f32 {
        self.volume.get()
    }

    pub fn set_accent_every(&self, beats: u32) {
        self.accent_every.store(beats, Ordering::Release);
    }

    pub fn accent_every(&self) -> u32 {
        self.accent_every.load(Ordering::Acquire)
    }

    pub fn set_mode(&self, mode: MetronomeMode) {
        self.mode.store(mode as u8, Ordering::Release);
    }

    pub fn mode(&self) -> MetronomeMode {
        MetronomeMode::from(self.mode.load(Ordering::Acquire))
    }
}

/// Click generator AudioNode.
/// Outputs stereo click sounds synced to the transport beat.
#[derive(Clone)]
pub struct ClickNode {
    state: Arc<ClickState>,
    sample_rate: f64,
    /// Pre-rendered normal click samples (mono, will be output to both channels)
    click_normal: Vec<f32>,
    /// Pre-rendered accent click samples
    click_accent: Vec<f32>,
    /// Current playback position in click buffer (0 = not playing)
    click_pos: usize,
    /// Whether current click is accented
    is_accent: bool,
    /// Last beat we triggered a click on
    last_click_beat: i64,
}

impl ClickNode {
    pub fn new(state: Arc<ClickState>, sample_rate: f64) -> Self {
        let click_normal = Self::generate_click(sample_rate, false);
        let click_accent = Self::generate_click(sample_rate, true);

        Self {
            state,
            sample_rate,
            click_normal,
            click_accent,
            click_pos: 0,
            is_accent: false,
            last_click_beat: -1,
        }
    }

    fn generate_click(sample_rate: f64, is_accent: bool) -> Vec<f32> {
        let click_duration = 0.03; // 30ms
        let num_samples = (sample_rate * click_duration) as usize;

        let mut samples = Vec::with_capacity(num_samples);

        // Higher frequency for accented beats
        let freq = if is_accent { 1200.0 } else { 1000.0 };
        let accent_volume = if is_accent { 1.0 } else { 0.7 };

        for i in 0..num_samples {
            let t = i as f64 / sample_rate;

            // Envelope: quick attack, short sustain, decay
            let env = if t < 0.001 {
                t / 0.001 // Attack
            } else if t < 0.02 {
                1.0 // Sustain
            } else {
                1.0 - (t - 0.02) / 0.01 // Release
            };

            // Sine wave click
            let phase = 2.0 * core::f64::consts::PI * freq * t;
            let sample = (phase.sin() * env * accent_volume) as f32;
            samples.push(sample);
        }

        samples
    }

    fn is_accent_beat(&self, beat: i64) -> bool {
        let accent_every = self.state.accent_every();
        if accent_every == 0 {
            return false;
        }
        (beat as u32) % accent_every == 0
    }
}

impl AudioNode for ClickNode {
    const ID: u64 = 0x436c69636b_u64; // "Click"

    type Inputs = U0;
    type Outputs = U2; // Stereo output

    #[inline]
    fn tick(&mut self, _input: &Frame<f32, Self::Inputs>) -> Frame<f32, Self::Outputs> {
        // Check if metronome is enabled based on mode and transport state
        let mode = self.state.mode();
        let is_playing = !self.state.paused.get();
        let is_recording = self.state.recording.get();
        let is_in_preroll = self.state.in_preroll.get();

        let should_play = match mode {
            MetronomeMode::Off => false,
            MetronomeMode::Always => is_playing,
            MetronomeMode::PrerollOnly => is_in_preroll,
            MetronomeMode::RecordingOnly => is_recording && !is_in_preroll,
        };

        if !should_play {
            // Reset state when disabled
            self.click_pos = 0;
            self.last_click_beat = -1;
            return [0.0, 0.0].into();
        }

        // Check for beat boundary crossing
        let current_beat = self.state.current_beat.get();
        let beat_int = current_beat.floor() as i64;

        if beat_int > self.last_click_beat {
            // New beat - start a click
            self.last_click_beat = beat_int;
            self.click_pos = 0;
            self.is_accent = self.is_accent_beat(beat_int);
        }

        // Output click sample if playing
        let click_buffer = if self.is_accent {
            &self.click_accent
        } else {
            &self.click_normal
        };

        if self.click_pos < click_buffer.len() {
            let sample = click_buffer[self.click_pos] * self.state.volume();
            self.click_pos += 1;
            [sample, sample].into()
        } else {
            [0.0, 0.0].into()
        }
    }

    fn reset(&mut self) {
        self.click_pos = 0;
        self.last_click_beat = -1;
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        if (self.sample_rate - sample_rate).abs() > 0.1 {
            self.sample_rate = sample_rate;
            // Regenerate click buffers for new sample rate
            self.click_normal = Self::generate_click(sample_rate, false);
            self.click_accent = Self::generate_click(sample_rate, true);
        }
    }
}

/// Create a click generator unit.
///
/// Note: The metronome is automatically mixed into output when using TuttiSystem.
/// You typically don't need to call this directly - just use:
/// ```ignore
/// engine.transport().metronome().always();
/// ```
pub fn click(state: Arc<ClickState>, sample_rate: f64) -> An<ClickNode> {
    An(ClickNode::new(state, sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_click_state() -> (
        Arc<crate::AtomicDouble>,
        Arc<crate::AtomicFlag>,
        Arc<ClickState>,
    ) {
        let current_beat = Arc::new(crate::AtomicDouble::new(0.0));
        let paused = Arc::new(crate::AtomicFlag::new(true));
        let recording = Arc::new(crate::AtomicFlag::new(false));
        let in_preroll = Arc::new(crate::AtomicFlag::new(false));
        let state = Arc::new(ClickState::new(
            current_beat.clone(),
            paused.clone(),
            recording,
            in_preroll,
        ));
        (current_beat, paused, state)
    }

    #[test]
    fn test_click_node_creation() {
        let (_, _, state) = make_click_state();
        let node = ClickNode::new(state, 44100.0);
        assert!(!node.click_normal.is_empty());
        assert!(!node.click_accent.is_empty());
    }

    #[test]
    fn test_click_node_silent_when_paused() {
        let (_, _, state) = make_click_state();
        state.set_mode(MetronomeMode::Always);

        let mut node = ClickNode::new(state, 44100.0);

        // Should be silent when paused
        let output = node.tick(&Frame::default());
        assert_eq!(output[0], 0.0);
        assert_eq!(output[1], 0.0);
    }

    #[test]
    fn test_click_node_plays_on_beat() {
        let (current_beat, paused, state) = make_click_state();
        paused.set(false); // Not paused
        state.set_mode(MetronomeMode::Always);
        state.set_volume(1.0);

        let mut node = ClickNode::new(state, 44100.0);

        // First few samples have attack envelope ramping up from 0
        // Tick several times to get past the zero-crossing
        let mut found_nonzero = false;
        for _ in 0..100 {
            let output = node.tick(&Frame::default());
            if output[0] != 0.0 || output[1] != 0.0 {
                found_nonzero = true;
                break;
            }
        }
        assert!(found_nonzero, "Click should produce non-zero output");

        // Advance to beat 1 - should start new click
        current_beat.set(1.0);
        found_nonzero = false;
        for _ in 0..100 {
            let output = node.tick(&Frame::default());
            if output[0] != 0.0 || output[1] != 0.0 {
                found_nonzero = true;
                break;
            }
        }
        assert!(
            found_nonzero,
            "Click should produce non-zero output on new beat"
        );
    }

    #[test]
    fn test_accent_pattern() {
        let (_, paused, state) = make_click_state();
        paused.set(false);
        state.set_accent_every(4);

        let node = ClickNode::new(state, 44100.0);

        // Beat 0 should be accented
        assert!(node.is_accent_beat(0));
        // Beat 1 should not
        assert!(!node.is_accent_beat(1));
        // Beat 4 should be accented
        assert!(node.is_accent_beat(4));
    }

    #[test]
    fn test_preroll_only_mode() {
        let current_beat = Arc::new(crate::AtomicDouble::new(0.0));
        let paused = Arc::new(crate::AtomicFlag::new(false));
        let recording = Arc::new(crate::AtomicFlag::new(false));
        let in_preroll = Arc::new(crate::AtomicFlag::new(false));
        let state = Arc::new(ClickState::new(
            current_beat,
            paused,
            recording.clone(),
            in_preroll.clone(),
        ));
        state.set_mode(MetronomeMode::PrerollOnly);
        state.set_volume(1.0);

        let mut node = ClickNode::new(state, 44100.0);

        // Should be silent when not in preroll
        let output = node.tick(&Frame::default());
        assert_eq!(output[0], 0.0);

        // Enable preroll - should play
        in_preroll.set(true);
        node.reset();
        let mut found_nonzero = false;
        for _ in 0..100 {
            let output = node.tick(&Frame::default());
            if output[0] != 0.0 {
                found_nonzero = true;
                break;
            }
        }
        assert!(found_nonzero, "Click should play during preroll");
    }

    #[test]
    fn test_recording_only_mode() {
        let current_beat = Arc::new(crate::AtomicDouble::new(0.0));
        let paused = Arc::new(crate::AtomicFlag::new(false));
        let recording = Arc::new(crate::AtomicFlag::new(false));
        let in_preroll = Arc::new(crate::AtomicFlag::new(false));
        let state = Arc::new(ClickState::new(
            current_beat,
            paused,
            recording.clone(),
            in_preroll.clone(),
        ));
        state.set_mode(MetronomeMode::RecordingOnly);
        state.set_volume(1.0);

        let mut node = ClickNode::new(state, 44100.0);

        // Should be silent when not recording
        let output = node.tick(&Frame::default());
        assert_eq!(output[0], 0.0);

        // Enable recording - should play
        recording.set(true);
        node.reset();
        let mut found_nonzero = false;
        for _ in 0..100 {
            let output = node.tick(&Frame::default());
            if output[0] != 0.0 {
                found_nonzero = true;
                break;
            }
        }
        assert!(found_nonzero, "Click should play during recording");

        // In preroll while recording - should NOT play (preroll takes priority)
        in_preroll.set(true);
        node.reset();
        let output = node.tick(&Frame::default());
        assert_eq!(
            output[0], 0.0,
            "Click should not play during preroll in RecordingOnly mode"
        );
    }
}
