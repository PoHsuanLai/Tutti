//! Request types for Butler thread communication.

use std::path::PathBuf;

/// Unique identifier for a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RegionId(pub u64);

impl RegionId {
    /// Generate a new unique region ID.
    pub fn generate() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

/// Unique identifier for a capture buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CaptureId(pub u64);

impl CaptureId {
    /// Generate a new unique capture ID.
    pub fn generate() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

use super::prefetch::CaptureBufferConsumer;
use super::varispeed::PlayDirection;

/// Butler transport state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ButlerState {
    #[default]
    Running,
    Paused,
    #[allow(dead_code)] // Used in butler_state() match as catch-all
    Shutdown,
}

/// Request to flush captured audio to disk.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FlushRequest {
    pub capture_id: CaptureId,
}

impl FlushRequest {
    pub(crate) fn new(capture_id: CaptureId) -> Self {
        Self { capture_id }
    }
}

/// Command sent to the Butler thread
pub(crate) enum ButlerCommand {
    /// Start/resume butler processing
    Run,
    /// Pause butler (e.g., during locate)
    Pause,
    /// Wait for butler to complete current work and signal ready
    WaitForCompletion,

    /// Stream an audio file to a channel buffer
    StreamAudioFile {
        /// Channel index to stream to
        channel_index: usize,
        /// Path to audio file
        file_path: PathBuf,
        /// Offset into the file to start reading (in samples)
        offset_samples: usize,
    },
    /// Stop streaming for a channel
    StopStreaming {
        /// Channel index to stop
        channel_index: usize,
    },

    /// Seek within a stream (in samples)
    SeekStream {
        /// Channel index
        channel_index: usize,
        /// Target position in samples
        position_samples: u64,
    },

    /// Set loop range for a channel (in samples)
    SetLoopRange {
        /// Channel index
        channel_index: usize,
        /// Loop start position in samples
        start_samples: u64,
        /// Loop end position in samples
        end_samples: u64,
        /// Crossfade length in samples (0 = no crossfade)
        crossfade_samples: usize,
    },

    /// Clear loop range for a channel
    ClearLoopRange {
        /// Channel index
        channel_index: usize,
    },

    /// Set varispeed (direction and speed) for a channel
    SetVarispeed {
        /// Channel index
        channel_index: usize,
        /// Playback direction
        direction: PlayDirection,
        /// Speed multiplier (1.0 = normal)
        speed: f32,
    },

    /// Update PDC preroll for a channel (plugin latency changed)
    UpdatePdcPreroll {
        /// Channel index
        channel_index: usize,
        /// New preroll amount in samples
        new_preroll: u64,
    },

    /// Register a capture buffer consumer (Butler will read from this and write to disk)
    RegisterCapture {
        /// Capture session identifier
        capture_id: CaptureId,
        /// Buffer consumer to read from
        consumer: CaptureBufferConsumer,
        /// Output file path
        file_path: PathBuf,
        /// Audio sample rate
        sample_rate: f64,
        /// Number of audio channels
        channels: usize,
    },
    /// Remove a capture buffer (finalize and close file)
    RemoveCapture(CaptureId),
    /// Flush capture buffer to disk
    Flush(FlushRequest),
    /// Flush all capture buffers (e.g., when stopping recording)
    FlushAll,

    /// Set buffer margin multiplier (for external sync jitter handling)
    SetBufferMargin {
        /// Margin multiplier (1.0 = normal, >1.0 = larger buffers for jitter)
        margin: f64,
    },

    /// Shutdown the butler thread
    Shutdown,
}

impl std::fmt::Debug for ButlerCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ButlerCommand::Run => write!(f, "Run"),
            ButlerCommand::Pause => write!(f, "Pause"),
            ButlerCommand::WaitForCompletion => write!(f, "WaitForCompletion"),

            ButlerCommand::StreamAudioFile {
                channel_index,
                file_path,
                offset_samples,
            } => f
                .debug_struct("StreamAudioFile")
                .field("channel_index", channel_index)
                .field("file_path", file_path)
                .field("offset_samples", offset_samples)
                .finish(),
            ButlerCommand::StopStreaming { channel_index } => f
                .debug_struct("StopStreaming")
                .field("channel_index", channel_index)
                .finish(),
            ButlerCommand::SeekStream {
                channel_index,
                position_samples,
            } => f
                .debug_struct("SeekStream")
                .field("channel_index", channel_index)
                .field("position_samples", position_samples)
                .finish(),
            ButlerCommand::SetLoopRange {
                channel_index,
                start_samples,
                end_samples,
                crossfade_samples,
            } => f
                .debug_struct("SetLoopRange")
                .field("channel_index", channel_index)
                .field("start_samples", start_samples)
                .field("end_samples", end_samples)
                .field("crossfade_samples", crossfade_samples)
                .finish(),
            ButlerCommand::ClearLoopRange { channel_index } => f
                .debug_struct("ClearLoopRange")
                .field("channel_index", channel_index)
                .finish(),
            ButlerCommand::SetVarispeed {
                channel_index,
                direction,
                speed,
            } => f
                .debug_struct("SetVarispeed")
                .field("channel_index", channel_index)
                .field("direction", direction)
                .field("speed", speed)
                .finish(),
            ButlerCommand::UpdatePdcPreroll {
                channel_index,
                new_preroll,
            } => f
                .debug_struct("UpdatePdcPreroll")
                .field("channel_index", channel_index)
                .field("new_preroll", new_preroll)
                .finish(),

            ButlerCommand::RegisterCapture {
                capture_id,
                file_path,
                ..
            } => f
                .debug_struct("RegisterCapture")
                .field("capture_id", capture_id)
                .field("file_path", file_path)
                .finish(),
            ButlerCommand::RemoveCapture(id) => f.debug_tuple("RemoveCapture").field(id).finish(),
            ButlerCommand::Flush(req) => f.debug_tuple("Flush").field(req).finish(),
            ButlerCommand::FlushAll => write!(f, "FlushAll"),

            ButlerCommand::SetBufferMargin { margin } => f
                .debug_struct("SetBufferMargin")
                .field("margin", margin)
                .finish(),

            ButlerCommand::Shutdown => write!(f, "Shutdown"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_region_id_uniqueness() {
        let mut ids = HashSet::new();
        for _ in 0..1000 {
            let id = RegionId::generate();
            assert!(ids.insert(id.0), "RegionId should be unique");
        }
        assert_eq!(ids.len(), 1000);
    }

    #[test]
    fn test_region_id_monotonic() {
        let id1 = RegionId::generate();
        let id2 = RegionId::generate();
        let id3 = RegionId::generate();
        assert!(id2.0 > id1.0, "IDs should be monotonically increasing");
        assert!(id3.0 > id2.0, "IDs should be monotonically increasing");
    }

    #[test]
    fn test_capture_id_uniqueness() {
        let mut ids = HashSet::new();
        for _ in 0..1000 {
            let id = CaptureId::generate();
            assert!(ids.insert(id.0), "CaptureId should be unique");
        }
        assert_eq!(ids.len(), 1000);
    }

    #[test]
    fn test_capture_id_monotonic() {
        let id1 = CaptureId::generate();
        let id2 = CaptureId::generate();
        let id3 = CaptureId::generate();
        assert!(id2.0 > id1.0, "IDs should be monotonically increasing");
        assert!(id3.0 > id2.0, "IDs should be monotonically increasing");
    }

    #[test]
    fn test_butler_state_default() {
        let state = ButlerState::default();
        assert_eq!(state, ButlerState::Running);
    }

    #[test]
    fn test_flush_request_new() {
        let capture_id = CaptureId::generate();
        let req = FlushRequest::new(capture_id);
        assert_eq!(req.capture_id, capture_id);
    }

    #[test]
    fn test_butler_command_debug_simple_variants() {
        assert_eq!(format!("{:?}", ButlerCommand::Run), "Run");
        assert_eq!(format!("{:?}", ButlerCommand::Pause), "Pause");
        assert_eq!(format!("{:?}", ButlerCommand::WaitForCompletion), "WaitForCompletion");
        assert_eq!(format!("{:?}", ButlerCommand::FlushAll), "FlushAll");
        assert_eq!(format!("{:?}", ButlerCommand::Shutdown), "Shutdown");
    }

    #[test]
    fn test_butler_command_debug_stream_audio_file() {
        let cmd = ButlerCommand::StreamAudioFile {
            channel_index: 0,
            file_path: PathBuf::from("/path/to/audio.wav"),
            offset_samples: 1000,
        };
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("StreamAudioFile"));
        assert!(debug.contains("channel_index: 0"));
        assert!(debug.contains("audio.wav"));
        assert!(debug.contains("offset_samples: 1000"));
    }

    #[test]
    fn test_butler_command_debug_stop_streaming() {
        let cmd = ButlerCommand::StopStreaming { channel_index: 5 };
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("StopStreaming"));
        assert!(debug.contains("channel_index: 5"));
    }

    #[test]
    fn test_butler_command_debug_seek_stream() {
        let cmd = ButlerCommand::SeekStream {
            channel_index: 2,
            position_samples: 44100,
        };
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("SeekStream"));
        assert!(debug.contains("channel_index: 2"));
        assert!(debug.contains("position_samples: 44100"));
    }

    #[test]
    fn test_butler_command_debug_set_loop_range() {
        let cmd = ButlerCommand::SetLoopRange {
            channel_index: 1,
            start_samples: 0,
            end_samples: 88200,
            crossfade_samples: 512,
        };
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("SetLoopRange"));
        assert!(debug.contains("channel_index: 1"));
        assert!(debug.contains("start_samples: 0"));
        assert!(debug.contains("end_samples: 88200"));
        assert!(debug.contains("crossfade_samples: 512"));
    }

    #[test]
    fn test_butler_command_debug_clear_loop_range() {
        let cmd = ButlerCommand::ClearLoopRange { channel_index: 3 };
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("ClearLoopRange"));
        assert!(debug.contains("channel_index: 3"));
    }

    #[test]
    fn test_butler_command_debug_set_varispeed() {
        let cmd = ButlerCommand::SetVarispeed {
            channel_index: 0,
            direction: PlayDirection::Forward,
            speed: 2.0,
        };
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("SetVarispeed"));
        assert!(debug.contains("channel_index: 0"));
        assert!(debug.contains("Forward"));
        assert!(debug.contains("speed: 2.0"));
    }

    #[test]
    fn test_butler_command_debug_update_pdc_preroll() {
        let cmd = ButlerCommand::UpdatePdcPreroll {
            channel_index: 4,
            new_preroll: 256,
        };
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("UpdatePdcPreroll"));
        assert!(debug.contains("channel_index: 4"));
        assert!(debug.contains("new_preroll: 256"));
    }

    #[test]
    fn test_butler_command_debug_remove_capture() {
        let id = CaptureId(42);
        let cmd = ButlerCommand::RemoveCapture(id);
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("RemoveCapture"));
        assert!(debug.contains("42"));
    }

    #[test]
    fn test_butler_command_debug_flush() {
        let req = FlushRequest::new(CaptureId(99));
        let cmd = ButlerCommand::Flush(req);
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("Flush"));
        assert!(debug.contains("99"));
    }

    #[test]
    fn test_butler_command_debug_set_buffer_margin() {
        let cmd = ButlerCommand::SetBufferMargin { margin: 1.5 };
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("SetBufferMargin"));
        assert!(debug.contains("margin: 1.5"));
    }

    #[test]
    fn test_region_id_equality() {
        let id1 = RegionId(100);
        let id2 = RegionId(100);
        let id3 = RegionId(200);
        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_capture_id_equality() {
        let id1 = CaptureId(100);
        let id2 = CaptureId(100);
        let id3 = CaptureId(200);
        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_butler_state_equality() {
        assert_eq!(ButlerState::Running, ButlerState::Running);
        assert_eq!(ButlerState::Paused, ButlerState::Paused);
        assert_eq!(ButlerState::Shutdown, ButlerState::Shutdown);
        assert_ne!(ButlerState::Running, ButlerState::Paused);
    }
}
