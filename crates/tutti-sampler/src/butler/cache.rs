//! LRU disk cache for audio files.
//!
//! Bounded cache with least-recently-used eviction.

use dashmap::DashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tutti_core::Wave;

/// LRU cache for audio files.
///
/// Limits cache by entry count and/or total bytes, evicting
/// least-recently-used entries when limits are exceeded.
pub struct LruCache {
    cache: DashMap<PathBuf, CacheEntry>,
    max_entries: usize,
    max_bytes: u64,
    current_bytes: AtomicU64,
}

struct CacheEntry {
    wave: Arc<Wave>,
    last_access: AtomicU64,
    size_bytes: u64,
}

impl LruCache {
    /// Create a new LRU cache with given limits.
    ///
    /// # Arguments
    /// * `max_entries` - Maximum number of cached files
    /// * `max_bytes` - Maximum total bytes (approximate, based on sample count)
    pub fn new(max_entries: usize, max_bytes: u64) -> Self {
        Self {
            cache: DashMap::new(),
            max_entries,
            max_bytes,
            current_bytes: AtomicU64::new(0),
        }
    }

    /// Get a cached wave file, updating its access time.
    pub fn get(&self, path: &PathBuf) -> Option<Arc<Wave>> {
        self.cache.get(path).map(|entry| {
            entry.last_access.store(now_ms(), Ordering::Relaxed);
            entry.wave.clone()
        })
    }

    /// Insert a wave file into the cache.
    ///
    /// Evicts LRU entries if necessary to make room.
    pub fn insert(&self, path: PathBuf, wave: Arc<Wave>) {
        let size = wave.len() as u64 * wave.channels() as u64 * 4;

        if let Some(existing) = self.cache.get(&path) {
            existing.last_access.store(now_ms(), Ordering::Relaxed);
            return;
        }

        while self.cache.len() >= self.max_entries
            || (self.current_bytes.load(Ordering::Relaxed) + size > self.max_bytes
                && !self.cache.is_empty())
        {
            if !self.evict_lru() {
                break;
            }
        }

        self.cache.insert(
            path,
            CacheEntry {
                wave,
                last_access: AtomicU64::new(now_ms()),
                size_bytes: size,
            },
        );
        self.current_bytes.fetch_add(size, Ordering::Relaxed);
    }

    /// Evict the least-recently-used entry.
    ///
    /// Returns true if an entry was evicted, false if cache is empty.
    fn evict_lru(&self) -> bool {
        let mut oldest_path = None;
        let mut oldest_time = u64::MAX;

        for entry in self.cache.iter() {
            let time = entry.value().last_access.load(Ordering::Relaxed);
            if time < oldest_time {
                oldest_time = time;
                oldest_path = Some(entry.key().clone());
            }
        }

        if let Some(path) = oldest_path {
            if let Some((_, entry)) = self.cache.remove(&path) {
                self.current_bytes
                    .fetch_sub(entry.size_bytes, Ordering::Relaxed);
                return true;
            }
        }
        false
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            entries: self.cache.len(),
            bytes: self.current_bytes.load(Ordering::Relaxed),
            max_entries: self.max_entries,
            max_bytes: self.max_bytes,
        }
    }

    /// Clear all cached entries.
    pub fn clear(&self) {
        self.cache.clear();
        self.current_bytes.store(0, Ordering::Relaxed);
    }

    /// Check if a path is cached.
    pub fn contains(&self, path: &PathBuf) -> bool {
        self.cache.contains_key(path)
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Check if cache is empty.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

/// Get current time in milliseconds since UNIX epoch.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Cache statistics snapshot.
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// Current number of cached entries
    pub entries: usize,
    /// Current total bytes cached
    pub bytes: u64,
    /// Maximum entries allowed
    pub max_entries: usize,
    /// Maximum bytes allowed
    pub max_bytes: u64,
}

impl CacheStats {
    /// Get fill percentage (0.0 - 1.0) based on entry count.
    pub fn entry_fill(&self) -> f32 {
        if self.max_entries == 0 {
            0.0
        } else {
            self.entries as f32 / self.max_entries as f32
        }
    }

    /// Get fill percentage (0.0 - 1.0) based on bytes.
    pub fn byte_fill(&self) -> f32 {
        if self.max_bytes == 0 {
            0.0
        } else {
            self.bytes as f32 / self.max_bytes as f32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_wave(samples: usize) -> Arc<Wave> {
        let data = vec![0.0f32; samples];
        Arc::new(Wave::from_samples(44100.0, &data))
    }

    #[test]
    fn test_cache_insert_and_get() {
        let cache = LruCache::new(10, 1024 * 1024);
        let path = PathBuf::from("/test/file.wav");
        let wave = make_wave(100);

        assert!(!cache.contains(&path));
        cache.insert(path.clone(), wave.clone());
        assert!(cache.contains(&path));

        let retrieved = cache.get(&path);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().len(), 100);
    }

    #[test]
    fn test_cache_max_entries_eviction() {
        let cache = LruCache::new(3, u64::MAX);

        for i in 0..3 {
            let path = PathBuf::from(format!("/test/file{}.wav", i));
            cache.insert(path, make_wave(10));
        }
        assert_eq!(cache.len(), 3);

        let path4 = PathBuf::from("/test/file3.wav");
        cache.insert(path4.clone(), make_wave(10));

        assert_eq!(cache.len(), 3);
        assert!(cache.contains(&path4));
    }

    #[test]
    fn test_cache_max_bytes_eviction() {
        let cache = LruCache::new(100, 1000);

        cache.insert(PathBuf::from("/test/a.wav"), make_wave(100));
        cache.insert(PathBuf::from("/test/b.wav"), make_wave(100));
        assert_eq!(cache.len(), 2);

        cache.insert(PathBuf::from("/test/c.wav"), make_wave(100));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_cache_stats() {
        let cache = LruCache::new(10, 10000);
        cache.insert(PathBuf::from("/test/a.wav"), make_wave(100));
        cache.insert(PathBuf::from("/test/b.wav"), make_wave(200));

        let stats = cache.stats();
        assert_eq!(stats.entries, 2);
        assert_eq!(stats.bytes, 1200);
        assert_eq!(stats.max_entries, 10);
        assert_eq!(stats.max_bytes, 10000);
    }

    #[test]
    fn test_cache_clear() {
        let cache = LruCache::new(10, 10000);
        cache.insert(PathBuf::from("/test/a.wav"), make_wave(100));
        cache.insert(PathBuf::from("/test/b.wav"), make_wave(100));
        assert_eq!(cache.len(), 2);

        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.stats().bytes, 0);
    }

    #[test]
    fn test_duplicate_insert_no_double_count() {
        let cache = LruCache::new(10, 10000);
        let path = PathBuf::from("/test/a.wav");

        cache.insert(path.clone(), make_wave(100));
        let bytes_after_first = cache.stats().bytes;

        cache.insert(path.clone(), make_wave(100));
        let bytes_after_second = cache.stats().bytes;

        assert_eq!(bytes_after_first, bytes_after_second);
        assert_eq!(cache.len(), 1);
    }
}
