//! Non-blocking audio file import with progress polling.

use crossbeam_channel::Receiver;
use std::path::Path;
use std::sync::Arc;
use std::thread::JoinHandle;
use tutti_core::Wave;

pub enum ImportStatus {
    /// Progress 0.0..1.0.
    Running(f32),
    Complete(Arc<Wave>),
    Failed(String),
    Pending,
}

/// Handle to a background audio file import.
/// Poll with [`progress()`] each frame.
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
    pub fn progress(&mut self) -> ImportStatus {
        while let Ok(p) = self.progress_rx.try_recv() {
            self.last_progress = Some(p);
        }

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

    pub fn is_done(&self) -> bool {
        self.thread
            .as_ref()
            .map(|t| t.is_finished())
            .unwrap_or(true)
    }

    /// Start a background wave import on a dedicated thread.
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

    /// Immediately resolves with a cached wave.
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
