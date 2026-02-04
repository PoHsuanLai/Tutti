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
