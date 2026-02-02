//! MIDI File I/O
//!
//! This module handles Standard MIDI File (SMF) parsing and writing using the `midly` crate.
//! It converts SMF events to our internal `TimedMidiEvent` representation for streaming playback.

use crate::error::{Error, Result};
use midly::{MetaMessage, MidiMessage, Smf, Timing, Track, TrackEventKind};
use std::path::Path;
use tracing::debug;

/// A parsed MIDI file ready for streaming playback
#[derive(Debug, Clone)]
pub struct ParsedMidiFile {
    /// All MIDI events with absolute timestamps in beats
    pub events: Vec<TimedMidiEvent>,

    /// Ticks per quarter note (for tempo conversion)
    pub ticks_per_beat: u16,

    /// Default tempo in BPM (from first tempo event, or 120 if none)
    pub tempo_bpm: f64,

    /// Total duration in beats
    pub duration_beats: f64,
}

/// A MIDI event with absolute timestamp
#[derive(Debug, Clone, Copy)]
pub struct TimedMidiEvent {
    /// Absolute time in beats from start of file
    pub time_beats: f64,

    /// MIDI channel (0-15)
    pub channel: u8,

    /// The actual MIDI event
    pub event: MidiEventType,
}

/// MIDI event types we support for playback
#[derive(Debug, Clone, Copy)]
pub enum MidiEventType {
    /// Note On: note number (0-127), velocity (0-127)
    NoteOn { note: u8, velocity: u8 },

    /// Note Off: note number (0-127), velocity (0-127)
    NoteOff { note: u8, velocity: u8 },

    /// Control Change: controller (0-127), value (0-127)
    ControlChange { controller: u8, value: u8 },

    /// Program Change: program (0-127)
    ProgramChange { program: u8 },

    /// Pitch Bend: value (-8192 to 8191)
    PitchBend { value: i16 },
}

impl ParsedMidiFile {
    /// Load and parse a MIDI file from disk
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let data = std::fs::read(path.as_ref())?;
        Self::parse(&data)
    }

    /// Parse MIDI file from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        let smf = Smf::parse(data)?;

        let ticks_per_beat = match smf.header.timing {
            Timing::Metrical(tpb) => tpb.as_int(),
            Timing::Timecode(_, _) => {
                return Err(Error::MidiUnsupportedTiming);
            }
        };

        debug!(
            "Parsing MIDI file: {} tracks, {} ticks per beat",
            smf.tracks.len(),
            ticks_per_beat
        );

        // Parse all tracks and merge into single event list
        let mut all_events = Vec::new();
        let mut tempo_bpm = 120.0; // Default tempo
        let mut found_tempo = false;

        for track in smf.tracks.iter() {
            let track_events = Self::parse_track(track, ticks_per_beat);

            // Extract tempo from first track (typically track 0 in Format 1)
            if !found_tempo {
                if let Some(tempo) = Self::extract_tempo(track) {
                    tempo_bpm = tempo;
                    found_tempo = true;
                    debug!("Found tempo: {} BPM", tempo_bpm);
                }
            }

            all_events.extend(track_events);
        }

        // Sort events by time
        all_events.sort_by(|a, b| {
            a.time_beats
                .partial_cmp(&b.time_beats)
                .expect("MIDI event time_beats should never be NaN")
        });

        // Calculate total duration
        let duration_beats = all_events.last().map(|e| e.time_beats).unwrap_or(0.0);

        debug!(
            "Parsed {} MIDI events, duration: {:.2} beats",
            all_events.len(),
            duration_beats
        );

        Ok(Self {
            events: all_events,
            ticks_per_beat,
            tempo_bpm,
            duration_beats,
        })
    }

    /// Parse a single track into timed events
    fn parse_track(track: &Track, ticks_per_beat: u16) -> Vec<TimedMidiEvent> {
        let mut events = Vec::new();
        let mut current_tick = 0u64;

        for event in track.iter() {
            // Accumulate delta time
            current_tick += event.delta.as_int() as u64;
            let time_beats = current_tick as f64 / ticks_per_beat as f64;

            // Convert track event to our format
            if let Some(midi_event) = Self::convert_event(&event.kind, time_beats) {
                events.push(midi_event);
            }
        }

        events
    }

    /// Convert midly event to our TimedMidiEvent
    fn convert_event(kind: &TrackEventKind, time_beats: f64) -> Option<TimedMidiEvent> {
        match kind {
            TrackEventKind::Midi { channel, message } => {
                let event_type = match message {
                    MidiMessage::NoteOn { key, vel } => {
                        // Note: velocity 0 is treated as Note Off
                        if vel.as_int() == 0 {
                            MidiEventType::NoteOff {
                                note: key.as_int(),
                                velocity: 0,
                            }
                        } else {
                            MidiEventType::NoteOn {
                                note: key.as_int(),
                                velocity: vel.as_int(),
                            }
                        }
                    }
                    MidiMessage::NoteOff { key, vel } => MidiEventType::NoteOff {
                        note: key.as_int(),
                        velocity: vel.as_int(),
                    },
                    MidiMessage::Controller { controller, value } => MidiEventType::ControlChange {
                        controller: controller.as_int(),
                        value: value.as_int(),
                    },
                    MidiMessage::ProgramChange { program } => MidiEventType::ProgramChange {
                        program: program.as_int(),
                    },
                    MidiMessage::PitchBend { bend } => {
                        // midly represents pitch bend as u14 (0-16383), center = 8192
                        let value = bend.as_int() - 8192;
                        MidiEventType::PitchBend { value }
                    }
                    _ => {
                        // Ignore other MIDI messages for now
                        return None;
                    }
                };

                Some(TimedMidiEvent {
                    time_beats,
                    channel: channel.as_int(),
                    event: event_type,
                })
            }
            _ => {
                // Ignore meta events and sysex for playback
                None
            }
        }
    }

    /// Extract tempo from track (looks for first tempo meta event)
    fn extract_tempo(track: &Track) -> Option<f64> {
        for event in track.iter() {
            if let TrackEventKind::Meta(MetaMessage::Tempo(tempo)) = &event.kind {
                // Tempo is in microseconds per quarter note
                let us_per_qn = tempo.as_int();
                let bpm = 60_000_000.0 / us_per_qn as f64;
                return Some(bpm);
            }
        }
        None
    }

    /// Get events in a time range (for buffering)
    pub fn get_events_in_range(&self, start_beats: f64, end_beats: f64) -> &[TimedMidiEvent] {
        // Binary search for start
        let start_idx = self
            .events
            .binary_search_by(|e| {
                e.time_beats
                    .partial_cmp(&start_beats)
                    .expect("MIDI event time_beats should never be NaN")
            })
            .unwrap_or_else(|idx| idx);

        // Linear search for end (events are sorted)
        let mut end_idx = start_idx;
        while end_idx < self.events.len() && self.events[end_idx].time_beats < end_beats {
            end_idx += 1;
        }

        &self.events[start_idx..end_idx]
    }
}

// REMOVED: convert_to_midi_notes() - uses dawai_document::MidiNote
// This function should be implemented in audio-bevy translation layer

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_midi() {
        // Minimal valid MIDI file (header only)
        let data = [
            // MThd
            0x4D, 0x54, 0x68, 0x64, // Header length (6)
            0x00, 0x00, 0x00, 0x06, // Format 0
            0x00, 0x00, // 1 track
            0x00, 0x01, // 480 ticks per beat
            0x01, 0xE0, // MTrk
            0x4D, 0x54, 0x72, 0x6B, // Track length (4)
            0x00, 0x00, 0x00, 0x04, // End of track
            0x00, 0xFF, 0x2F, 0x00,
        ];

        let result = ParsedMidiFile::parse(&data);
        assert!(result.is_ok());
        let file = result.unwrap();
        assert_eq!(file.ticks_per_beat, 480);
        assert_eq!(file.events.len(), 0); // No note events
    }

    #[test]
    fn test_note_on_velocity_zero() {
        // Test that NoteOn with velocity 0 is converted to NoteOff
        let event_kind = TrackEventKind::Midi {
            channel: 0.into(),
            message: MidiMessage::NoteOn {
                key: 60.into(),
                vel: 0.into(),
            },
        };

        let result = ParsedMidiFile::convert_event(&event_kind, 0.0);
        assert!(result.is_some());

        match result.unwrap().event {
            MidiEventType::NoteOff { note, velocity } => {
                assert_eq!(note, 60);
                assert_eq!(velocity, 0);
            }
            _ => panic!("Expected NoteOff"),
        }
    }
}
