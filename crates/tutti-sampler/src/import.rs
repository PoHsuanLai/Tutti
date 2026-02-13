//! Non-blocking audio file import with progress polling.

use crossbeam_channel::Receiver;
use std::path::Path;
use std::sync::Arc;
use std::thread::JoinHandle;
use tutti_core::Wave;

/// Status of a background import operation.
pub enum ImportStatus {
    /// Import is in progress (0.0 to 1.0).
    Running(f32),
    /// Import completed successfully.
    Complete(Arc<Wave>),
    /// Import failed with an error message.
    Failed(String),
    /// No progress yet (just started).
    Pending,
}

/// Handle to a background audio file import.
///
/// Created by [`TuttiEngine::start_load_wave()`]. Poll with [`progress()`] each frame.
///
/// # Example
/// ```ignore
/// let mut import = engine.start_load_wave("song.mp3");
///
/// loop {
///     match import.progress() {
///         ImportStatus::Running(p) => println!("Loading: {:.0}%", p * 100.0),
///         ImportStatus::Complete(wave) => { println!("Done: {}s", wave.duration()); break; }
///         ImportStatus::Failed(e) => { eprintln!("Error: {}", e); break; }
///         ImportStatus::Pending => {}
///     }
/// }
/// ```
pub struct ImportHandle {
    progress_rx: Receiver<f32>,
    thread: Option<JoinHandle<std::result::Result<Arc<Wave>, String>>>,
    last_progress: Option<f32>,
}

impl ImportHandle {
    pub(crate) fn new(
        progress_rx: Receiver<f32>,
        thread: JoinHandle<std::result::Result<Arc<Wave>, String>>,
    ) -> Self {
        Self {
            progress_rx,
            thread: Some(thread),
            last_progress: None,
        }
    }

    /// Poll for the latest import progress (non-blocking).
    ///
    /// Drains all pending progress messages and returns the latest one.
    /// If the import thread has finished, returns `Complete` or `Failed`.
    pub fn progress(&mut self) -> ImportStatus {
        // Drain all pending progress messages to get the latest.
        while let Ok(p) = self.progress_rx.try_recv() {
            self.last_progress = Some(p);
        }

        // Check if the thread has finished.
        if let Some(ref thread) = self.thread {
            if thread.is_finished() {
                let thread = self.thread.take().unwrap();
                return match thread.join() {
                    Ok(Ok(wave)) => ImportStatus::Complete(wave),
                    Ok(Err(e)) => ImportStatus::Failed(e),
                    Err(_) => ImportStatus::Failed("Import thread panicked".to_string()),
                };
            }
        } else {
            return ImportStatus::Failed("Import already consumed".to_string());
        }

        match self.last_progress {
            Some(p) => ImportStatus::Running(p),
            None => ImportStatus::Pending,
        }
    }

    /// Block until the import finishes and return the wave.
    pub fn wait(mut self) -> std::result::Result<Arc<Wave>, String> {
        if let Some(thread) = self.thread.take() {
            match thread.join() {
                Ok(result) => result,
                Err(_) => Err("Import thread panicked".to_string()),
            }
        } else {
            Err("Import already consumed".to_string())
        }
    }

    /// Check if the import has finished (non-blocking).
    pub fn is_done(&self) -> bool {
        self.thread
            .as_ref()
            .map(|t| t.is_finished())
            .unwrap_or(true)
    }

    /// Start a background wave import from the given path.
    ///
    /// Returns an `ImportHandle` that can be polled for progress.
    /// Uses `Wave::load_with_progress` on a dedicated thread.
    pub fn start(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let (tx, rx) = crossbeam_channel::bounded(64);

        let thread = std::thread::Builder::new()
            .name("tutti-load-wave".into())
            .spawn(move || {
                let wave = Wave::load_with_progress(&path, |p| {
                    let _ = tx.try_send(p);
                })
                .map_err(|e| e.to_string())?;
                Ok(Arc::new(wave))
            })
            .expect("failed to spawn wave load thread");

        Self::new(rx, thread)
    }

    /// Create an `ImportHandle` that immediately resolves with a cached wave.
    pub fn from_cached(wave: Arc<Wave>) -> Self {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let _ = tx.send(1.0);
        let thread = std::thread::Builder::new()
            .name("tutti-load-wave".into())
            .spawn(move || Ok(wave))
            .expect("failed to spawn thread");
        Self::new(rx, thread)
    }
}
