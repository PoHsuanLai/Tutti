//! Shared memory for zero-copy audio buffer passing.

use crate::error::{BridgeError, Result};
use memmap2::MmapMut;
use std::cell::UnsafeCell;
use std::fs::OpenOptions;

#[cfg(unix)]
use std::path::PathBuf;

/// Uses `UnsafeCell` for interior mutability since the underlying memory-mapped
/// region is shared between processes and needs to be written to from an immutable
/// reference. This is safe because:
/// 1. Only one process writes to each channel at a time (single producer)
/// 2. The memory is synchronized at the OS level via shared memory
pub struct SharedAudioBuffer {
    mmap: UnsafeCell<MmapMut>,
    name: String,
    channels: usize,
    samples: usize,
    sample_format: crate::protocol::SampleFormat,
    /// Creator owns the memory and cleans up on drop.
    owns_memory: bool,
}

impl SharedAudioBuffer {
    pub fn create(name: String, channels: usize, samples: usize) -> Result<Self> {
        Self::create_with_format(
            name,
            channels,
            samples,
            crate::protocol::SampleFormat::Float32,
        )
    }

    pub fn create_with_format(
        name: String,
        channels: usize,
        samples: usize,
        sample_format: crate::protocol::SampleFormat,
    ) -> Result<Self> {
        let sample_size = match sample_format {
            crate::protocol::SampleFormat::Float32 => std::mem::size_of::<f32>(),
            crate::protocol::SampleFormat::Float64 => std::mem::size_of::<f64>(),
        };
        let size = channels * samples * sample_size;

        #[cfg(unix)]
        let mmap = Self::create_unix(&name, size)?;

        #[cfg(windows)]
        let mmap = Self::create_windows(&name, size)?;

        Ok(Self {
            mmap: UnsafeCell::new(mmap),
            name,
            channels,
            samples,
            sample_format,
            owns_memory: true, // Creator owns the memory
        })
    }

    /// Defaults to f32 format.
    pub fn open(name: String, channels: usize, samples: usize) -> Result<Self> {
        Self::open_with_format(
            name,
            channels,
            samples,
            crate::protocol::SampleFormat::Float32,
        )
    }

    pub fn open_with_format(
        name: String,
        channels: usize,
        samples: usize,
        sample_format: crate::protocol::SampleFormat,
    ) -> Result<Self> {
        let sample_size = match sample_format {
            crate::protocol::SampleFormat::Float32 => std::mem::size_of::<f32>(),
            crate::protocol::SampleFormat::Float64 => std::mem::size_of::<f64>(),
        };
        let size = channels * samples * sample_size;

        #[cfg(unix)]
        let mmap = Self::open_unix(&name, size)?;

        #[cfg(windows)]
        let mmap = Self::open_windows(&name, size)?;

        Ok(Self {
            mmap: UnsafeCell::new(mmap),
            name,
            channels,
            samples,
            sample_format,
            owns_memory: false, // Opener doesn't own the memory
        })
    }

    #[cfg(unix)]
    fn create_unix(name: &str, size: usize) -> Result<MmapMut> {
        use std::os::unix::fs::OpenOptionsExt;

        let path = Self::shm_path_unix(name);

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| {
                BridgeError::SharedMemoryError(format!(
                    "Failed to create shared memory file: {}",
                    e
                ))
            })?;

        file.set_len(size as u64).map_err(|e| {
            BridgeError::SharedMemoryError(format!("Failed to set file size: {}", e))
        })?;

        let mmap = unsafe { MmapMut::map_mut(&file) }.map_err(|e| {
            BridgeError::SharedMemoryError(format!("Failed to create memory map: {}", e))
        })?;

        Ok(mmap)
    }

    #[cfg(unix)]
    fn open_unix(name: &str, _size: usize) -> Result<MmapMut> {
        let path = Self::shm_path_unix(name);

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| {
                BridgeError::SharedMemoryError(format!("Failed to open shared memory file: {}", e))
            })?;

        let mmap = unsafe { MmapMut::map_mut(&file) }.map_err(|e| {
            BridgeError::SharedMemoryError(format!("Failed to open memory map: {}", e))
        })?;

        Ok(mmap)
    }

    #[cfg(unix)]
    fn shm_path_unix(name: &str) -> PathBuf {
        // On Linux/macOS, use /dev/shm for shared memory
        #[cfg(target_os = "linux")]
        let base = PathBuf::from("/dev/shm");

        #[cfg(target_os = "macos")]
        let base = std::env::temp_dir();

        base.join(format!("dawai_{}", name))
    }

    #[cfg(windows)]
    fn create_windows(name: &str, size: usize) -> Result<MmapMut> {
        // Windows native shared memory (CreateFileMapping with INVALID_HANDLE_VALUE)
        // would be more efficient, but memmap2 doesn't expose that API.
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join(format!("dawai_{}", name));

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;

        file.set_len(size as u64)?;

        let mmap = unsafe { MmapMut::map_mut(&file) }?;

        Ok(mmap)
    }

    #[cfg(windows)]
    fn open_windows(name: &str, _size: usize) -> Result<MmapMut> {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join(format!("dawai_{}", name));

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| {
                BridgeError::SharedMemoryError(format!("Failed to open shared memory file: {}", e))
            })?;

        let mmap = unsafe { MmapMut::map_mut(&file) }.map_err(|e| {
            BridgeError::SharedMemoryError(format!("Failed to open memory map: {}", e))
        })?;

        Ok(mmap)
    }

    /// Caller must ensure single-writer-per-channel.
    pub fn write_channel(&self, channel: usize, data: &[f32]) -> Result<()> {
        if channel >= self.channels {
            return Err(BridgeError::SharedMemoryError(
                "Channel index out of bounds".to_string(),
            ));
        }

        if data.len() > self.samples {
            return Err(BridgeError::SharedMemoryError(
                "Data length exceeds buffer capacity".to_string(),
            ));
        }

        let offset = channel * self.samples * std::mem::size_of::<f32>();

        // SAFETY: We ensure single-writer-per-channel at the API level
        let mmap = unsafe { &mut *self.mmap.get() };
        let slice = &mut mmap[offset..offset + std::mem::size_of_val(data)];

        let bytes = unsafe {
            std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(data))
        };

        slice.copy_from_slice(bytes);

        Ok(())
    }

    /// Returns a newly allocated Vec.
    pub fn read_channel(&self, channel: usize) -> Result<Vec<f32>> {
        if channel >= self.channels {
            return Err(BridgeError::SharedMemoryError(
                "Channel index out of bounds".to_string(),
            ));
        }

        let offset = channel * self.samples * std::mem::size_of::<f32>();

        // SAFETY: Reading is always safe, even with concurrent writers
        let mmap = unsafe { &*self.mmap.get() };
        let slice = &mmap[offset..offset + self.samples * std::mem::size_of::<f32>()];

        let mut data = vec![0.0f32; self.samples];
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(
                data.as_mut_ptr() as *mut u8,
                data.len() * std::mem::size_of::<f32>(),
            )
        };

        bytes.copy_from_slice(slice);

        Ok(data)
    }

    /// Zero-copy into provided buffer. RT-safe.
    pub fn read_channel_into(&self, channel: usize, output: &mut [f32]) -> Result<usize> {
        if channel >= self.channels {
            return Err(BridgeError::SharedMemoryError(
                "Channel index out of bounds".to_string(),
            ));
        }

        let offset = channel * self.samples * std::mem::size_of::<f32>();

        // SAFETY: Reading is always safe, even with concurrent writers
        let mmap = unsafe { &*self.mmap.get() };
        let slice = &mmap[offset..offset + self.samples * std::mem::size_of::<f32>()];

        let copy_samples = self.samples.min(output.len());
        let copy_bytes = copy_samples * std::mem::size_of::<f32>();

        let bytes =
            unsafe { std::slice::from_raw_parts_mut(output.as_mut_ptr() as *mut u8, copy_bytes) };

        bytes.copy_from_slice(&slice[..copy_bytes]);

        Ok(copy_samples)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn samples(&self) -> usize {
        self.samples
    }

    pub fn sample_format(&self) -> crate::protocol::SampleFormat {
        self.sample_format
    }

    pub fn write_channel_f64(&self, channel: usize, data: &[f64]) -> Result<()> {
        if channel >= self.channels {
            return Err(BridgeError::SharedMemoryError(
                "Channel index out of bounds".to_string(),
            ));
        }

        if data.len() > self.samples {
            return Err(BridgeError::SharedMemoryError(
                "Data length exceeds buffer capacity".to_string(),
            ));
        }

        let offset = channel * self.samples * std::mem::size_of::<f64>();

        // SAFETY: We ensure single-writer-per-channel at the API level
        let mmap = unsafe { &mut *self.mmap.get() };
        let slice = &mut mmap[offset..offset + std::mem::size_of_val(data)];

        let bytes = unsafe {
            std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(data))
        };

        slice.copy_from_slice(bytes);

        Ok(())
    }

    pub fn read_channel_f64(&self, channel: usize) -> Result<Vec<f64>> {
        if channel >= self.channels {
            return Err(BridgeError::SharedMemoryError(
                "Channel index out of bounds".to_string(),
            ));
        }

        let offset = channel * self.samples * std::mem::size_of::<f64>();

        // SAFETY: Reading is always safe, even with concurrent writers
        let mmap = unsafe { &*self.mmap.get() };
        let slice = &mmap[offset..offset + self.samples * std::mem::size_of::<f64>()];

        let mut data = vec![0.0f64; self.samples];
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(
                data.as_mut_ptr() as *mut u8,
                data.len() * std::mem::size_of::<f64>(),
            )
        };

        bytes.copy_from_slice(slice);

        Ok(data)
    }

    /// Zero-copy into provided buffer. RT-safe.
    pub fn read_channel_into_f64(&self, channel: usize, output: &mut [f64]) -> Result<usize> {
        if channel >= self.channels {
            return Err(BridgeError::SharedMemoryError(
                "Channel index out of bounds".to_string(),
            ));
        }

        let offset = channel * self.samples * std::mem::size_of::<f64>();

        // SAFETY: Reading is always safe, even with concurrent writers
        let mmap = unsafe { &*self.mmap.get() };
        let slice = &mmap[offset..offset + self.samples * std::mem::size_of::<f64>()];

        let copy_samples = self.samples.min(output.len());
        let copy_bytes = copy_samples * std::mem::size_of::<f64>();

        let bytes =
            unsafe { std::slice::from_raw_parts_mut(output.as_mut_ptr() as *mut u8, copy_bytes) };

        bytes.copy_from_slice(&slice[..copy_bytes]);

        Ok(copy_samples)
    }
}

impl Clone for SharedAudioBuffer {
    fn clone(&self) -> Self {
        // Reopen the shared memory buffer with the correct format (doesn't duplicate memory, just creates a new mapping)
        Self::open_with_format(
            self.name.clone(),
            self.channels,
            self.samples,
            self.sample_format,
        )
        .expect("Failed to clone SharedAudioBuffer - shared memory no longer accessible")
    }
}

// SAFETY: SharedAudioBuffer is Sync because:
// 1. The UnsafeCell<MmapMut> is only used for interior mutability
// 2. write_channel() is documented to require external synchronization (single writer per channel)
// 3. read_channel() is safe to call concurrently with writes (data race is acceptable for audio)
// 4. The underlying memory-mapped region is shared between processes and is already Sync at the OS level
unsafe impl Sync for SharedAudioBuffer {}

impl Drop for SharedAudioBuffer {
    fn drop(&mut self) {
        // Only clean up shared memory file if this instance owns it
        if self.owns_memory {
            #[cfg(unix)]
            {
                let path = Self::shm_path_unix(&self.name);
                let _ = std::fs::remove_file(path);
            }

            #[cfg(windows)]
            {
                let temp_dir = std::env::temp_dir();
                let path = temp_dir.join(format!("dawai_{}", self.name));
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_buffer_roundtrip() {
        let name = format!("test_buffer_{}", std::process::id());
        let channels = 2;
        let samples = 512;

        // Create buffer
        let writer = SharedAudioBuffer::create(name.clone(), channels, samples).unwrap();

        // Write data
        let test_data: Vec<f32> = (0..samples).map(|i| i as f32 * 0.1).collect();
        writer.write_channel(0, &test_data).unwrap();

        // Open for reading
        let reader = SharedAudioBuffer::open(name, channels, samples).unwrap();

        // Read back
        let read_data = reader.read_channel(0).unwrap();

        // Verify
        assert_eq!(test_data, read_data);
    }

    #[test]
    fn test_shared_buffer_f64_roundtrip() {
        use crate::protocol::SampleFormat;

        let name = format!("test_buffer_f64_{}", std::process::id());
        let channels = 2;
        let samples = 256;

        // Create f64 buffer
        let writer = SharedAudioBuffer::create_with_format(
            name.clone(),
            channels,
            samples,
            SampleFormat::Float64,
        )
        .unwrap();

        assert_eq!(writer.sample_format(), SampleFormat::Float64);

        // Write f64 data with high precision values
        let test_data: Vec<f64> = (0..samples)
            .map(|i| i as f64 * 0.000_000_001 + std::f64::consts::PI)
            .collect();
        writer.write_channel_f64(0, &test_data).unwrap();

        // Open for reading
        let reader =
            SharedAudioBuffer::open_with_format(name, channels, samples, SampleFormat::Float64)
                .unwrap();

        // Read back
        let read_data = reader.read_channel_f64(0).unwrap();

        // Verify exact f64 values preserved
        assert_eq!(test_data, read_data);
    }

    #[test]
    fn test_shared_buffer_f64_read_into() {
        use crate::protocol::SampleFormat;

        let name = format!("test_buffer_f64_into_{}", std::process::id());
        let channels = 1;
        let samples = 128;

        let buffer = SharedAudioBuffer::create_with_format(
            name.clone(),
            channels,
            samples,
            SampleFormat::Float64,
        )
        .unwrap();

        let test_data: Vec<f64> = (0..samples).map(|i| (i as f64).sin()).collect();
        buffer.write_channel_f64(0, &test_data).unwrap();

        // Read into pre-allocated buffer
        let mut output = vec![0.0f64; samples];
        let copied = buffer.read_channel_into_f64(0, &mut output).unwrap();

        assert_eq!(copied, samples);
        assert_eq!(test_data, output);
    }

    #[test]
    fn test_shared_buffer_clone_preserves_format() {
        use crate::protocol::SampleFormat;

        let name = format!("test_buffer_clone_fmt_{}", std::process::id());
        let channels = 2;
        let samples = 64;

        let original =
            SharedAudioBuffer::create_with_format(name, channels, samples, SampleFormat::Float64)
                .unwrap();

        assert_eq!(original.sample_format(), SampleFormat::Float64);

        let cloned = original.clone();
        assert_eq!(cloned.sample_format(), SampleFormat::Float64);
    }

    #[test]
    fn test_shared_buffer_f32_read_into() {
        let name = format!("test_buffer_f32_into_{}", std::process::id());
        let channels = 1;
        let samples = 128;

        let buffer = SharedAudioBuffer::create(name, channels, samples).unwrap();

        let test_data: Vec<f32> = (0..samples).map(|i| (i as f32).sin()).collect();
        buffer.write_channel(0, &test_data).unwrap();

        let mut output = vec![0.0f32; samples];
        let copied = buffer.read_channel_into(0, &mut output).unwrap();

        assert_eq!(copied, samples);
        assert_eq!(test_data, output);
    }

    #[test]
    fn test_write_channel_out_of_bounds() {
        let name = format!("test_oob_write_{}", std::process::id());
        let buffer = SharedAudioBuffer::create(name, 2, 64).unwrap();
        let data = vec![0.0f32; 64];
        // Channel 2 doesn't exist (0 and 1 only)
        assert!(buffer.write_channel(2, &data).is_err());
        assert!(buffer.write_channel(100, &data).is_err());
    }

    #[test]
    fn test_write_channel_oversized_data() {
        let name = format!("test_oversize_{}", std::process::id());
        let buffer = SharedAudioBuffer::create(name, 1, 64).unwrap();
        let data = vec![0.0f32; 128]; // 128 > 64 samples
        assert!(buffer.write_channel(0, &data).is_err());
    }

    #[test]
    fn test_read_channel_out_of_bounds() {
        let name = format!("test_oob_read_{}", std::process::id());
        let buffer = SharedAudioBuffer::create(name, 2, 64).unwrap();
        assert!(buffer.read_channel(2).is_err());
        assert!(buffer.read_channel(100).is_err());
    }

    #[test]
    fn test_getters() {
        let name = format!("test_getters_{}", std::process::id());
        let buffer = SharedAudioBuffer::create(name.clone(), 4, 256).unwrap();
        assert_eq!(buffer.name(), name);
        assert_eq!(buffer.channels(), 4);
        assert_eq!(buffer.samples(), 256);
    }
}
