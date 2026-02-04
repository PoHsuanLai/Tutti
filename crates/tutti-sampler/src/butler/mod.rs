//! Asynchronous disk I/O for audio file streaming.

mod cache;
mod capture;
mod config;
mod crossfade;
mod loops;
mod metrics;
mod pdc;
mod prefetch;
mod refill;
mod request;
mod shared_state;
mod stream_state;
mod thread;
mod transport;
mod varispeed;

pub use cache::{CacheStats, LruCache};
pub use config::BufferConfig;
pub use metrics::{IOMetrics, IOMetricsSnapshot};
pub use prefetch::{CaptureBufferProducer, RegionBufferConsumer};
pub use request::CaptureId;
pub use shared_state::SharedStreamState;
pub use stream_state::ChannelStreamState;
pub use varispeed::{PlayDirection, Varispeed};

pub(crate) use transport::TransportBridge;

pub(crate) use crossfade::LoopCrossfade;
pub(crate) use prefetch::{CaptureBuffer, CaptureBufferConsumer};
pub(crate) use request::{ButlerCommand, FlushRequest};
pub(crate) use thread::ButlerThread;
