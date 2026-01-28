//! Audio input management for recording from hardware devices.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use tutti_core::AtomicFloat;
use cpal::traits::{DeviceTrait, HostTrait};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::Arc;

/// Sentinel value for "no device selected"
const NO_DEVICE_SELECTED: usize = usize::MAX;

/// Input device information
#[derive(Debug, Clone)]
pub struct InputDeviceInfo {
    /// Device index
    pub index: usize,
    /// Device name
    pub name: String,
    /// Number of input channels
    pub channels: u16,
    /// Supported sample rates
    pub sample_rates: Vec<u32>,
}

/// Thread-safe audio input manager.
pub struct AudioInputManager {
    input_sender: Option<Sender<(f32, f32)>>,
    input_receiver: Option<Arc<Receiver<(f32, f32)>>>,
    monitoring_enabled: Arc<AtomicBool>,
    input_gain: Arc<AtomicFloat>,
    peak_level: Arc<AtomicFloat>,
    is_capturing: Arc<AtomicBool>,
    sample_rate: u32,
    dropped_samples: Arc<AtomicU32>,
    selected_device: Arc<AtomicUsize>,
    start_requested: Arc<AtomicBool>,
    stop_requested: Arc<AtomicBool>,
}

impl Clone for AudioInputManager {
    fn clone(&self) -> Self {
        Self {
            input_sender: None, // Sender is moved to callback, can't be cloned
            input_receiver: self.input_receiver.clone(),
            monitoring_enabled: Arc::clone(&self.monitoring_enabled),
            input_gain: Arc::clone(&self.input_gain),
            peak_level: Arc::clone(&self.peak_level),
            is_capturing: Arc::clone(&self.is_capturing),
            sample_rate: self.sample_rate,
            dropped_samples: Arc::clone(&self.dropped_samples),
            selected_device: Arc::clone(&self.selected_device), // Clone Arc, not value
            start_requested: Arc::clone(&self.start_requested),
            stop_requested: Arc::clone(&self.stop_requested),
        }
    }
}

impl AudioInputManager {
    /// Create a new audio input state
    pub fn new(sample_rate: u32) -> Self {
        Self {
            input_sender: None,
            input_receiver: None,
            monitoring_enabled: Arc::new(AtomicBool::new(false)),
            input_gain: Arc::new(AtomicFloat::new(1.0)),
            peak_level: Arc::new(AtomicFloat::new(0.0)),
            is_capturing: Arc::new(AtomicBool::new(false)),
            sample_rate,
            dropped_samples: Arc::new(AtomicU32::new(0)),
            selected_device: Arc::new(AtomicUsize::new(NO_DEVICE_SELECTED)),
            start_requested: Arc::new(AtomicBool::new(false)),
            stop_requested: Arc::new(AtomicBool::new(false)),
        }
    }

    /// List available input devices
    pub fn list_input_devices(&self) -> Vec<InputDeviceInfo> {
        let host = cpal::default_host();
        let mut devices = Vec::new();

        if let Ok(input_devices) = host.input_devices() {
            for (idx, device) in input_devices.enumerate() {
                let name = device.name().unwrap_or_else(|_| "Unknown".to_string());

                let (channels, sample_rates) = if let Ok(config) = device.default_input_config() {
                    let channels = config.channels();
                    let sample_rates = vec![config.sample_rate().0];
                    (channels, sample_rates)
                } else {
                    (2, vec![44100])
                };

                devices.push(InputDeviceInfo {
                    index: idx,
                    name,
                    channels,
                    sample_rates,
                });
            }
        }

        devices
    }

    /// Get the default input device info
    pub fn default_device_info(&self) -> Option<InputDeviceInfo> {
        let host = cpal::default_host();
        let device = host.default_input_device()?;
        let name = device.name().unwrap_or_else(|_| "Default".to_string());
        let config = device.default_input_config().ok()?;

        Some(InputDeviceInfo {
            index: 0,
            name,
            channels: config.channels(),
            sample_rates: vec![config.sample_rate().0],
        })
    }

    /// Select an input device by index
    /// Lock-free: Uses AtomicUsize with Release ordering
    pub fn select_device(&self, device_index: usize) -> crate::Result<()> {
        let host = cpal::default_host();
        let devices: Vec<_> = host.input_devices()?.collect();

        if device_index >= devices.len() {
            return Err(crate::Error::DeviceNotFound(format!(
                "Device index {} out of range (0-{})",
                device_index,
                devices.len().saturating_sub(1)
            )));
        }

        // Release ordering: ensures device index is visible to other threads
        self.selected_device.store(device_index, Ordering::Release);
        Ok(())
    }

    /// Request to start capturing (will be picked up by stream manager)
    pub fn request_start(&mut self) {
        // Create lock-free MPMC channel (500ms buffer at current sample rate)
        let buffer_size = (self.sample_rate as usize) / 2;
        let (tx, rx) = bounded(buffer_size);

        self.input_sender = Some(tx); // Will be moved to callback
        self.input_receiver = Some(Arc::new(rx)); // Can be cloned for multiple readers!

        self.start_requested.store(true, Ordering::Release);
    }

    /// Request to stop capturing
    pub fn request_stop(&mut self) {
        self.stop_requested.store(true, Ordering::Release);
    }

    /// Check if start was requested (and clear the flag)
    pub fn take_start_request(&self) -> bool {
        self.start_requested.swap(false, Ordering::AcqRel)
    }

    /// Check if stop was requested (and clear the flag)
    pub fn take_stop_request(&self) -> bool {
        self.stop_requested.swap(false, Ordering::AcqRel)
    }

    /// Take the input sender (moves ownership to stream callback)
    pub fn take_input_sender(&mut self) -> Option<Sender<(f32, f32)>> {
        self.input_sender.take()
    }

    /// Mark as capturing
    pub fn set_capturing(&self, capturing: bool) {
        self.is_capturing.store(capturing, Ordering::Release);
    }

    /// Check if currently capturing
    pub fn is_capturing(&self) -> bool {
        self.is_capturing.load(Ordering::Acquire)
    }

    /// Check if input is running (alias for is_capturing)
    pub fn is_running(&self) -> bool {
        self.is_capturing()
    }

    /// Enable/disable input monitoring (hear yourself)
    pub fn set_monitoring(&self, enabled: bool) {
        self.monitoring_enabled.store(enabled, Ordering::Release);
    }

    /// Check if monitoring is enabled
    pub fn is_monitoring(&self) -> bool {
        self.monitoring_enabled.load(Ordering::Acquire)
    }

    /// Set input gain (0.0 to 2.0)
    pub fn set_gain(&self, gain: f32) {
        self.input_gain.set(gain.clamp(0.0, 2.0));
    }

    /// Get current input gain
    pub fn gain(&self) -> f32 {
        self.input_gain.get()
    }

    /// Get current peak level (for metering)
    pub fn peak_level(&self) -> f32 {
        self.peak_level.get()
    }

    /// Get input gain Arc (for callback)
    pub fn input_gain_arc(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.input_gain)
    }

    /// Get peak level Arc (for callback)
    pub fn peak_level_arc(&self) -> Arc<AtomicFloat> {
        Arc::clone(&self.peak_level)
    }

    /// Get dropped samples Arc (for callback)
    pub fn dropped_samples_arc(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.dropped_samples)
    }

    /// Get the input receiver (for transferring samples to recording)
    ///
    /// **Lock-free MPMC**: Receiver can be cloned for multiple concurrent readers!
    pub fn input_receiver(&self) -> Option<Arc<Receiver<(f32, f32)>>> {
        self.input_receiver.clone()
    }

    /// Get monitoring enabled flag (for audio callback)
    pub fn monitoring_enabled_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.monitoring_enabled)
    }

    /// Get selected device index
    /// Lock-free: Uses AtomicUsize with Acquire ordering
    /// Returns None if no device selected (sentinel value)
    pub fn selected_device(&self) -> Option<usize> {
        // Acquire ordering: ensures we see the latest device index
        let device = self.selected_device.load(Ordering::Acquire);
        if device == NO_DEVICE_SELECTED {
            None
        } else {
            Some(device)
        }
    }

    /// Get sample rate
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Read samples for monitoring (call from output callback if monitoring enabled)
    ///
    /// **Lock-free**: Uses crossbeam's `try_recv()` - zero mutex overhead!
    pub fn read_monitor_sample(&self) -> (f32, f32) {
        if !self.monitoring_enabled.load(Ordering::Acquire) {
            return (0.0, 0.0);
        }

        if let Some(receiver) = &self.input_receiver {
            if let Ok(sample) = receiver.try_recv() {
                return sample;
            }
        }

        (0.0, 0.0)
    }

    /// Get number of dropped samples (for debugging)
    pub fn dropped_samples(&self) -> u32 {
        self.dropped_samples.load(Ordering::Relaxed)
    }

    /// Reset dropped samples counter
    pub fn reset_dropped_samples(&self) {
        self.dropped_samples.store(0, Ordering::Relaxed);
    }
}

/// The actual CPAL input stream (NonSend resource).
///

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_devices() {
        let manager = AudioInputManager::new(44100);
        let devices = manager.list_input_devices();
        println!("Found {} input devices", devices.len());
        for device in &devices {
            println!(
                "  {}: {} ({} ch)",
                device.index, device.name, device.channels
            );
        }
    }

    #[test]
    fn test_gain_clamp() {
        let manager = AudioInputManager::new(44100);
        manager.set_gain(3.0);
        assert_eq!(manager.gain(), 2.0);

        manager.set_gain(-1.0);
        assert_eq!(manager.gain(), 0.0);

        manager.set_gain(0.5);
        assert_eq!(manager.gain(), 0.5);
    }
}
