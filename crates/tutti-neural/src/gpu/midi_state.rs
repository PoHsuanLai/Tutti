//! Per-voice MIDI state accumulator for neural inference.

pub const MIDI_FEATURE_COUNT: usize = 12;

/// Per-voice MIDI state converted to feature vectors for inference.
///
/// Feature layout: `[pitch_hz, loudness, pitch_bend, mod_wheel, brightness,
/// expression, channel_pressure, sustain, note_number, velocity_raw, bend_range, reserved]`
#[derive(Debug, Clone)]
pub struct MidiState {
    pub note: Option<u8>,
    pub velocity: u8,
    pub pitch_bend: f32,
    pub pitch_bend_range: f32,
    pub mod_wheel: f32,
    pub brightness: f32,
    pub expression: f32,
    pub channel_pressure: f32,
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
    /// Returns true if the event should trigger new inference (note on/off).
    #[cfg(feature = "midi")]
    pub fn apply(&mut self, event: &tutti_core::midi::MidiEvent) -> bool {
        use tutti_core::midi::ChannelVoiceMsg;

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

    #[cfg(feature = "midi")]
    fn apply_cc(&mut self, control: tutti_core::midi::ControlChange) {
        use tutti_core::midi::ControlChange;
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

    pub fn pitch_hz(&self) -> f32 {
        let note = self.note.unwrap_or(60) as f32;
        let bent = note + self.pitch_bend * self.pitch_bend_range;
        440.0 * 2.0_f32.powf((bent - 69.0) / 12.0)
    }

    pub fn loudness(&self) -> f32 {
        (self.velocity as f32 / 127.0) * self.expression
    }

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
            0.0,
        ]
    }
}

#[cfg(all(test, feature = "midi"))]
mod tests {
    use super::*;
    use tutti_core::midi::MidiEvent;

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
        let event = MidiEvent::note_on_builder(69, 100)
            .channel(0)
            .offset(0)
            .build();
        assert!(state.apply(&event));

        assert_eq!(state.note, Some(69));
        assert_eq!(state.velocity, 100);
        assert!((state.pitch_hz() - 440.0).abs() < 0.1);
        assert!((state.loudness() - 100.0 / 127.0).abs() < 0.01);
    }

    #[test]
    fn test_note_off() {
        let mut state = MidiState::default();
        state.apply(
            &MidiEvent::note_on_builder(60, 80)
                .channel(0)
                .offset(0)
                .build(),
        );
        let triggered = state.apply(&MidiEvent::note_off_builder(60).channel(0).offset(0).build());

        assert!(triggered);
        assert_eq!(state.velocity, 0);
        assert_eq!(state.loudness(), 0.0);
        assert_eq!(state.note, Some(60));
    }

    #[test]
    fn test_pitch_bend() {
        let mut state = MidiState::default();
        state.apply(
            &MidiEvent::note_on_builder(69, 100)
                .channel(0)
                .offset(0)
                .build(),
        );

        let bend_event = MidiEvent::bend_builder(16383).channel(0).offset(0).build();
        let triggered = state.apply(&bend_event);
        assert!(!triggered);

        assert!((state.pitch_bend - 1.0).abs() < 0.01);
        assert!((state.pitch_hz() - 493.88).abs() < 1.0);

        state.apply(&MidiEvent::bend_builder(8192).channel(0).offset(0).build());
        assert!(state.pitch_bend.abs() < 0.01);
        assert!((state.pitch_hz() - 440.0).abs() < 0.1);
    }

    #[test]
    fn test_cc_mod_wheel() {
        let mut state = MidiState::default();
        let event = MidiEvent::cc_builder(1, 64).channel(0).offset(0).build();
        let triggered = state.apply(&event);

        assert!(!triggered);
        assert!((state.mod_wheel - 64.0 / 127.0).abs() < 0.01);
    }

    #[test]
    fn test_cc_expression() {
        let mut state = MidiState::default();
        state.apply(
            &MidiEvent::note_on_builder(60, 100)
                .channel(0)
                .offset(0)
                .build(),
        );

        state.apply(&MidiEvent::cc_builder(11, 64).channel(0).offset(0).build());
        let expected = (100.0 / 127.0) * (64.0 / 127.0);
        assert!((state.loudness() - expected).abs() < 0.01);
    }

    #[test]
    fn test_cc_sustain() {
        let mut state = MidiState::default();

        state.apply(&MidiEvent::cc_builder(64, 127).channel(0).offset(0).build());
        assert!(state.sustain);

        state.apply(&MidiEvent::cc_builder(64, 0).channel(0).offset(0).build());
        assert!(!state.sustain);
    }

    #[test]
    fn test_channel_pressure() {
        let mut state = MidiState::default();
        let event = MidiEvent::aftertouch_builder(100)
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
            &MidiEvent::note_on_builder(60, 100)
                .channel(0)
                .offset(0)
                .build(),
        );

        let features = state.to_features();
        assert_eq!(features.len(), MIDI_FEATURE_COUNT);

        assert!((features[0] - 261.63).abs() < 1.0);
        assert!(features[1] > 0.0);
        assert_eq!(features[8], 60.0);
        assert!((features[9] - 100.0 / 127.0).abs() < 0.01);
    }
}
