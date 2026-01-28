//! Request types for Butler thread communication.

use std::path::PathBuf;

/// Unique identifier for a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegionId(pub u64);

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

use super::prefetch::{CaptureBufferConsumer, RegionBufferProducer};

/// Butler transport state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButlerState {
    #[default]
    Running,
    Paused,
    #[allow(dead_code)] // Used in butler_state() match as catch-all
    Shutdown,
}

/// Priority for flush operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FlushPriority {
    Normal = 0,
    Urgent = 1,
}

/// Request to flush captured audio to disk.
#[derive(Debug, Clone)]
pub struct FlushRequest {
    pub capture_id: CaptureId,
    pub file_path: PathBuf,
    pub priority: FlushPriority,
}

impl FlushRequest {
    pub fn new(capture_id: CaptureId, file_path: PathBuf) -> Self {
        Self {
            capture_id,
            file_path,
            priority: FlushPriority::Normal,
        }
    }

    pub fn urgent(capture_id: CaptureId, file_path: PathBuf) -> Self {
        Self {
            capture_id,
            file_path,
            priority: FlushPriority::Urgent,
        }
    }
}

/// Command sent to the Butler thread
pub enum ButlerCommand {
    // === Transport Control (Ardour-style) ===
    /// Start/resume butler processing
    Run,
    /// Pause butler (e.g., during locate)
    Pause,
    /// Wait for butler to complete current work and signal ready
    WaitForCompletion,

    // === Region Management ===
    /// Register a region buffer producer (Butler will write to this)
    RegisterProducer {
        /// Region identifier
        region_id: RegionId,
        /// Buffer producer for this region
        producer: RegionBufferProducer,
    },
    /// Remove a region buffer
    RemoveRegion(RegionId),
    /// Seek a region to a new position (flush buffer and reposition)
    SeekRegion {
        /// Region identifier
        region_id: RegionId,
        /// Target sample offset
        sample_offset: usize,
    },

    // === File Streaming (Ultra Low-Level) ===
    /// Stream an audio file to a channel buffer
    StreamAudioFile {
        /// Channel index to stream to
        channel_index: usize,
        /// Path to audio file
        file_path: PathBuf,
        /// Start position in the file (in samples)
        start_sample: usize,
        /// Duration to stream (in samples)
        duration_samples: usize,
        /// Offset into the file to start reading (in samples)
        offset_samples: usize,
        /// Playback speed multiplier
        speed: f32,
        /// Gain multiplier (linear, not dB)
        gain: f32,
    },
    /// Stop streaming for a channel
    StopStreaming {
        /// Channel index to stop
        channel_index: usize
    },
    /// Set playback position for a channel (in seconds)
    SetPlaybackPosition {
        /// Channel index
        channel_index: usize,
        /// Position in seconds
        position_seconds: f64,
    },

    // === Capture (Recording Write-Behind) ===
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

    // === Lifecycle ===
    /// Shutdown the butler thread
    Shutdown,
}

impl std::fmt::Debug for ButlerCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Transport control
            ButlerCommand::Run => write!(f, "Run"),
            ButlerCommand::Pause => write!(f, "Pause"),
            ButlerCommand::WaitForCompletion => write!(f, "WaitForCompletion"),

            // Region management
            ButlerCommand::RegisterProducer { region_id, .. } => f
                .debug_struct("RegisterProducer")
                .field("region_id", region_id)
                .finish(),
            ButlerCommand::RemoveRegion(id) => f.debug_tuple("RemoveRegion").field(id).finish(),
            ButlerCommand::SeekRegion {
                region_id,
                sample_offset,
            } => f
                .debug_struct("SeekRegion")
                .field("region_id", region_id)
                .field("sample_offset", sample_offset)
                .finish(),

            // File streaming
            ButlerCommand::StreamAudioFile {
                channel_index,
                file_path,
                start_sample,
                duration_samples,
                ..
            } => f
                .debug_struct("StreamAudioFile")
                .field("channel_index", channel_index)
                .field("file_path", file_path)
                .field("start_sample", start_sample)
                .field("duration_samples", duration_samples)
                .finish(),
            ButlerCommand::StopStreaming { channel_index } => f
                .debug_struct("StopStreaming")
                .field("channel_index", channel_index)
                .finish(),
            ButlerCommand::SetPlaybackPosition {
                channel_index,
                position_seconds,
            } => f
                .debug_struct("SetPlaybackPosition")
                .field("channel_index", channel_index)
                .field("position_seconds", position_seconds)
                .finish(),

            // Capture
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

            // Lifecycle
            ButlerCommand::Shutdown => write!(f, "Shutdown"),
        }
    }
}
