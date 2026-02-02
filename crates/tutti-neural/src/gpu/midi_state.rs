//! Per-voice MIDI state accumulator
//!
//! Tracks cumulative MIDI state (note, velocity, pitch bend, CCs, pressure)
//! and converts it to a feature vector for neural inference.
//!
//! This is intentionally decoupled from the inference engine — nodes own
//! their `MidiState` and pass feature vectors to the engine.

/// Number of features produced by `MidiState::to_features()`
pub const MIDI_FEATURE_COUNT: usize = 12;

/// Per-voice MIDI state
///
/// Accumulates MIDI events into a coherent state snapshot.
/// Call `apply()` for each incoming event, then `to_features()`
/// to get the feature vector for inference.
///
/// ## Feature layout
/// ```text
/// [0]  pitch_hz           - f0 with pitch bend applied (Hz)
/// [1]  loudness           - velocity * expression [0, 1]
/// [2]  pitch_bend         - normalized bend [-1, 1]
/// [3]  mod_wheel          - CC1 [0, 1]
/// [4]  brightness         - CC74 / MPE slide [0, 1]
/// [5]  expression         - CC11 [0, 1]
/// [6]  channel_pressure   - aftertouch [0, 1]
/// [7]  sustain            - CC64 on/off (0 or 1)
/// [8]  note_number        - raw MIDI note [0, 127]
/// [9]  velocity_raw       - raw velocity normalized [0, 1]
/// [10] bend_range         - semitones (default 2)
/// [11] reserved
/// ```
#[derive(Debug, Clone)]
pub struct MidiState {
    /// Current MIDI note number (0-127), None if no note active
    pub note: Option<u8>,
    /// Current velocity (0-127)
    pub velocity: u8,
    /// Pitch bend normalized to [-1.0, 1.0]
    pub pitch_bend: f32,
    /// Pitch bend range in semitones (default: 2)
    pub pitch_bend_range: f32,
    /// CC1: Mod wheel [0.0, 1.0]
    pub mod_wheel: f32,
    /// CC74: Brightness / MPE slide [0.0, 1.0]
    pub brightness: f32,
    /// CC11: Expression [0.0, 1.0]
    pub expression: f32,
    /// Channel pressure (aftertouch) [0.0, 1.0]
    pub channel_pressure: f32,
    /// CC64: Sustain pedal
    pub sustain: bool,
}

impl Default for MidiState {
    fn default() -> Self {
        Self {
            note: None,
            velocity: 0,
            pitch_bend: 0.0,
            pitch_bend_range: 2.0,
            mod_wheel: 0.0,
            brightness: 0.5,
            expression: 1.0,
            channel_pressure: 0.0,
            sustain: false,
        }
    }
}

impl MidiState {
    /// Update state from a MIDI event.
    ///
    /// Returns `true` if the event should trigger new inference
    /// (note on/off). Returns `false` for continuous controllers
    /// (CC, pitch bend, pressure) which only update state.
    pub fn apply(&mut self, event: &tutti_midi_io::MidiEvent) -> bool {
        use midi_msg::ChannelVoiceMsg;

        match event.msg {
            ChannelVoiceMsg::NoteOn { note, velocity } if velocity > 0 => {
                self.note = Some(note);
                self.velocity = velocity;
                true
            }
            ChannelVoiceMsg::NoteOn { velocity: 0, .. } | ChannelVoiceMsg::NoteOff { .. } => {
                self.velocity = 0;
                true
            }
            ChannelVoiceMsg::PitchBend { bend } => {
                // midi_msg: 14-bit unsigned (0–16383), center = 8192
                self.pitch_bend = (bend as f32 - 8192.0) / 8192.0;
                false
            }
            ChannelVoiceMsg::ChannelPressure { pressure } => {
                self.channel_pressure = pressure as f32 / 127.0;
                false
            }
            ChannelVoiceMsg::PolyPressure { note, pressure } => {
                if self.note == Some(note) {
                    self.channel_pressure = pressure as f32 / 127.0;
                }
                false
            }
            ChannelVoiceMsg::ControlChange { control } => {
                self.apply_cc(control);
                false
            }
            _ => false,
        }
    }

    fn apply_cc(&mut self, control: midi_msg::ControlChange) {
        use midi_msg::ControlChange;
        if let ControlChange::CC { control: cc, value } = control {
            let norm = value as f32 / 127.0;
            match cc {
                1 => self.mod_wheel = norm,
                11 => self.expression = norm,
                64 => self.sustain = value >= 64,
                74 => self.brightness = norm,
                _ => {}
            }
        }
    }

    /// Pitch in Hz, with pitch bend applied.
    pub fn pitch_hz(&self) -> f32 {
        let note = self.note.unwrap_or(60) as f32;
        let bent = note + self.pitch_bend * self.pitch_bend_range;
        440.0 * 2.0_f32.powf((bent - 69.0) / 12.0)
    }

    /// Loudness combining velocity and expression.
    pub fn loudness(&self) -> f32 {
        (self.velocity as f32 / 127.0) * self.expression
    }

    /// Convert current state to a fixed-size feature vector.
    pub fn to_features(&self) -> [f32; MIDI_FEATURE_COUNT] {
        [
            self.pitch_hz(),
            self.loudness(),
            self.pitch_bend,
            self.mod_wheel,
            self.brightness,
            self.expression,
            self.channel_pressure,
            if self.sustain { 1.0 } else { 0.0 },
            self.note.unwrap_or(0) as f32,
            self.velocity as f32 / 127.0,
            self.pitch_bend_range,
            0.0, // reserved
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_state() {
        let state = MidiState::default();
        assert_eq!(state.note, None);
        assert_eq!(state.velocity, 0);
        assert_eq!(state.pitch_bend, 0.0);
        assert_eq!(state.loudness(), 0.0);
    }

    #[test]
    fn test_note_on() {
        let mut state = MidiState::default();
        let event = tutti_midi_io::MidiEvent::note_on_builder(69, 100)
            .channel(0)
            .offset(0)
            .build(); // A4
        assert!(state.apply(&event)); // should trigger inference

        assert_eq!(state.note, Some(69));
        assert_eq!(state.velocity, 100);
        // A4 = 440 Hz (no bend)
        assert!((state.pitch_hz() - 440.0).abs() < 0.1);
        // loudness = (100/127) * 1.0 expression
        assert!((state.loudness() - 100.0 / 127.0).abs() < 0.01);
    }

    #[test]
    fn test_note_off() {
        let mut state = MidiState::default();
        state.apply(
            &tutti_midi_io::MidiEvent::note_on_builder(60, 80)
                .channel(0)
                .offset(0)
                .build(),
        );
        let triggered = state.apply(
            &tutti_midi_io::MidiEvent::note_off_builder(60)
                .channel(0)
                .offset(0)
                .build(),
        );

        assert!(triggered);
        assert_eq!(state.velocity, 0);
        assert_eq!(state.loudness(), 0.0);
        // note is still remembered (for release phase)
        assert_eq!(state.note, Some(60));
    }

    #[test]
    fn test_pitch_bend() {
        let mut state = MidiState::default();
        state.apply(
            &tutti_midi_io::MidiEvent::note_on_builder(69, 100)
                .channel(0)
                .offset(0)
                .build(),
        ); // A4

        // Bend fully up: bend=16383 → normalized=1.0 → +2 semitones (default range)
        let bend_event = tutti_midi_io::MidiEvent::bend_builder(16383)
            .channel(0)
            .offset(0)
            .build();
        let triggered = state.apply(&bend_event);
        assert!(!triggered); // pitch bend doesn't trigger inference

        assert!((state.pitch_bend - 1.0).abs() < 0.01);
        // A4 + 2 semitones = B4 ≈ 493.88 Hz
        assert!((state.pitch_hz() - 493.88).abs() < 1.0);

        // Bend center: bend=8192 → normalized=0.0
        state.apply(
            &tutti_midi_io::MidiEvent::bend_builder(8192)
                .channel(0)
                .offset(0)
                .build(),
        );
        assert!(state.pitch_bend.abs() < 0.01);
        assert!((state.pitch_hz() - 440.0).abs() < 0.1);
    }

    #[test]
    fn test_cc_mod_wheel() {
        let mut state = MidiState::default();
        let event = tutti_midi_io::MidiEvent::cc_builder(1, 64)
            .channel(0)
            .offset(0)
            .build();
        let triggered = state.apply(&event);

        assert!(!triggered);
        assert!((state.mod_wheel - 64.0 / 127.0).abs() < 0.01);
    }

    #[test]
    fn test_cc_expression() {
        let mut state = MidiState::default();
        state.apply(
            &tutti_midi_io::MidiEvent::note_on_builder(60, 100)
                .channel(0)
                .offset(0)
                .build(),
        );

        // Expression at 50%
        state.apply(
            &tutti_midi_io::MidiEvent::cc_builder(11, 64)
                .channel(0)
                .offset(0)
                .build(),
        );
        let expected = (100.0 / 127.0) * (64.0 / 127.0);
        assert!((state.loudness() - expected).abs() < 0.01);
    }

    #[test]
    fn test_cc_sustain() {
        let mut state = MidiState::default();

        // Sustain on (CC64 >= 64)
        state.apply(
            &tutti_midi_io::MidiEvent::cc_builder(64, 127)
                .channel(0)
                .offset(0)
                .build(),
        );
        assert!(state.sustain);

        // Sustain off (CC64 < 64)
        state.apply(
            &tutti_midi_io::MidiEvent::cc_builder(64, 0)
                .channel(0)
                .offset(0)
                .build(),
        );
        assert!(!state.sustain);
    }

    #[test]
    fn test_channel_pressure() {
        let mut state = MidiState::default();
        let event = tutti_midi_io::MidiEvent::aftertouch_builder(100)
            .channel(0)
            .offset(0)
            .build();
        state.apply(&event);

        assert!((state.channel_pressure - 100.0 / 127.0).abs() < 0.01);
    }

    #[test]
    fn test_to_features_layout() {
        let mut state = MidiState::default();
        state.apply(
            &tutti_midi_io::MidiEvent::note_on_builder(60, 100)
                .channel(0)
                .offset(0)
                .build(),
        );

        let features = state.to_features();
        assert_eq!(features.len(), MIDI_FEATURE_COUNT);

        // [0] pitch_hz — C4 ≈ 261.63
        assert!((features[0] - 261.63).abs() < 1.0);
        // [1] loudness
        assert!(features[1] > 0.0);
        // [8] note number
        assert_eq!(features[8], 60.0);
        // [9] velocity normalized
        assert!((features[9] - 100.0 / 127.0).abs() < 0.01);
    }
}
