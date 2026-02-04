//! Recording session management for MIDI and audio input.
//! - Audio callback: Minimal interaction (only for audio input via Butler)

use crate::butler::{ButlerCommand, CaptureBuffer, CaptureId, FlushRequest};
use crate::recording::{
    PunchEvent, RecordedData, RecordingBuffer, RecordingConfig, RecordingMode, RecordingSession,
    RecordingSource, RecordingState, XRunEvent,
};
use dashmap::DashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Manages audio and MIDI recording sessions
///
/// Handles per-channel recording sessions with disk I/O via butler thread.
pub struct RecordingManager {
    /// Recording sessions (per-channel)
    /// Key = channel index, sparse storage via DashMap
    sessions: Arc<dashmap::DashMap<usize, Arc<RecordingSession>>>,

    /// Butler command sender (for audio input disk recording)
    butler_tx: crossbeam_channel::Sender<ButlerCommand>,

    /// Sample rate
    sample_rate: f64,
}

impl RecordingManager {
    /// Create new recording manager
    pub(crate) fn new(
        _initial_track_count: usize,
        butler_tx: crossbeam_channel::Sender<ButlerCommand>,
        sample_rate: f64,
    ) -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
            butler_tx,
            sample_rate,
        }
    }

    /// Resize session storage for more tracks (no-op for DashMap)
    pub fn resize(&self, _new_track_count: usize) {}

    /// Start recording on a track
    ///
    /// # Arguments
    /// * `channel_index` - Track to record on
    /// * `source` - Recording source (MIDI, audio, pattern)
    /// * `mode` - Recording mode (replace, overdub, etc.)
    /// * `current_beat` - Current transport beat position (from TransportManager)
    pub fn start_recording(
        &self,
        channel_index: usize,
        source: RecordingSource,
        mode: RecordingMode,
        current_beat: f64,
    ) -> crate::error::Result<()> {
        let config = RecordingConfig {
            channel_index,
            source,
            mode,
            ..Default::default()
        };

        let session = RecordingSession::new(config, self.sample_rate, current_beat);

        if source == RecordingSource::AudioInput {
            self.setup_audio_input_capture(&session)?;
        }

        self.sessions.insert(channel_index, Arc::new(session));

        Ok(())
    }

    /// Stop recording on a track
    pub fn stop_recording(&self, channel_index: usize) -> crate::error::Result<RecordedData> {
        // Get session and clone Arc to avoid holding DashMap lock
        let session = self
            .sessions
            .get(&channel_index)
            .ok_or_else(|| {
                crate::error::Error::Recording(format!(
                    "No recording session on channel {}",
                    channel_index
                ))
            })
            .map(|s| Arc::clone(&s))?;

        // Mark session as stopped
        session.set_state(RecordingState::Stopped);
        session.deactivate();

        // Get punch and XRun events from session
        let punch_events = session.get_punch_events();
        let xrun_events = session.get_xrun_events();

        // Extract recorded data based on source
        let data = match session.source() {
            RecordingSource::MidiInput => {
                let buffer = Arc::unwrap_or_clone(
                    session.swap_buffer(RecordingBuffer::new(0.0, self.sample_rate)),
                );
                RecordedData::Midi {
                    buffer,
                    punch_events,
                    xrun_events,
                }
            }
            RecordingSource::AudioInput => {
                self.stop_audio_input_capture(&session, punch_events, xrun_events)?
            }
            RecordingSource::InternalAudio => {
                let buffer = Arc::unwrap_or_clone(
                    session.swap_buffer(RecordingBuffer::new(0.0, self.sample_rate)),
                );
                RecordedData::InternalAudio {
                    buffer,
                    punch_events,
                    xrun_events,
                }
            }
            RecordingSource::Pattern => {
                let buffer = Arc::unwrap_or_clone(
                    session.swap_buffer(RecordingBuffer::new(0.0, self.sample_rate)),
                );
                RecordedData::Pattern {
                    buffer,
                    punch_events,
                    xrun_events,
                }
            }
        };

        self.sessions.remove(&channel_index);

        Ok(data)
    }

    /// Check if track is recording
    pub fn is_recording(&self, channel_index: usize) -> bool {
        self.sessions
            .get(&channel_index)
            .map(|s| s.is_active())
            .unwrap_or(false)
    }

    /// Get recording state for a track
    pub fn get_recording_state(&self, channel_index: usize) -> Option<RecordingState> {
        self.sessions.get(&channel_index).map(|s| s.get_state())
    }

    /// Update preroll for all armed sessions (call from audio thread)
    ///
    /// Returns number of sessions that completed preroll
    pub fn update_prerolls(&self, delta_beats: f64) -> usize {
        let mut completed_count = 0;
        for session in self.sessions.iter() {
            if session.get_state() == RecordingState::Armed && session.update_preroll(delta_beats) {
                completed_count += 1;
            }
        }
        completed_count
    }

    /// Get all sessions currently in preroll
    pub fn preroll_sessions(&self) -> Vec<Arc<RecordingSession>> {
        self.sessions
            .iter()
            .filter(|entry| entry.is_in_preroll())
            .map(|entry| Arc::clone(entry.value()))
            .collect()
    }

    /// Check if any session is in preroll
    pub fn has_preroll_sessions(&self) -> bool {
        self.sessions.iter().any(|entry| entry.is_in_preroll())
    }

    /// Check if any session is actively recording (not in preroll)
    pub fn has_active_recording(&self) -> bool {
        use crate::recording::RecordingState;
        self.sessions.iter().any(|entry| {
            let state = entry.get_state();
            matches!(
                state,
                RecordingState::Recording | RecordingState::Overdubbing
            )
        })
    }

    /// Process punch events for all sessions based on current transport position.
    ///
    /// Call this method each audio callback to handle automatic punch-in/out.
    /// Sessions with punch ranges configured will automatically transition states.
    ///
    /// # Arguments
    /// * `current_beat` - Current transport position in beats
    /// * `sample_position` - Optional sample position for sample-accurate punch
    ///
    /// # Returns
    /// Number of state transitions that occurred
    pub fn process_punch_all(&self, current_beat: f64, sample_position: Option<u64>) -> usize {
        let mut transitions = 0;
        for session in self.sessions.iter() {
            if session
                .process_punch(current_beat, sample_position)
                .is_some()
            {
                transitions += 1;
            }
        }
        transitions
    }

    /// Set record safe mode for a channel.
    ///
    /// When record safe is enabled, the session will not record any data
    /// even if it's in Recording state. Useful for "safe" playback during rehearsal.
    pub fn set_record_safe(&self, channel_index: usize, safe: bool) -> crate::error::Result<()> {
        let session = self.sessions.get(&channel_index).ok_or_else(|| {
            crate::error::Error::Recording(format!(
                "No recording session on channel {}",
                channel_index
            ))
        })?;
        session.set_record_safe(safe);
        Ok(())
    }

    /// Check if record safe mode is enabled for a channel.
    pub fn is_record_safe(&self, channel_index: usize) -> bool {
        self.sessions
            .get(&channel_index)
            .map(|s| s.is_record_safe())
            .unwrap_or(false)
    }

    /// Record an XRun (buffer underrun/overrun) event for a channel.
    ///
    /// This should be called when the audio callback detects a buffer issue.
    pub fn record_xrun(
        &self,
        channel_index: usize,
        sample_position: u64,
        beat: Option<f64>,
        xrun_type: crate::recording::session::XRunType,
    ) -> crate::error::Result<()> {
        let session = self.sessions.get(&channel_index).ok_or_else(|| {
            crate::error::Error::Recording(format!(
                "No recording session on channel {}",
                channel_index
            ))
        })?;
        session.record_xrun(sample_position, beat, xrun_type);
        Ok(())
    }

    /// Get XRun count for a channel.
    pub fn xrun_count(&self, channel_index: usize) -> usize {
        self.sessions
            .get(&channel_index)
            .map(|s| s.xrun_count())
            .unwrap_or(0)
    }

    /// Check if any session has XRun events.
    pub fn has_xruns(&self) -> bool {
        self.sessions.iter().any(|entry| entry.has_xruns())
    }

    /// Record MIDI note on event (beat-only, for backwards compatibility)
    pub fn record_midi_note_on(
        &self,
        channel_index: usize,
        note: u8,
        velocity: u8,
        beat: f64,
        channel: u8,
    ) -> crate::error::Result<()> {
        self.record_midi_note_on_with_sample(channel_index, note, velocity, beat, channel, None)
    }

    /// Record MIDI note on event with sample position (sample-accurate)
    pub fn record_midi_note_on_with_sample(
        &self,
        channel_index: usize,
        note: u8,
        velocity: u8,
        beat: f64,
        channel: u8,
        sample_position: Option<u64>,
    ) -> crate::error::Result<()> {
        let session = self.sessions.get(&channel_index).ok_or_else(|| {
            crate::error::Error::Recording(format!(
                "No recording session on channel {}",
                channel_index
            ))
        })?;

        if !session.is_active() {
            return Err(crate::error::Error::Recording(format!(
                "Recording session on channel {} is not active",
                channel_index
            )));
        }

        if session.source() != RecordingSource::MidiInput {
            return Err(crate::error::Error::Recording(format!(
                "Channel {} is not recording MIDI input (source: {:?})",
                channel_index,
                session.source()
            )));
        }

        session.with_buffer(|buffer| {
            if let Some(sample) = sample_position {
                buffer.record_midi_note_on_with_sample(note, velocity, beat, channel, sample);
            } else {
                buffer.record_midi_note_on(note, velocity, beat, channel);
            }
        });

        Ok(())
    }

    /// Record MIDI note off event (beat-only, for backwards compatibility)
    pub fn record_midi_note_off(
        &self,
        channel_index: usize,
        note: u8,
        beat: f64,
        channel: u8,
    ) -> crate::error::Result<()> {
        self.record_midi_note_off_with_sample(channel_index, note, beat, channel, None)
    }

    /// Record MIDI note off event with sample position (sample-accurate)
    pub fn record_midi_note_off_with_sample(
        &self,
        channel_index: usize,
        note: u8,
        beat: f64,
        channel: u8,
        sample_position: Option<u64>,
    ) -> crate::error::Result<()> {
        let session = self.sessions.get(&channel_index).ok_or_else(|| {
            crate::error::Error::Recording(format!(
                "No recording session on channel {}",
                channel_index
            ))
        })?;

        if !session.is_active() {
            return Err(crate::error::Error::Recording(format!(
                "Recording session on channel {} is not active",
                channel_index
            )));
        }

        if session.source() != RecordingSource::MidiInput {
            return Err(crate::error::Error::Recording(format!(
                "Channel {} is not recording MIDI input (source: {:?})",
                channel_index,
                session.source()
            )));
        }

        session.with_buffer(|buffer| {
            if let Some(sample) = sample_position {
                buffer.record_midi_note_off_with_sample(note, beat, channel, sample);
            } else {
                buffer.record_midi_note_off(note, beat, channel);
            }
        });

        Ok(())
    }

    /// Record MIDI CC event (beat-only, for backwards compatibility)
    pub fn record_midi_cc(
        &self,
        channel_index: usize,
        channel: u8,
        controller: u8,
        value: u8,
        beat: f64,
    ) -> crate::error::Result<()> {
        self.record_midi_cc_with_sample(channel_index, channel, controller, value, beat, None)
    }

    /// Record MIDI CC event with sample position (sample-accurate)
    pub fn record_midi_cc_with_sample(
        &self,
        channel_index: usize,
        channel: u8,
        controller: u8,
        value: u8,
        beat: f64,
        sample_position: Option<u64>,
    ) -> crate::error::Result<()> {
        let session = self.sessions.get(&channel_index).ok_or_else(|| {
            crate::error::Error::Recording(format!(
                "No recording session on channel {}",
                channel_index
            ))
        })?;

        if !session.is_active() {
            return Err(crate::error::Error::Recording(format!(
                "Recording session on channel {} is not active",
                channel_index
            )));
        }

        if session.source() != RecordingSource::MidiInput {
            return Err(crate::error::Error::Recording(format!(
                "Channel {} is not recording MIDI input (source: {:?})",
                channel_index,
                session.source()
            )));
        }

        session.with_buffer(|buffer| {
            if let Some(sample) = sample_position {
                buffer.record_midi_cc_with_sample(channel, controller, value, beat, sample);
            } else {
                buffer.record_midi_cc(channel, controller, value, beat);
            }
        });

        Ok(())
    }

    /// Record pattern trigger event
    pub fn record_pattern_trigger(
        &self,
        channel_index: usize,
        symbol: String,
        step: u32,
        beat: f64,
        velocity: f32,
    ) -> crate::error::Result<()> {
        let session = self.sessions.get(&channel_index).ok_or_else(|| {
            crate::error::Error::Recording(format!(
                "No recording session on channel {}",
                channel_index
            ))
        })?;

        if !session.is_active() {
            return Err(crate::error::Error::Recording(format!(
                "Recording session on channel {} is not active",
                channel_index
            )));
        }

        if session.source() != RecordingSource::Pattern {
            return Err(crate::error::Error::Recording(format!(
                "Channel {} is not recording patterns (source: {:?})",
                channel_index,
                session.source()
            )));
        }

        session.with_buffer(|buffer| {
            buffer.record_pattern_trigger(symbol, step, beat, velocity);
        });

        Ok(())
    }

    /// Get recording session for a track (lock-free)
    #[inline]
    pub fn get_session(&self, channel_index: usize) -> Option<Arc<RecordingSession>> {
        self.sessions.get(&channel_index).map(|r| Arc::clone(&*r))
    }

    /// Get capture buffer producer for a track (lock-free, for audio input stream)
    #[inline]
    pub fn get_capture_producer(
        &self,
        channel_index: usize,
    ) -> Option<Arc<crate::butler::CaptureBufferProducer>> {
        self.sessions.get(&channel_index)?.get_capture_producer()
    }

    /// Setup audio input capture for a session
    fn setup_audio_input_capture(&self, session: &RecordingSession) -> crate::error::Result<()> {
        let capture_id = CaptureId::generate();

        let file_path = PathBuf::from(format!(
            "recordings/track_{}_{}.wav",
            session.channel_index(),
            capture_id.0
        ));

        let (producer, consumer) = CaptureBuffer::new(
            capture_id,
            file_path.clone(),
            self.sample_rate,
            1000.0, // 1 second buffer
        );

        self.butler_tx
            .send(ButlerCommand::RegisterCapture {
                capture_id,
                consumer,
                file_path: file_path.clone(),
                sample_rate: self.sample_rate,
                channels: 2,
            })
            .map_err(|e| {
                crate::error::Error::Recording(format!("Failed to send RegisterCapture: {}", e))
            })?;

        session.set_capture_id(capture_id);
        session.set_recording_file(file_path);
        session.set_capture_producer(producer);

        Ok(())
    }

    /// Stop audio input capture for a session
    fn stop_audio_input_capture(
        &self,
        session: &RecordingSession,
        punch_events: Vec<PunchEvent>,
        xrun_events: Vec<XRunEvent>,
    ) -> crate::error::Result<RecordedData> {
        let capture_id = session.get_capture_id().ok_or_else(|| {
            crate::error::Error::Recording("No capture ID for audio input recording".to_string())
        })?;

        let file_path = session
            .get_recording_file()
            .ok_or_else(|| crate::error::Error::Recording("No recording file path".to_string()))?;

        self.butler_tx
            .send(ButlerCommand::Flush(FlushRequest::new(capture_id)))
            .map_err(|e| crate::error::Error::Recording(format!("Failed to send Flush: {}", e)))?;

        self.butler_tx
            .send(ButlerCommand::RemoveCapture(capture_id))
            .map_err(|e| {
                crate::error::Error::Recording(format!("Failed to send RemoveCapture: {}", e))
            })?;

        let duration_seconds = match hound::WavReader::open(&file_path) {
            Ok(reader) => {
                let spec = reader.spec();
                let sample_count = reader.len();
                sample_count as f64 / (spec.sample_rate as f64 * spec.channels as f64)
            }
            Err(_) => 0.0,
        };

        Ok(RecordedData::Audio {
            file_path,
            duration_seconds,
            punch_events,
            xrun_events,
        })
    }
}

impl Default for RecordingManager {
    fn default() -> Self {
        let (tx, _rx) = crossbeam_channel::unbounded();
        Self::new(0, tx, 44100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_manager() -> RecordingManager {
        let (tx, _rx) = crossbeam_channel::unbounded();
        RecordingManager::new(8, tx, 44100.0)
    }

    #[test]
    fn test_recording_manager_creation() {
        let manager = create_test_manager();
        assert!(!manager.is_recording(0));
        assert!(!manager.is_recording(7));
        assert_eq!(manager.get_recording_state(0), None);
    }

    #[test]
    fn test_resize() {
        let manager = create_test_manager();
        manager.resize(16);
        assert!(!manager.is_recording(0));
    }

    #[test]
    fn test_start_stop_midi_recording() {
        let manager = create_test_manager();

        manager
            .start_recording(0, RecordingSource::MidiInput, RecordingMode::Replace, 0.0)
            .unwrap();

        assert!(manager.is_recording(0));
        assert_eq!(manager.get_recording_state(0), Some(RecordingState::Armed));

        // Record some MIDI events
        manager.record_midi_note_on(0, 60, 100, 0.0, 0).unwrap();
        manager.record_midi_note_off(0, 60, 1.0, 0).unwrap();

        let data = manager.stop_recording(0).unwrap();
        assert!(!manager.is_recording(0));

        match data {
            RecordedData::Midi { buffer, .. } => {
                assert_eq!(buffer.midi_events.len(), 1);
                assert_eq!(buffer.midi_events[0].note, 60);
                assert_eq!(buffer.midi_events[0].duration, 1.0);
            }
            _ => panic!("Expected MIDI data"),
        }
    }

    #[test]
    fn test_pattern_recording() {
        let manager = create_test_manager();

        manager
            .start_recording(0, RecordingSource::Pattern, RecordingMode::Replace, 0.0)
            .unwrap();

        let _ = manager.record_pattern_trigger(0, "bd".to_string(), 0, 0.0, 1.0);
        let _ = manager.record_pattern_trigger(0, "sn".to_string(), 4, 1.0, 0.8);

        let data = manager.stop_recording(0).unwrap();

        match data {
            RecordedData::Pattern { buffer, .. } => {
                assert_eq!(buffer.pattern_events.len(), 2);
                assert_eq!(buffer.pattern_events[0].symbol, "bd");
                assert_eq!(buffer.pattern_events[1].step, 4);
            }
            _ => panic!("Expected Pattern data"),
        }
    }

    #[test]
    fn test_concurrent_recording() {
        let manager = create_test_manager();

        manager
            .start_recording(0, RecordingSource::MidiInput, RecordingMode::Replace, 0.0)
            .unwrap();
        manager
            .start_recording(1, RecordingSource::Pattern, RecordingMode::Overdub, 0.0)
            .unwrap();

        assert!(manager.is_recording(0));
        assert!(manager.is_recording(1));
        assert!(!manager.is_recording(2));

        manager.record_midi_note_on(0, 60, 100, 0.0, 0).unwrap();
        let _ = manager.record_pattern_trigger(1, "bd".to_string(), 0, 0.0, 1.0);

        let data0 = manager.stop_recording(0).unwrap();
        assert!(!manager.is_recording(0));
        assert!(manager.is_recording(1));

        match data0 {
            RecordedData::Midi { buffer, .. } => {
                assert_eq!(buffer.active_note_count(), 1);
            }
            _ => panic!("Expected MIDI data"),
        }
    }

    #[test]
    fn test_record_safe_mode() {
        let manager = create_test_manager();

        manager
            .start_recording(0, RecordingSource::MidiInput, RecordingMode::Replace, 0.0)
            .unwrap();

        assert!(!manager.is_record_safe(0));

        manager.set_record_safe(0, true).unwrap();
        assert!(manager.is_record_safe(0));

        manager.set_record_safe(0, false).unwrap();
        assert!(!manager.is_record_safe(0));

        assert!(!manager.is_record_safe(99));
    }

    #[test]
    fn test_xrun_tracking() {
        use crate::recording::session::XRunType;

        let manager = create_test_manager();

        manager
            .start_recording(0, RecordingSource::MidiInput, RecordingMode::Replace, 0.0)
            .unwrap();

        assert_eq!(manager.xrun_count(0), 0);
        assert!(!manager.has_xruns());

        manager
            .record_xrun(0, 44100, Some(1.0), XRunType::Underrun)
            .unwrap();

        assert_eq!(manager.xrun_count(0), 1);
        assert!(manager.has_xruns());

        manager
            .record_xrun(0, 88200, Some(2.0), XRunType::Overrun)
            .unwrap();

        assert_eq!(manager.xrun_count(0), 2);
    }

    #[test]
    fn test_punch_all_sessions() {
        let manager = create_test_manager();

        let config = crate::recording::RecordingConfig::builder()
            .channel(0)
            .source(RecordingSource::MidiInput)
            .punch_range(4.0, 8.0)
            .build();

        let session = crate::recording::RecordingSession::new(config, 44100.0, 0.0);
        manager.sessions.insert(0, std::sync::Arc::new(session));

        assert_eq!(manager.get_recording_state(0), Some(RecordingState::Armed));
        assert_eq!(manager.process_punch_all(0.0, None), 0);

        assert_eq!(manager.process_punch_all(4.0, None), 1);
        assert_eq!(
            manager.get_recording_state(0),
            Some(RecordingState::Recording)
        );

        assert_eq!(manager.process_punch_all(6.0, None), 0);

        assert_eq!(manager.process_punch_all(8.0, None), 1);
        assert_eq!(
            manager.get_recording_state(0),
            Some(RecordingState::Stopped)
        );
    }

    #[test]
    fn test_punch_events_in_recorded_data() {
        let manager = create_test_manager();

        let config = crate::recording::RecordingConfig::builder()
            .channel(0)
            .source(RecordingSource::MidiInput)
            .punch_range(4.0, 8.0)
            .build();

        let session = crate::recording::RecordingSession::new(config, 44100.0, 0.0);
        manager.sessions.insert(0, std::sync::Arc::new(session));

        manager.process_punch_all(4.0, Some(176400));
        manager.process_punch_all(8.0, Some(352800));

        let data = manager.stop_recording(0).unwrap();

        match data {
            RecordedData::Midi { punch_events, .. } => {
                assert_eq!(punch_events.len(), 2);
                assert!(matches!(
                    punch_events[0],
                    crate::recording::PunchEvent::PunchIn { beat: 4.0, .. }
                ));
                assert!(matches!(
                    punch_events[1],
                    crate::recording::PunchEvent::PunchOut { beat: 8.0, .. }
                ));
            }
            _ => panic!("Expected MIDI data"),
        }
    }
}
