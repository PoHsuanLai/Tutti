//! External synchronization support for transport.
//!
//! Supports syncing to external time sources:
//! - MIDI Time Code (MTC) - SMPTE timecode over MIDI
//! - MIDI Clock - 24 PPQN beat clock
//! - Linear Timecode (LTC) - Audio-embedded SMPTE timecode

use crate::compat::Ordering;
use crate::{AtomicDouble, AtomicFlag, AtomicFloat, AtomicU8};

/// External sync source type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum SyncSource {
    /// Internal clock (default) - transport runs independently
    #[default]
    Internal = 0,
    /// MIDI Time Code - follows SMPTE timecode from external device
    MidiTimecode = 1,
    /// MIDI Clock - follows 24 PPQN beat clock from external device
    MidiClock = 2,
    /// Linear Timecode - follows audio-embedded SMPTE timecode
    Ltc = 3,
}

impl SyncSource {
    fn from_u8(val: u8) -> Self {
        match val {
            1 => SyncSource::MidiTimecode,
            2 => SyncSource::MidiClock,
            3 => SyncSource::Ltc,
            _ => SyncSource::Internal,
        }
    }
}

/// Sync lock status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum SyncStatus {
    /// Not synced to external source
    #[default]
    Unlocked = 0,
    /// Attempting to lock to external source
    Locking = 1,
    /// Locked and following external source
    Locked = 2,
    /// Locked but drifting (losing sync)
    Drifting = 3,
}

impl SyncStatus {
    fn from_u8(val: u8) -> Self {
        match val {
            1 => SyncStatus::Locking,
            2 => SyncStatus::Locked,
            3 => SyncStatus::Drifting,
            _ => SyncStatus::Unlocked,
        }
    }
}

/// SMPTE frame rate for MTC/LTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum SmpteFrameRate {
    /// 24 fps (film)
    Fps24 = 0,
    /// 25 fps (PAL video)
    Fps25 = 1,
    /// 29.97 fps drop-frame (NTSC video)
    #[default]
    Fps2997Df = 2,
    /// 29.97 fps non-drop (rare)
    Fps2997Ndf = 3,
    /// 30 fps (audio/music)
    Fps30 = 4,
}

impl SmpteFrameRate {
    /// Get frames per second as f64.
    pub fn fps(&self) -> f64 {
        match self {
            SmpteFrameRate::Fps24 => 24.0,
            SmpteFrameRate::Fps25 => 25.0,
            SmpteFrameRate::Fps2997Df | SmpteFrameRate::Fps2997Ndf => 30000.0 / 1001.0,
            SmpteFrameRate::Fps30 => 30.0,
        }
    }

    /// Check if drop-frame.
    pub fn is_drop_frame(&self) -> bool {
        matches!(self, SmpteFrameRate::Fps2997Df)
    }
}

/// External sync state - lock-free for RT access.
pub struct SyncState {
    /// Current sync source
    source: AtomicU8,
    /// Current sync status
    status: AtomicU8,
    /// External position in beats (from MTC/LTC/MIDI Clock)
    external_position_beats: AtomicDouble,
    /// External tempo in BPM (from MIDI Clock, or derived from MTC/LTC)
    external_tempo: AtomicFloat,
    /// Sync offset in samples (for fine-tuning alignment)
    offset_samples: AtomicDouble,
    /// Whether transport should follow external position
    following: AtomicFlag,
    /// SMPTE frame rate for MTC/LTC
    smpte_frame_rate: AtomicU8,
}

impl Default for SyncState {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncState {
    /// Create new sync state (internal clock, unlocked).
    pub fn new() -> Self {
        Self {
            source: AtomicU8::new(SyncSource::Internal as u8),
            status: AtomicU8::new(SyncStatus::Unlocked as u8),
            external_position_beats: AtomicDouble::new(0.0),
            external_tempo: AtomicFloat::new(120.0),
            offset_samples: AtomicDouble::new(0.0),
            following: AtomicFlag::new(false),
            smpte_frame_rate: AtomicU8::new(SmpteFrameRate::Fps2997Df as u8),
        }
    }

    /// Get current sync source.
    pub fn source(&self) -> SyncSource {
        SyncSource::from_u8(self.source.load(Ordering::Acquire))
    }

    /// Set sync source.
    pub fn set_source(&self, source: SyncSource) {
        self.source.store(source as u8, Ordering::Release);
        // Reset status when changing source
        if source == SyncSource::Internal {
            self.status
                .store(SyncStatus::Unlocked as u8, Ordering::Release);
            self.following.set(false);
        } else {
            self.status
                .store(SyncStatus::Locking as u8, Ordering::Release);
        }
    }

    /// Check if using internal clock.
    pub fn is_internal(&self) -> bool {
        self.source() == SyncSource::Internal
    }

    /// Check if synced to external source.
    pub fn is_external(&self) -> bool {
        !self.is_internal()
    }

    /// Get current sync status.
    pub fn status(&self) -> SyncStatus {
        SyncStatus::from_u8(self.status.load(Ordering::Acquire))
    }

    /// Set sync status.
    pub fn set_status(&self, status: SyncStatus) {
        self.status.store(status as u8, Ordering::Release);
    }

    /// Check if locked to external source.
    pub fn is_locked(&self) -> bool {
        self.status() == SyncStatus::Locked
    }

    /// Get external position in beats.
    pub fn external_position(&self) -> f64 {
        self.external_position_beats.get()
    }

    /// Set external position in beats (called when receiving MTC/LTC/MIDI Clock).
    pub fn set_external_position(&self, beats: f64) {
        self.external_position_beats.set(beats);
    }

    /// Get external tempo in BPM.
    pub fn external_tempo(&self) -> f32 {
        self.external_tempo.get()
    }

    /// Set external tempo in BPM.
    pub fn set_external_tempo(&self, bpm: f32) {
        self.external_tempo.set(bpm.clamp(20.0, 300.0));
    }

    /// Get sync offset in samples.
    pub fn offset_samples(&self) -> f64 {
        self.offset_samples.get()
    }

    /// Set sync offset in samples (positive = delay internal, negative = advance internal).
    pub fn set_offset_samples(&self, samples: f64) {
        self.offset_samples.set(samples);
    }

    /// Check if transport should follow external position.
    pub fn is_following(&self) -> bool {
        self.following.get()
    }

    /// Set whether transport should follow external position.
    pub fn set_following(&self, follow: bool) {
        self.following.set(follow);
    }

    /// Get SMPTE frame rate.
    pub fn smpte_frame_rate(&self) -> SmpteFrameRate {
        match self.smpte_frame_rate.load(Ordering::Acquire) {
            0 => SmpteFrameRate::Fps24,
            1 => SmpteFrameRate::Fps25,
            2 => SmpteFrameRate::Fps2997Df,
            3 => SmpteFrameRate::Fps2997Ndf,
            4 => SmpteFrameRate::Fps30,
            _ => SmpteFrameRate::Fps2997Df,
        }
    }

    /// Set SMPTE frame rate.
    pub fn set_smpte_frame_rate(&self, rate: SmpteFrameRate) {
        self.smpte_frame_rate.store(rate as u8, Ordering::Release);
    }

    /// Get a snapshot of sync state for UI display.
    pub fn snapshot(&self) -> SyncSnapshot {
        SyncSnapshot {
            source: self.source(),
            status: self.status(),
            external_position: self.external_position(),
            external_tempo: self.external_tempo(),
            offset_samples: self.offset_samples(),
            following: self.is_following(),
            smpte_frame_rate: self.smpte_frame_rate(),
        }
    }
}

/// Snapshot of sync state for UI display.
#[derive(Debug, Clone, Copy)]
pub struct SyncSnapshot {
    pub source: SyncSource,
    pub status: SyncStatus,
    pub external_position: f64,
    pub external_tempo: f32,
    pub offset_samples: f64,
    pub following: bool,
    pub smpte_frame_rate: SmpteFrameRate,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_state_default() {
        let state = SyncState::new();
        assert_eq!(state.source(), SyncSource::Internal);
        assert_eq!(state.status(), SyncStatus::Unlocked);
        assert!(!state.is_following());
        assert!(state.is_internal());
    }

    #[test]
    fn test_set_sync_source() {
        let state = SyncState::new();

        state.set_source(SyncSource::MidiTimecode);
        assert_eq!(state.source(), SyncSource::MidiTimecode);
        assert_eq!(state.status(), SyncStatus::Locking);
        assert!(state.is_external());

        state.set_source(SyncSource::Internal);
        assert_eq!(state.source(), SyncSource::Internal);
        assert_eq!(state.status(), SyncStatus::Unlocked);
    }

    #[test]
    fn test_external_position() {
        let state = SyncState::new();
        state.set_external_position(32.5);
        assert!((state.external_position() - 32.5).abs() < 0.001);
    }

    #[test]
    fn test_external_tempo() {
        let state = SyncState::new();
        state.set_external_tempo(140.0);
        assert!((state.external_tempo() - 140.0).abs() < 0.001);

        // Test clamping
        state.set_external_tempo(10.0);
        assert!((state.external_tempo() - 20.0).abs() < 0.001);

        state.set_external_tempo(400.0);
        assert!((state.external_tempo() - 300.0).abs() < 0.001);
    }

    #[test]
    fn test_smpte_frame_rate() {
        assert!((SmpteFrameRate::Fps24.fps() - 24.0).abs() < 0.001);
        assert!((SmpteFrameRate::Fps25.fps() - 25.0).abs() < 0.001);
        assert!((SmpteFrameRate::Fps30.fps() - 30.0).abs() < 0.001);
        assert!(SmpteFrameRate::Fps2997Df.is_drop_frame());
        assert!(!SmpteFrameRate::Fps2997Ndf.is_drop_frame());
    }

    #[test]
    fn test_snapshot() {
        let state = SyncState::new();
        state.set_source(SyncSource::MidiClock);
        state.set_external_position(16.0);
        state.set_external_tempo(128.0);
        state.set_following(true);

        let snap = state.snapshot();
        assert_eq!(snap.source, SyncSource::MidiClock);
        assert!((snap.external_position - 16.0).abs() < 0.001);
        assert!((snap.external_tempo - 128.0).abs() < 0.001);
        assert!(snap.following);
    }
}
