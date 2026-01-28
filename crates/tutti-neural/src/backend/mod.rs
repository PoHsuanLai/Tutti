//! GPU backend management
//!
//! Manages Burn ML backends (GPU + CPU) with automatic device selection.

mod pool;

pub use pool::BackendPool;
