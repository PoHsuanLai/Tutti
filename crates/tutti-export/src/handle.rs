//! Non-blocking export handle with progress polling.

use crate::export_builder::ExportProgress;
use crossbeam_channel::Receiver;
use std::thread::JoinHandle;

/// Status of a background export operation.
#[derive(Debug)]
pub enum ExportStatus {
    /// Export is in progress.
    Running(ExportProgress),
    /// Export completed successfully.
    Complete,
    /// Export failed with an error message.
    Failed(String),
    /// No progress yet (just started).
    Pending,
}

/// Handle to a background export operation.
///
/// Created by [`ExportBuilder::start()`]. Poll with [`progress()`] each frame.
///
/// # Example
/// ```ignore
/// let mut export = engine.export()
///     .duration_seconds(60.0)
///     .format(AudioFormat::Wav)
///     .start("output.wav");
///
/// // Poll each frame
/// loop {
///     match export.progress() {
///         ExportStatus::Running(p) => println!("{:?} {:.0}%", p.phase, p.progress * 100.0),
///         ExportStatus::Complete => { println!("Done!"); break; }
///         ExportStatus::Failed(e) => { eprintln!("Error: {}", e); break; }
///         ExportStatus::Pending => {}
///     }
/// }
/// ```
pub struct ExportHandle {
    progress_rx: Receiver<ExportProgress>,
    thread: Option<JoinHandle<crate::Result<()>>>,
    last_progress: Option<ExportProgress>,
}

impl ExportHandle {
    pub(crate) fn new(
        progress_rx: Receiver<ExportProgress>,
        thread: JoinHandle<crate::Result<()>>,
    ) -> Self {
        Self {
            progress_rx,
            thread: Some(thread),
            last_progress: None,
        }
    }

    /// Poll for the latest export progress (non-blocking).
    ///
    /// Drains all pending progress messages and returns the latest one.
    /// If the export thread has finished, returns `Complete` or `Failed`.
    pub fn progress(&mut self) -> ExportStatus {
        // Drain all pending progress messages to get the latest
        while let Ok(p) = self.progress_rx.try_recv() {
            self.last_progress = Some(p);
        }

        // Check if the thread has finished
        if let Some(ref thread) = self.thread {
            if thread.is_finished() {
                let thread = self.thread.take().unwrap();
                return match thread.join() {
                    Ok(Ok(())) => ExportStatus::Complete,
                    Ok(Err(e)) => ExportStatus::Failed(e.to_string()),
                    Err(_) => ExportStatus::Failed("Export thread panicked".to_string()),
                };
            }
        } else {
            // Thread already joined — we already returned Complete/Failed
            return ExportStatus::Complete;
        }

        // Still running — return latest progress
        match self.last_progress {
            Some(p) => ExportStatus::Running(p),
            None => ExportStatus::Pending,
        }
    }

    /// Block until the export finishes and return the result.
    pub fn wait(mut self) -> crate::Result<()> {
        if let Some(thread) = self.thread.take() {
            match thread.join() {
                Ok(result) => result,
                Err(_) => Err(crate::ExportError::Render("Export thread panicked".into())),
            }
        } else {
            Ok(())
        }
    }

    /// Check if the export has finished (non-blocking).
    pub fn is_done(&self) -> bool {
        self.thread
            .as_ref()
            .map(|t| t.is_finished())
            .unwrap_or(true)
    }
}
