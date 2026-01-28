//! Thumbnail Caching
//!
//! Provides memory and disk caching for waveform thumbnails.
//!
//! ## Features
//!
//! - LRU memory cache with configurable size
//! - Optional disk persistence
//! - Hash-based file identification (path + mtime + size)

use crate::waveform::MultiResolutionSummary;
use lru::LruCache;
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

/// Thumbnail cache with LRU memory cache and optional disk persistence
pub struct ThumbnailCache {
    /// In-memory LRU cache
    memory_cache: LruCache<u64, MultiResolutionSummary>,
    /// Optional disk cache directory
    disk_path: Option<PathBuf>,
}

impl ThumbnailCache {
    /// Create a memory-only cache
    ///
    /// # Arguments
    /// * `max_entries` - Maximum number of thumbnails to keep in memory
    pub fn new(max_entries: usize) -> Self {
        let capacity = NonZeroUsize::new(max_entries.max(1)).unwrap();
        Self {
            memory_cache: LruCache::new(capacity),
            disk_path: None,
        }
    }

    /// Create a cache with disk persistence
    ///
    /// # Arguments
    /// * `max_memory_entries` - Maximum entries in memory
    /// * `disk_path` - Directory for disk cache storage
    pub fn with_disk_cache(max_memory_entries: usize, disk_path: PathBuf) -> io::Result<Self> {
        // Ensure disk cache directory exists
        fs::create_dir_all(&disk_path)?;

        let capacity = NonZeroUsize::new(max_memory_entries.max(1)).unwrap();
        Ok(Self {
            memory_cache: LruCache::new(capacity),
            disk_path: Some(disk_path),
        })
    }

    /// Get a cached thumbnail or compute it
    ///
    /// Checks memory cache first, then disk cache, then computes.
    ///
    /// # Arguments
    /// * `hash` - Unique identifier for this audio (use `hash_file` or `hash_samples`)
    /// * `compute` - Function to compute the thumbnail if not cached
    pub fn get_or_compute<F>(&mut self, hash: u64, compute: F) -> &MultiResolutionSummary
    where
        F: FnOnce() -> MultiResolutionSummary,
    {
        // Check memory cache
        if self.memory_cache.contains(&hash) {
            return self.memory_cache.get(&hash).unwrap();
        }

        // Check disk cache
        if let Some(ref disk_path) = self.disk_path {
            if let Some(summary) = self.load_from_disk(disk_path, hash) {
                self.memory_cache.put(hash, summary);
                return self.memory_cache.get(&hash).unwrap();
            }
        }

        // Compute and cache
        let summary = compute();

        // Save to disk if enabled
        if let Some(ref disk_path) = self.disk_path {
            let _ = self.save_to_disk(disk_path, hash, &summary);
        }

        self.memory_cache.put(hash, summary);
        self.memory_cache.get(&hash).unwrap()
    }

    /// Get a cached thumbnail without computing
    pub fn get(&mut self, hash: u64) -> Option<&MultiResolutionSummary> {
        // Check memory cache first
        if self.memory_cache.contains(&hash) {
            return self.memory_cache.get(&hash);
        }

        // Check disk cache
        if let Some(ref disk_path) = self.disk_path {
            if let Some(summary) = self.load_from_disk(disk_path, hash) {
                self.memory_cache.put(hash, summary);
                return self.memory_cache.get(&hash);
            }
        }

        None
    }

    /// Store a thumbnail in the cache
    pub fn put(&mut self, hash: u64, summary: MultiResolutionSummary) {
        // Save to disk if enabled
        if let Some(ref disk_path) = self.disk_path {
            let _ = self.save_to_disk(disk_path, hash, &summary);
        }

        self.memory_cache.put(hash, summary);
    }

    /// Remove a thumbnail from cache
    pub fn remove(&mut self, hash: u64) {
        self.memory_cache.pop(&hash);

        // Remove from disk if present
        if let Some(ref disk_path) = self.disk_path {
            let path = self.disk_cache_path(disk_path, hash);
            let _ = fs::remove_file(path);
        }
    }

    /// Clear all cached thumbnails
    pub fn clear(&mut self) {
        self.memory_cache.clear();

        // Clear disk cache if enabled
        if let Some(ref disk_path) = self.disk_path {
            if let Ok(entries) = fs::read_dir(disk_path) {
                for entry in entries.flatten() {
                    if entry.path().extension().map_or(false, |e| e == "thumb") {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
        }
    }

    /// Get number of entries in memory cache
    pub fn len(&self) -> usize {
        self.memory_cache.len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.memory_cache.is_empty()
    }

    fn disk_cache_path(&self, base: &Path, hash: u64) -> PathBuf {
        base.join(format!("{:016x}.thumb", hash))
    }

    fn load_from_disk(&self, base: &Path, hash: u64) -> Option<MultiResolutionSummary> {
        let path = self.disk_cache_path(base, hash);
        let mut file = File::open(path).ok()?;

        let mut data = Vec::new();
        file.read_to_end(&mut data).ok()?;

        // Simple binary format: [num_levels: u32] [for each level: [num_blocks: u32] [blocks...]]
        self.deserialize_summary(&data)
    }

    fn save_to_disk(
        &self,
        base: &Path,
        hash: u64,
        summary: &MultiResolutionSummary,
    ) -> io::Result<()> {
        let path = self.disk_cache_path(base, hash);
        let mut file = File::create(path)?;

        let data = self.serialize_summary(summary);
        file.write_all(&data)?;

        Ok(())
    }

    fn serialize_summary(&self, summary: &MultiResolutionSummary) -> Vec<u8> {
        let mut data = Vec::new();

        // Version byte
        data.push(1u8);

        // Base samples per block
        data.extend_from_slice(&(summary.base_samples_per_block as u32).to_le_bytes());

        // Number of levels
        data.extend_from_slice(&(summary.levels.len() as u32).to_le_bytes());

        for level in &summary.levels {
            // Samples per block for this level
            data.extend_from_slice(&(level.samples_per_block as u32).to_le_bytes());
            // Total samples
            data.extend_from_slice(&(level.total_samples as u64).to_le_bytes());
            // Number of blocks
            data.extend_from_slice(&(level.blocks.len() as u32).to_le_bytes());

            // Blocks (min, max, rms as f32)
            for block in &level.blocks {
                data.extend_from_slice(&block.min.to_le_bytes());
                data.extend_from_slice(&block.max.to_le_bytes());
                data.extend_from_slice(&block.rms.to_le_bytes());
            }
        }

        data
    }

    fn deserialize_summary(&self, data: &[u8]) -> Option<MultiResolutionSummary> {
        if data.is_empty() {
            return None;
        }

        let mut pos = 0;

        // Version check
        let version = data.get(pos)?;
        if *version != 1 {
            return None;
        }
        pos += 1;

        // Base samples per block
        let base_samples_per_block =
            u32::from_le_bytes(data.get(pos..pos + 4)?.try_into().ok()?) as usize;
        pos += 4;

        // Number of levels
        let num_levels = u32::from_le_bytes(data.get(pos..pos + 4)?.try_into().ok()?) as usize;
        pos += 4;

        let mut levels = Vec::with_capacity(num_levels);

        for _ in 0..num_levels {
            // Samples per block
            let samples_per_block =
                u32::from_le_bytes(data.get(pos..pos + 4)?.try_into().ok()?) as usize;
            pos += 4;

            // Total samples
            let total_samples =
                u64::from_le_bytes(data.get(pos..pos + 8)?.try_into().ok()?) as usize;
            pos += 8;

            // Number of blocks
            let num_blocks = u32::from_le_bytes(data.get(pos..pos + 4)?.try_into().ok()?) as usize;
            pos += 4;

            let mut blocks = Vec::with_capacity(num_blocks);

            for _ in 0..num_blocks {
                let min = f32::from_le_bytes(data.get(pos..pos + 4)?.try_into().ok()?);
                pos += 4;
                let max = f32::from_le_bytes(data.get(pos..pos + 4)?.try_into().ok()?);
                pos += 4;
                let rms = f32::from_le_bytes(data.get(pos..pos + 4)?.try_into().ok()?);
                pos += 4;

                blocks.push(crate::waveform::WaveformBlock { min, max, rms });
            }

            levels.push(crate::waveform::WaveformSummary {
                blocks,
                samples_per_block,
                total_samples,
            });
        }

        Some(MultiResolutionSummary {
            levels,
            base_samples_per_block,
        })
    }
}

/// Compute a hash for a file based on path, modification time, and size
///
/// This provides a fast way to identify if a file has changed without
/// reading its contents.
pub fn hash_file(path: &Path) -> io::Result<u64> {
    let metadata = fs::metadata(path)?;
    let modified = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();

    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    modified.as_secs().hash(&mut hasher);
    modified.subsec_nanos().hash(&mut hasher);

    Ok(hasher.finish())
}

/// Compute a hash for raw sample data
pub fn hash_samples(samples: &[f32]) -> u64 {
    let mut hasher = DefaultHasher::new();

    // Hash length
    samples.len().hash(&mut hasher);

    // Hash a subset of samples for speed (every Nth sample)
    let step = (samples.len() / 1000).max(1);
    for (i, &sample) in samples.iter().enumerate().step_by(step) {
        i.hash(&mut hasher);
        sample.to_bits().hash(&mut hasher);
    }

    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waveform::{WaveformBlock, WaveformSummary};

    fn create_test_summary() -> MultiResolutionSummary {
        MultiResolutionSummary {
            levels: vec![
                WaveformSummary {
                    blocks: vec![
                        WaveformBlock {
                            min: -0.5,
                            max: 0.5,
                            rms: 0.3,
                        },
                        WaveformBlock {
                            min: -0.8,
                            max: 0.8,
                            rms: 0.5,
                        },
                    ],
                    samples_per_block: 512,
                    total_samples: 1024,
                },
                WaveformSummary {
                    blocks: vec![WaveformBlock {
                        min: -0.8,
                        max: 0.8,
                        rms: 0.4,
                    }],
                    samples_per_block: 1024,
                    total_samples: 1024,
                },
            ],
            base_samples_per_block: 512,
        }
    }

    #[test]
    fn test_memory_cache() {
        let mut cache = ThumbnailCache::new(10);

        let summary = create_test_summary();
        cache.put(12345, summary.clone());

        assert_eq!(cache.len(), 1);

        let retrieved = cache.get(12345);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().levels.len(), 2);

        // Non-existent key
        assert!(cache.get(99999).is_none());
    }

    #[test]
    fn test_get_or_compute() {
        let mut cache = ThumbnailCache::new(10);

        let mut compute_count = 0;

        // First call should compute
        let _ = cache.get_or_compute(12345, || {
            compute_count += 1;
            create_test_summary()
        });
        assert_eq!(compute_count, 1);

        // Second call should use cache
        let _ = cache.get_or_compute(12345, || {
            compute_count += 1;
            create_test_summary()
        });
        assert_eq!(compute_count, 1); // Should still be 1
    }

    #[test]
    fn test_lru_eviction() {
        let mut cache = ThumbnailCache::new(2);

        cache.put(1, create_test_summary());
        cache.put(2, create_test_summary());
        cache.put(3, create_test_summary()); // Should evict 1

        assert_eq!(cache.len(), 2);
        assert!(cache.get(1).is_none()); // Should be evicted
        assert!(cache.get(2).is_some());
        assert!(cache.get(3).is_some());
    }

    #[test]
    fn test_serialization_roundtrip() {
        let cache = ThumbnailCache::new(10);
        let original = create_test_summary();

        let serialized = cache.serialize_summary(&original);
        let deserialized = cache.deserialize_summary(&serialized);

        assert!(deserialized.is_some());
        let restored = deserialized.unwrap();

        assert_eq!(restored.levels.len(), original.levels.len());
        assert_eq!(
            restored.base_samples_per_block,
            original.base_samples_per_block
        );

        for (orig_level, rest_level) in original.levels.iter().zip(restored.levels.iter()) {
            assert_eq!(orig_level.blocks.len(), rest_level.blocks.len());
            assert_eq!(orig_level.samples_per_block, rest_level.samples_per_block);

            for (orig_block, rest_block) in orig_level.blocks.iter().zip(rest_level.blocks.iter()) {
                assert!((orig_block.min - rest_block.min).abs() < 0.0001);
                assert!((orig_block.max - rest_block.max).abs() < 0.0001);
                assert!((orig_block.rms - rest_block.rms).abs() < 0.0001);
            }
        }
    }

    #[test]
    fn test_hash_samples() {
        let samples1: Vec<f32> = (0..1000).map(|i| (i as f32 / 100.0).sin()).collect();
        let samples2: Vec<f32> = (0..1000).map(|i| (i as f32 / 100.0).sin()).collect();
        let samples3: Vec<f32> = (0..1000).map(|i| (i as f32 / 50.0).sin()).collect();

        let hash1 = hash_samples(&samples1);
        let hash2 = hash_samples(&samples2);
        let hash3 = hash_samples(&samples3);

        assert_eq!(hash1, hash2, "Identical samples should have same hash");
        assert_ne!(hash1, hash3, "Different samples should have different hash");
    }

    #[test]
    fn test_clear() {
        let mut cache = ThumbnailCache::new(10);

        cache.put(1, create_test_summary());
        cache.put(2, create_test_summary());
        assert_eq!(cache.len(), 2);

        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_remove() {
        let mut cache = ThumbnailCache::new(10);

        cache.put(1, create_test_summary());
        cache.put(2, create_test_summary());

        cache.remove(1);

        assert!(cache.get(1).is_none());
        assert!(cache.get(2).is_some());
        assert_eq!(cache.len(), 1);
    }
}
