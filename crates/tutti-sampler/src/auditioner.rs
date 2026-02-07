//! Auditioner: dedicated preview player for quick file auditioning.
//!
//! Provides low-latency file preview from a browser panel. Uses in-memory
//! playback for short/cached files and disk streaming for longer files.
//! Only one file can be previewed at a time.

use crate::sampler::{SamplerUnit, StreamingSamplerUnit};
use crate::SamplerSystem;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tutti_core::{AtomicFloat, Wave};

/// Reserved channel index for auditioner streaming.
/// Uses a high value to avoid collision with track channels (0, 1, 2...).
const AUDITIONER_CHANNEL: usize = usize::MAX - 1;

/// Threshold in samples: files shorter than this use in-memory playback.
/// ~10 seconds at 48kHz.
const IN_MEMORY_THRESHOLD: usize = 480_000;

/// Playback mode for the current preview.
enum PreviewMode {
    /// In-memory playback (short files or cached files).
    InMemory(SamplerUnit),
    /// Disk streaming via butler (long uncached files).
    Streaming,
}

/// Auditioner for quick file preview from a browser panel.
///
/// Only one file can be previewed at a time. Starting a new preview
/// automatically stops the current one. Uses in-memory playback for
/// short/cached files (instant start) and disk streaming for longer files.
///
/// SRC is applied automatically when the file's sample rate differs
/// from the session sample rate.
///
/// # Example
/// ```ignore
/// let auditioner = sampler.auditioner();
/// auditioner.preview(Path::new("drums.wav"))?;
/// // ... later
/// auditioner.stop();
/// ```
pub struct Auditioner {
    sampler: Arc<SamplerSystem>,
    mode: parking_lot::Mutex<Option<PreviewMode>>,
    current_path: parking_lot::Mutex<Option<PathBuf>>,
    playing: AtomicBool,
    gain: AtomicFloat,
    speed: AtomicFloat,
    session_sample_rate: f64,
}

impl Auditioner {
    /// Create a new Auditioner backed by the given SamplerSystem.
    pub(crate) fn new(sampler: Arc<SamplerSystem>) -> Self {
        let sr = sampler.sample_rate();
        Self {
            sampler,
            mode: parking_lot::Mutex::new(None),
            current_path: parking_lot::Mutex::new(None),
            playing: AtomicBool::new(false),
            gain: AtomicFloat::new(1.0),
            speed: AtomicFloat::new(1.0),
            session_sample_rate: sr,
        }
    }

    /// Preview a file. Stops any current preview first.
    ///
    /// For short files (< ~10s) or files already in the LRU cache,
    /// playback starts instantly using in-memory mode. For longer
    /// uncached files, disk streaming is used.
    pub fn preview(&self, file_path: &Path) -> crate::Result<()> {
        self.stop();

        let path = file_path.to_path_buf();
        let cache = self.sampler.cache();

        let cached_wave = cache.as_ref().and_then(|c| c.get(&path));

        if let Some(wave) = cached_wave {
            self.start_in_memory(wave, &path);
        } else {
            let wave = Arc::new(
                Wave::load(file_path).map_err(|e| crate::Error::SampleNotFound(e.to_string()))?,
            );

            if let Some(ref c) = cache {
                c.insert(path.clone(), wave.clone());
            }

            if wave.len() <= IN_MEMORY_THRESHOLD {
                self.start_in_memory(wave, &path);
            } else {
                self.start_streaming(&path);
            }
        }

        Ok(())
    }

    fn start_in_memory(&self, wave: Arc<Wave>, path: &Path) {
        let mut unit = SamplerUnit::with_settings(wave, self.gain.get(), self.speed.get(), false);
        unit.set_session_sample_rate(self.session_sample_rate);
        unit.trigger();

        *self.mode.lock() = Some(PreviewMode::InMemory(unit));
        *self.current_path.lock() = Some(path.to_path_buf());
        self.playing.store(true, Ordering::Release);
    }

    fn start_streaming(&self, path: &Path) {
        self.sampler.stream_file(AUDITIONER_CHANNEL, path);

        let speed = self.speed.get();
        if (speed - 1.0).abs() > 0.001 {
            self.sampler.set_speed(AUDITIONER_CHANNEL, speed);
        }

        *self.mode.lock() = Some(PreviewMode::Streaming);
        *self.current_path.lock() = Some(path.to_path_buf());
        self.playing.store(true, Ordering::Release);
    }

    /// Stop current preview.
    pub fn stop(&self) {
        let mode = self.mode.lock().take();
        if let Some(mode) = mode {
            match mode {
                PreviewMode::InMemory(unit) => {
                    unit.stop();
                }
                PreviewMode::Streaming => {
                    self.sampler.stop_stream(AUDITIONER_CHANNEL);
                }
            }
        }
        *self.current_path.lock() = None;
        self.playing.store(false, Ordering::Release);
    }

    /// Check if currently playing.
    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Acquire)
    }

    /// Set preview gain (0.0+).
    pub fn set_gain(&self, gain: f32) {
        self.gain.set(gain.max(0.0));
    }

    /// Get current gain.
    pub fn gain(&self) -> f32 {
        self.gain.get()
    }

    /// Set preview speed (clamped to 0.25 - 4.0).
    pub fn set_speed(&self, speed: f32) {
        let clamped = speed.clamp(0.25, 4.0);
        self.speed.set(clamped);
        let mode = self.mode.lock();
        if let Some(PreviewMode::Streaming) = mode.as_ref() {
            self.sampler.set_speed(AUDITIONER_CHANNEL, clamped);
        }
    }

    /// Get current speed.
    pub fn speed(&self) -> f32 {
        self.speed.get()
    }

    /// Get the current file path being previewed.
    pub fn current_path(&self) -> Option<PathBuf> {
        self.current_path.lock().clone()
    }

    /// Get the duration of the current preview file in seconds.
    pub fn duration(&self) -> Option<f64> {
        let mode = self.mode.lock();
        match mode.as_ref()? {
            PreviewMode::InMemory(unit) => Some(unit.duration_seconds()),
            PreviewMode::Streaming => {
                let path = self.current_path.lock();
                let path = path.as_ref()?;
                let cache = self.sampler.cache()?;
                let wave = cache.get(path)?;
                Some(wave.duration())
            }
        }
    }

    /// Get a clone of the in-memory SamplerUnit for audio graph integration.
    ///
    /// Returns None if not in in-memory mode or not playing.
    pub fn in_memory_unit(&self) -> Option<SamplerUnit> {
        let mode = self.mode.lock();
        match mode.as_ref()? {
            PreviewMode::InMemory(unit) => Some(unit.clone()),
            PreviewMode::Streaming => None,
        }
    }

    /// Get the StreamingSamplerUnit for audio graph integration.
    ///
    /// Returns None if not in streaming mode or not playing.
    pub fn streaming_unit(&self) -> Option<StreamingSamplerUnit> {
        let mode = self.mode.lock();
        match mode.as_ref()? {
            PreviewMode::Streaming => self.sampler.streaming_unit(AUDITIONER_CHANNEL),
            PreviewMode::InMemory(_) => None,
        }
    }
}
