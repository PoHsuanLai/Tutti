//! Plugin Delay Compensation (PDC) system.

pub(crate) mod delay_buffer;
pub(crate) mod manager;
mod unit;

pub use delay_buffer::DelayBuffer;
pub use manager::{PdcManager, PdcState};
pub use unit::PdcDelayUnit;
