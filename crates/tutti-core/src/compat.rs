//! Compatibility layer - always no_std + alloc
//!
//! Only `std` is enabled when CPAL audio I/O is needed.

// Use parking_lot for consistent lock API (no Result from .lock())
pub use parking_lot::{Mutex, RwLock};

// Always use no_std types
pub use alloc::{
    boxed::Box,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};

pub use core::{
    any,
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering},
};

pub use hashbrown::HashMap;
