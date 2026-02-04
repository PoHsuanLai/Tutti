//! Transport integration for butler thread.
//!
//! Bridges TransportManager events to butler commands for synchronized playback.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossbeam_channel::Sender;
use dashmap::DashMap;
use tutti_core::{MotionState, SyncSource, SyncStatus, TransportManager};

use super::request::ButlerCommand;
use super::stream_state::ChannelStreamState;

/// Slaved mode buffer margin multiplier.
/// When synced to external source, use larger buffers to handle jitter.
const SLAVED_BUFFER_MARGIN: f64 = 1.5;

/// Transport bridge that syncs butler state with TransportManager.
///
/// Runs a polling loop that detects transport state changes and
/// translates them to butler commands for ALL active streaming channels.
pub struct TransportBridge {
    running: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl TransportBridge {
    /// Create and start a transport bridge.
    ///
    /// The bridge polls the transport manager and sends commands to the butler.
    /// Polling interval is 5ms for responsive transport sync.
    ///
    /// Commands (seek, loop) are broadcast to all active streaming channels.
    pub(crate) fn new(
        transport: Arc<TransportManager>,
        butler_tx: Sender<ButlerCommand>,
        stream_states: Arc<DashMap<usize, ChannelStreamState>>,
        sample_rate: f64,
    ) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();

        let thread = thread::spawn(move || {
            run_bridge(
                transport,
                butler_tx,
                stream_states,
                sample_rate,
                running_clone,
            );
        });

        Self {
            running,
            thread: Some(thread),
        }
    }

    /// Stop the transport bridge.
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for TransportBridge {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Bridge main loop.
fn run_bridge(
    transport: Arc<TransportManager>,
    butler_tx: Sender<ButlerCommand>,
    stream_states: Arc<DashMap<usize, ChannelStreamState>>,
    sample_rate: f64,
    running: Arc<AtomicBool>,
) {
    let mut last_motion = MotionState::Stopped;
    let mut last_beat = 0.0f64;
    let mut last_loop_enabled = false;
    let mut last_loop_range: Option<(f64, f64)> = None;
    let mut last_sync_source = SyncSource::Internal;
    let mut last_sync_locked = false;

    while running.load(Ordering::Acquire) {
        let sync_snap = transport.sync_snapshot();
        let sync_source_changed = sync_snap.source != last_sync_source;
        let sync_lock_changed = (sync_snap.status == SyncStatus::Locked) != last_sync_locked;

        if sync_source_changed || sync_lock_changed {
            let is_slaved = sync_snap.source != SyncSource::Internal
                && sync_snap.status == SyncStatus::Locked
                && sync_snap.following;

            let buffer_margin = if is_slaved { SLAVED_BUFFER_MARGIN } else { 1.0 };

            let _ = butler_tx.try_send(ButlerCommand::SetBufferMargin {
                margin: buffer_margin,
            });

            if is_slaved && sync_snap.external_position > 0.0 {
                let external_samples = (sync_snap.external_position * sample_rate * 60.0
                    / sync_snap.external_tempo as f64)
                    as u64;

                for entry in stream_states.iter() {
                    let channel_index = *entry.key();
                    if entry.value().is_streaming() {
                        let _ = butler_tx.try_send(ButlerCommand::SeekStream {
                            channel_index,
                            position_samples: external_samples,
                        });
                    }
                }
            }

            last_sync_source = sync_snap.source;
            last_sync_locked = sync_snap.status == SyncStatus::Locked;
        }

        let motion = transport.motion_state();
        if motion != last_motion {
            match motion {
                MotionState::Rolling => {
                    let _ = butler_tx.try_send(ButlerCommand::Run);
                }
                MotionState::Stopped | MotionState::DeclickToStop => {
                    let _ = butler_tx.try_send(ButlerCommand::Pause);
                }
                _ => {}
            }
            last_motion = motion;
        }

        let current_beat = transport.get_current_beat();
        let beat_delta = (current_beat - last_beat).abs();

        if beat_delta > 0.5 && motion != MotionState::Rolling {
            let position_samples = transport.beats_to_samples(current_beat);

            for entry in stream_states.iter() {
                let channel_index = *entry.key();
                if entry.value().is_streaming() {
                    let _ = butler_tx.try_send(ButlerCommand::SeekStream {
                        channel_index,
                        position_samples,
                    });
                }
            }
        }
        last_beat = current_beat;

        let loop_enabled = transport.is_loop_enabled();
        let loop_range = transport.get_loop_range();

        if loop_enabled != last_loop_enabled || loop_range != last_loop_range {
            for entry in stream_states.iter() {
                let channel_index = *entry.key();
                if entry.value().is_streaming() {
                    if loop_enabled {
                        if let Some((start, end)) = loop_range {
                            let start_samples = transport.beats_to_samples(start);
                            let end_samples = transport.beats_to_samples(end);
                            let _ = butler_tx.try_send(ButlerCommand::SetLoopRange {
                                channel_index,
                                start_samples,
                                end_samples,
                                crossfade_samples: 64,
                            });
                        }
                    } else {
                        let _ = butler_tx.try_send(ButlerCommand::ClearLoopRange { channel_index });
                    }
                }
            }
            last_loop_enabled = loop_enabled;
            last_loop_range = loop_range;
        }

        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transport_bridge_creation() {
        let transport = Arc::new(TransportManager::default());
        let samples = transport.beats_to_samples(1.0);
        assert!(samples > 0);
    }
}
