//! Recording session types with lock-free access patterns.

use super::config::{RecordingConfig, RecordingMode, RecordingSource};
use super::events::RecordingBuffer;
use crate::butler::{CaptureBufferProducer, CaptureId};
use arc_swap::ArcSwap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;

/// Recording state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RecordingState {
    /// Waiting to start recording (pre-roll)
    Armed = 0,
    /// Actively recording
    Recording = 1,
    /// Overdubbing (adding to existing content)
    Overdubbing = 2,
    /// Recording stopped
    Stopped = 3,
}

/// Punch event type
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PunchEvent {
    /// Recording started due to punch-in
    PunchIn {
        /// Beat position where punch occurred
        beat: f64,
        /// Sample position (if available)
        sample: Option<u64>,
    },
    /// Recording stopped due to punch-out
    PunchOut {
        /// Beat position where punch occurred
        beat: f64,
        /// Sample position (if available)
        sample: Option<u64>,
    },
}

/// XRun (buffer underrun/overrun) event
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XRunEvent {
    /// Sample position where XRun occurred
    pub sample_position: u64,
    /// Beat position (if available)
    pub beat: Option<f64>,
    /// Type of XRun
    pub xrun_type: XRunType,
}

/// Type of XRun event
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XRunType {
    /// Playback buffer underrun (couldn't read fast enough)
    Underrun,
    /// Capture buffer overrun (couldn't write fast enough)
    Overrun,
}

impl From<u8> for RecordingState {
    fn from(value: u8) -> Self {
        match value {
            0 => RecordingState::Armed,
            1 => RecordingState::Recording,
            2 => RecordingState::Overdubbing,
            3 => RecordingState::Stopped,
            _ => RecordingState::Stopped,
        }
    }
}

/// Recording session for a track.
pub struct RecordingSession {
    state: AtomicU8,
    config: RecordingConfig,
    buffer: ArcSwap<RecordingBuffer>,
    preroll_remaining: AtomicU64,
    capture_id: AtomicU64,
    recording_file: ArcSwap<Option<PathBuf>>,
    capture_producer: ArcSwap<Option<Arc<CaptureBufferProducer>>>,
    is_active: AtomicBool,
    /// Record safe mode - prevents accidental recording
    record_safe: AtomicBool,
    /// Punch events that occurred during this session
    punch_events: ArcSwap<Vec<PunchEvent>>,
    /// XRun events that occurred during this session
    xrun_events: ArcSwap<Vec<XRunEvent>>,
}

impl RecordingSession {
    /// Create new recording session
    pub fn new(config: RecordingConfig, sample_rate: f64, start_beat: f64) -> Self {
        let buffer = RecordingBuffer::new(start_beat, sample_rate);
        let preroll_beats = config.preroll_beats;

        Self {
            state: AtomicU8::new(RecordingState::Armed as u8),
            config,
            buffer: ArcSwap::new(Arc::new(buffer)),
            preroll_remaining: AtomicU64::new(preroll_beats.to_bits()),
            capture_id: AtomicU64::new(0), // 0 = None
            recording_file: ArcSwap::new(Arc::new(None)),
            capture_producer: ArcSwap::new(Arc::new(None)),
            is_active: AtomicBool::new(true),
            record_safe: AtomicBool::new(false),
            punch_events: ArcSwap::new(Arc::new(Vec::new())),
            xrun_events: ArcSwap::new(Arc::new(Vec::new())),
        }
    }

    /// Get current recording state
    pub fn get_state(&self) -> RecordingState {
        RecordingState::from(self.state.load(Ordering::Acquire))
    }

    /// Set recording state
    pub fn set_state(&self, state: RecordingState) {
        self.state.store(state as u8, Ordering::Release);
    }

    /// Check if session is active
    pub fn is_active(&self) -> bool {
        self.is_active.load(Ordering::Acquire)
    }

    /// Deactivate session
    pub fn deactivate(&self) {
        self.is_active.store(false, Ordering::Release);
    }

    /// Get recording configuration
    pub fn config(&self) -> &RecordingConfig {
        &self.config
    }

    /// Get channel index
    pub fn channel_index(&self) -> usize {
        self.config.channel_index
    }

    /// Get recording source
    pub fn source(&self) -> RecordingSource {
        self.config.source
    }

    /// Get recording mode
    pub fn mode(&self) -> RecordingMode {
        self.config.mode
    }

    /// Modify recording buffer (lock-free clone-modify-swap)
    ///
    /// Loads buffer, clones it, applies function, then atomically swaps back.
    /// This is lock-free but involves a clone operation.
    /// Uses RCU (Read-Copy-Update) pattern.
    pub fn with_buffer<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut RecordingBuffer) -> R,
    {
        // Load current buffer (RCU: Read)
        let current = self.buffer.load_full();

        // Clone and modify (RCU: Copy + Update)
        let mut modified = (*current).clone();
        let result = f(&mut modified);

        // Swap atomically (RCU: Update)
        self.buffer.store(Arc::new(modified));

        result
    }

    /// Get buffer snapshot (lock-free atomic load)
    pub fn get_buffer(&self) -> Arc<RecordingBuffer> {
        self.buffer.load_full()
    }

    /// Swap buffer with a new one (lock-free atomic swap)
    ///
    /// Returns the old buffer
    pub fn swap_buffer(&self, new_buffer: RecordingBuffer) -> Arc<RecordingBuffer> {
        self.buffer.swap(Arc::new(new_buffer))
    }

    /// Update pre-roll countdown (lock-free atomic CAS loop)
    ///
    /// Returns true if pre-roll is complete and recording should start
    pub fn update_preroll(&self, delta_beats: f64) -> bool {
        // Atomic CAS loop to update preroll_remaining
        let mut current_bits = self.preroll_remaining.load(Ordering::Acquire);
        loop {
            let current = f64::from_bits(current_bits);
            let new_value = current - delta_beats;
            let new_bits = new_value.to_bits();

            match self.preroll_remaining.compare_exchange_weak(
                current_bits,
                new_bits,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    // Successfully updated, check if pre-roll is complete
                    if new_value <= 0.0 {
                        let state = self.get_state();
                        if state == RecordingState::Armed {
                            let new_state = match self.config.mode {
                                RecordingMode::Replace => RecordingState::Recording,
                                RecordingMode::Overdub => RecordingState::Overdubbing,
                                RecordingMode::Loop => RecordingState::Recording,
                            };
                            self.set_state(new_state);
                            return true;
                        }
                    }
                    return false;
                }
                Err(updated_bits) => {
                    // Retry with updated value
                    current_bits = updated_bits;
                }
            }
        }
    }

    /// Check if currently in pre-roll (lock-free)
    pub fn is_in_preroll(&self) -> bool {
        let bits = self.preroll_remaining.load(Ordering::Acquire);
        f64::from_bits(bits) > 0.0
    }

    /// Get remaining pre-roll beats (lock-free)
    pub fn preroll_remaining(&self) -> f64 {
        let bits = self.preroll_remaining.load(Ordering::Acquire);
        f64::from_bits(bits)
    }

    /// Set capture ID (for Butler disk recording) - lock-free
    pub fn set_capture_id(&self, id: CaptureId) {
        self.capture_id.store(id.0, Ordering::Release);
    }

    /// Get capture ID - lock-free
    pub fn get_capture_id(&self) -> Option<CaptureId> {
        let id_value = self.capture_id.load(Ordering::Acquire);
        if id_value == 0 {
            None
        } else {
            Some(CaptureId(id_value))
        }
    }

    /// Set recording file path - lock-free via ArcSwap
    pub fn set_recording_file(&self, path: PathBuf) {
        self.recording_file.store(Arc::new(Some(path)));
    }

    /// Get recording file path - lock-free via ArcSwap
    pub fn get_recording_file(&self) -> Option<PathBuf> {
        let guard = self.recording_file.load();
        guard.as_ref().clone()
    }

    /// Set capture buffer producer (for audio input recording)
    pub fn set_capture_producer(&self, producer: CaptureBufferProducer) {
        self.capture_producer
            .store(Arc::new(Some(Arc::new(producer))));
    }

    /// Get capture buffer producer (lock-free read for audio thread)
    pub fn get_capture_producer(&self) -> Option<Arc<CaptureBufferProducer>> {
        let guard = self.capture_producer.load();
        guard.as_ref().clone()
    }

    /// Check if punch-in time reached
    pub fn check_punch_in(&self, current_beat: f64) -> bool {
        if let Some(punch_in) = self.config.punch_in {
            current_beat >= punch_in && self.get_state() == RecordingState::Armed
        } else {
            false
        }
    }

    /// Check if punch-out time reached
    pub fn check_punch_out(&self, current_beat: f64) -> bool {
        if let Some(punch_out) = self.config.punch_out {
            current_beat >= punch_out
                && (self.get_state() == RecordingState::Recording
                    || self.get_state() == RecordingState::Overdubbing)
        } else {
            false
        }
    }

    /// Enable or disable record safe mode.
    ///
    /// When record safe is enabled, the session will not record any data
    /// even if armed or in recording state.
    pub fn set_record_safe(&self, safe: bool) {
        self.record_safe.store(safe, Ordering::Release);
    }

    /// Check if record safe mode is enabled.
    pub fn is_record_safe(&self) -> bool {
        self.record_safe.load(Ordering::Acquire)
    }

    /// Check if recording is allowed (not in record safe mode).
    pub fn can_record(&self) -> bool {
        !self.is_record_safe()
    }

    /// Process punch events and automatically transition states.
    ///
    /// Call this from the audio callback with the current beat position.
    /// Returns `Some(PunchEvent)` if a punch transition occurred.
    ///
    /// This method handles:
    /// - Armed → Recording/Overdubbing on punch_in
    /// - Recording/Overdubbing → Armed on punch_out (if auto_disarm is set)
    /// - Recording/Overdubbing → Stopped on punch_out (default)
    pub fn process_punch(
        &self,
        current_beat: f64,
        sample_position: Option<u64>,
    ) -> Option<PunchEvent> {
        // Don't process if record safe is enabled
        if self.is_record_safe() {
            return None;
        }

        let state = self.get_state();

        match state {
            RecordingState::Armed => {
                if self.check_punch_in(current_beat) {
                    let new_state = match self.config.mode {
                        RecordingMode::Replace => RecordingState::Recording,
                        RecordingMode::Overdub => RecordingState::Overdubbing,
                        RecordingMode::Loop => RecordingState::Recording,
                    };
                    self.set_state(new_state);

                    let event = PunchEvent::PunchIn {
                        beat: current_beat,
                        sample: sample_position,
                    };
                    self.record_punch_event(event);
                    return Some(event);
                }
            }
            RecordingState::Recording | RecordingState::Overdubbing => {
                if self.check_punch_out(current_beat) {
                    // Default: stop recording on punch out
                    self.set_state(RecordingState::Stopped);

                    let event = PunchEvent::PunchOut {
                        beat: current_beat,
                        sample: sample_position,
                    };
                    self.record_punch_event(event);
                    return Some(event);
                }
            }
            RecordingState::Stopped => {}
        }

        None
    }

    /// Record a punch event (lock-free via ArcSwap).
    fn record_punch_event(&self, event: PunchEvent) {
        let current = self.punch_events.load_full();
        let mut events = (*current).clone();
        events.push(event);
        self.punch_events.store(Arc::new(events));
    }

    /// Get all punch events that occurred during this session.
    pub fn get_punch_events(&self) -> Vec<PunchEvent> {
        let events = self.punch_events.load_full();
        (*events).clone()
    }

    /// Record an XRun event (lock-free via ArcSwap).
    ///
    /// Call this when a buffer underrun or overrun is detected.
    pub fn record_xrun(&self, sample_position: u64, beat: Option<f64>, xrun_type: XRunType) {
        let event = XRunEvent {
            sample_position,
            beat,
            xrun_type,
        };

        let current = self.xrun_events.load_full();
        let mut events = (*current).clone();
        events.push(event);
        self.xrun_events.store(Arc::new(events));
    }

    /// Get all XRun events that occurred during this session.
    pub fn get_xrun_events(&self) -> Vec<XRunEvent> {
        let events = self.xrun_events.load_full();
        (*events).clone()
    }

    /// Get XRun count.
    pub fn xrun_count(&self) -> usize {
        self.xrun_events.load().len()
    }

    /// Check if any XRuns occurred.
    pub fn has_xruns(&self) -> bool {
        !self.xrun_events.load().is_empty()
    }
}

impl std::fmt::Debug for RecordingSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingSession")
            .field("state", &self.get_state())
            .field("channel_index", &self.config.channel_index)
            .field("source", &self.config.source)
            .field("mode", &self.config.mode)
            .field("is_active", &self.is_active())
            .finish()
    }
}

/// Recorded data result (returned when stopping recording)
#[derive(Debug)]
pub enum RecordedData {
    /// MIDI recording result
    Midi {
        /// Recorded buffer
        buffer: RecordingBuffer,
        /// Punch events that occurred
        punch_events: Vec<PunchEvent>,
        /// XRun events that occurred
        xrun_events: Vec<XRunEvent>,
    },
    /// Audio recording result (disk file)
    Audio {
        /// File path to recorded WAV file
        file_path: PathBuf,
        /// Duration in seconds
        duration_seconds: f64,
        /// Punch events that occurred
        punch_events: Vec<PunchEvent>,
        /// XRun events that occurred
        xrun_events: Vec<XRunEvent>,
    },
    /// Internal audio recording result (in-memory)
    InternalAudio {
        /// Recorded buffer
        buffer: RecordingBuffer,
        /// Punch events that occurred
        punch_events: Vec<PunchEvent>,
        /// XRun events that occurred
        xrun_events: Vec<XRunEvent>,
    },
    /// Pattern recording result
    Pattern {
        /// Recorded buffer
        buffer: RecordingBuffer,
        /// Punch events that occurred
        punch_events: Vec<PunchEvent>,
        /// XRun events that occurred
        xrun_events: Vec<XRunEvent>,
    },
}

impl RecordedData {
    /// Get punch events from any variant.
    pub fn punch_events(&self) -> &[PunchEvent] {
        match self {
            RecordedData::Midi { punch_events, .. } => punch_events,
            RecordedData::Audio { punch_events, .. } => punch_events,
            RecordedData::InternalAudio { punch_events, .. } => punch_events,
            RecordedData::Pattern { punch_events, .. } => punch_events,
        }
    }

    /// Get XRun events from any variant.
    pub fn xrun_events(&self) -> &[XRunEvent] {
        match self {
            RecordedData::Midi { xrun_events, .. } => xrun_events,
            RecordedData::Audio { xrun_events, .. } => xrun_events,
            RecordedData::InternalAudio { xrun_events, .. } => xrun_events,
            RecordedData::Pattern { xrun_events, .. } => xrun_events,
        }
    }

    /// Check if any XRuns occurred during recording.
    pub fn has_xruns(&self) -> bool {
        !self.xrun_events().is_empty()
    }

    /// Get XRun count.
    pub fn xrun_count(&self) -> usize {
        self.xrun_events().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recording_state_conversion() {
        assert_eq!(RecordingState::from(0), RecordingState::Armed);
        assert_eq!(RecordingState::from(1), RecordingState::Recording);
        assert_eq!(RecordingState::from(2), RecordingState::Overdubbing);
        assert_eq!(RecordingState::from(3), RecordingState::Stopped);
        assert_eq!(RecordingState::from(99), RecordingState::Stopped);
    }

    #[test]
    fn test_capture_id_generation() {
        let id1 = CaptureId::generate();
        let id2 = CaptureId::generate();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_recording_session_creation() {
        let config = RecordingConfig::default();
        let session = RecordingSession::new(config, 44100.0, 0.0);

        assert_eq!(session.get_state(), RecordingState::Armed);
        assert!(session.is_active());
        assert_eq!(session.channel_index(), 0);
    }

    #[test]
    fn test_preroll_countdown() {
        let config = RecordingConfig {
            preroll_beats: 4.0,
            ..Default::default()
        };

        let session = RecordingSession::new(config, 44100.0, 0.0);

        assert!(session.is_in_preroll());
        assert!(!session.update_preroll(2.0));
        assert!(session.is_in_preroll());
        assert!(session.update_preroll(2.5));
        assert!(!session.is_in_preroll());
        assert_eq!(session.get_state(), RecordingState::Recording);
    }

    #[test]
    fn test_punch_in_out() {
        let config = RecordingConfig {
            punch_in: Some(4.0),
            punch_out: Some(8.0),
            ..Default::default()
        };

        let session = RecordingSession::new(config, 44100.0, 0.0);

        assert!(!session.check_punch_in(0.0));
        assert!(session.check_punch_in(4.0));
        assert!(!session.check_punch_out(4.0));

        session.set_state(RecordingState::Recording);
        assert!(session.check_punch_out(8.0));
    }

    #[test]
    fn test_automatic_punch_processing() {
        let config = RecordingConfig {
            punch_in: Some(4.0),
            punch_out: Some(8.0),
            preroll_beats: 0.0, // No preroll for this test
            ..Default::default()
        };

        let session = RecordingSession::new(config, 44100.0, 0.0);

        assert!(session.process_punch(2.0, Some(88200)).is_none());
        assert_eq!(session.get_state(), RecordingState::Armed);

        let event = session.process_punch(4.0, Some(176400));
        assert!(event.is_some());
        assert!(matches!(
            event.unwrap(),
            PunchEvent::PunchIn { beat: 4.0, .. }
        ));
        assert_eq!(session.get_state(), RecordingState::Recording);

        assert!(session.process_punch(6.0, Some(264600)).is_none());
        assert_eq!(session.get_state(), RecordingState::Recording);

        let event = session.process_punch(8.0, Some(352800));
        assert!(event.is_some());
        assert!(matches!(
            event.unwrap(),
            PunchEvent::PunchOut { beat: 8.0, .. }
        ));
        assert_eq!(session.get_state(), RecordingState::Stopped);

        // Verify punch events were recorded
        let events = session.get_punch_events();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_record_safe_mode() {
        let config = RecordingConfig {
            punch_in: Some(4.0),
            ..Default::default()
        };

        let session = RecordingSession::new(config, 44100.0, 0.0);

        assert!(!session.is_record_safe());
        assert!(session.can_record());

        session.set_record_safe(true);
        assert!(session.is_record_safe());
        assert!(!session.can_record());

        let event = session.process_punch(4.0, None);
        assert!(event.is_none());
        assert_eq!(session.get_state(), RecordingState::Armed);

        session.set_record_safe(false);
        assert!(session.can_record());

        let event = session.process_punch(4.0, None);
        assert!(event.is_some());
        assert_eq!(session.get_state(), RecordingState::Recording);
    }

    #[test]
    fn test_xrun_tracking() {
        let config = RecordingConfig::default();
        let session = RecordingSession::new(config, 44100.0, 0.0);

        assert!(!session.has_xruns());
        assert_eq!(session.xrun_count(), 0);

        session.record_xrun(44100, Some(1.0), XRunType::Underrun);
        session.record_xrun(88200, Some(2.0), XRunType::Overrun);

        assert!(session.has_xruns());
        assert_eq!(session.xrun_count(), 2);

        let events = session.get_xrun_events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].xrun_type, XRunType::Underrun);
        assert_eq!(events[1].xrun_type, XRunType::Overrun);
        assert_eq!(events[0].sample_position, 44100);
    }
}
