//! Asynchronous disk I/O for audio file streaming.

mod prefetch;
mod request;
mod stream_state;
mod thread;

pub use prefetch::{
    // Capture buffers (Audio callback writes → Butler reads)
    CaptureBuffer,
    CaptureBufferConsumer,
    CaptureBufferProducer,
    // Playback buffers (Butler writes → Audio callback reads)
    RegionBufferConsumer,
    RegionBufferProducer,
};
pub use request::{
    // Commands
    ButlerCommand,
    CaptureId,
    // Capture/flush types
    FlushPriority,
    FlushRequest,
    // Identifiers
    RegionId,
};
pub(crate) use thread::ButlerThread;
