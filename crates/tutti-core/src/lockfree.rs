//! Lock-free primitives for real-time audio.

use crate::compat::AtomicBool as StdAtomicBool;
use crate::Ordering;
use atomic_float::{AtomicF32, AtomicF64};

/// Cache-line aligned atomic f32.
#[derive(Debug)]
#[repr(align(64))]
pub struct AtomicFloat {
    value: AtomicF32,
}

impl AtomicFloat {
    pub fn new(value: f32) -> Self {
        Self {
            value: AtomicF32::new(value),
        }
    }

    #[inline]
    pub fn get(&self) -> f32 {
        self.value.load(Ordering::Acquire)
    }

    #[inline]
    pub fn get_relaxed(&self) -> f32 {
        self.value.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn set(&self, value: f32) {
        self.value.store(value, Ordering::Release);
    }

    #[inline]
    pub fn swap(&self, value: f32) -> f32 {
        self.value.swap(value, Ordering::AcqRel)
    }
}

impl Clone for AtomicFloat {
    fn clone(&self) -> Self {
        Self::new(self.get())
    }
}

impl Default for AtomicFloat {
    fn default() -> Self {
        Self::new(0.0)
    }
}

/// Cache-line aligned atomic bool.
#[derive(Debug)]
#[repr(align(64))]
pub struct AtomicFlag {
    value: StdAtomicBool,
}

impl AtomicFlag {
    pub fn new(value: bool) -> Self {
        Self {
            value: StdAtomicBool::new(value),
        }
    }

    #[inline]
    pub fn get(&self) -> bool {
        self.value.load(Ordering::Acquire)
    }

    #[inline]
    pub fn set(&self, value: bool) {
        self.value.store(value, Ordering::Release);
    }

    #[inline]
    pub fn swap(&self, value: bool) -> bool {
        self.value.swap(value, Ordering::AcqRel)
    }
}

impl Clone for AtomicFlag {
    fn clone(&self) -> Self {
        Self::new(self.get())
    }
}

impl Default for AtomicFlag {
    fn default() -> Self {
        Self::new(false)
    }
}

/// Cache-line aligned atomic f64.
#[derive(Debug)]
#[repr(align(64))]
pub struct AtomicDouble {
    value: AtomicF64,
}

impl AtomicDouble {
    pub fn new(value: f64) -> Self {
        Self {
            value: AtomicF64::new(value),
        }
    }

    #[inline]
    pub fn get(&self) -> f64 {
        self.value.load(Ordering::Acquire)
    }

    #[inline]
    pub fn set(&self, value: f64) {
        self.value.store(value, Ordering::Release);
    }

    #[inline]
    pub fn swap(&self, value: f64) -> f64 {
        self.value.swap(value, Ordering::AcqRel)
    }
}

impl Clone for AtomicDouble {
    fn clone(&self) -> Self {
        Self::new(self.get())
    }
}

impl Default for AtomicDouble {
    fn default() -> Self {
        Self::new(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atomic_float() {
        let val = AtomicFloat::new(1.0);
        assert_eq!(val.get(), 1.0);
        val.set(2.5);
        assert_eq!(val.get(), 2.5);
    }

    #[test]
    fn test_atomic_flag() {
        let flag = AtomicFlag::new(false);
        assert!(!flag.get());
        flag.set(true);
        assert!(flag.get());
    }
}
