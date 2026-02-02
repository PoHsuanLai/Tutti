//! MIDI Output Manager
//!
//! Handles MIDI device enumeration, connection, and message sending.
//! Uses dedicated thread for platform thread-safety.

#![cfg_attr(not(feature = "midi-io"), allow(unused_imports, dead_code))]

use crate::MidiEvent;
use crossbeam_channel::{bounded, Receiver, Sender};
use midi_msg::MidiMsg;
#[cfg(feature = "midi-io")]
use midir::{MidiOutput, MidiOutputConnection};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use tracing::debug;

/// MIDI message to send to output device
#[derive(Debug, Clone)]
pub struct MidiOutputMessage {
    /// Raw MIDI bytes to send
    pub bytes: Vec<u8>,
}

impl MidiOutputMessage {
    /// Create a Control Change message
    pub fn control_change(channel: u8, cc_number: u8, value: u8) -> Self {
        let channel = channel.min(15); // MIDI channels are 0-15
        let status = 0xB0 | channel;
        Self {
            bytes: vec![status, cc_number & 0x7F, value & 0x7F],
        }
    }

    /// Create a Note On message
    pub fn note_on(channel: u8, note: u8, velocity: u8) -> Self {
        let channel = channel.min(15);
        let status = 0x90 | channel;
        Self {
            bytes: vec![status, note & 0x7F, velocity & 0x7F],
        }
    }

    /// Create a Note Off message
    pub fn note_off(channel: u8, note: u8, velocity: u8) -> Self {
        let channel = channel.min(15);
        let status = 0x80 | channel;
        Self {
            bytes: vec![status, note & 0x7F, velocity & 0x7F],
        }
    }

    /// Create a Program Change message
    pub fn program_change(channel: u8, program: u8) -> Self {
        let channel = channel.min(15);
        let status = 0xC0 | channel;
        Self {
            bytes: vec![status, program & 0x7F],
        }
    }

    /// Create a Pitch Bend message
    pub fn pitch_bend(channel: u8, value: i16) -> Self {
        let channel = channel.min(15);
        let status = 0xE0 | channel;
        // Convert signed value (-8192 to 8191) to unsigned 14-bit (0 to 16383)
        let unsigned = (value + 8192).clamp(0, 16383) as u16;
        let lsb = (unsigned & 0x7F) as u8;
        let msb = ((unsigned >> 7) & 0x7F) as u8;
        Self {
            bytes: vec![status, lsb, msb],
        }
    }

    /// Create from a MidiEvent (using midi-msg serialization)
    pub fn from_event(event: &MidiEvent) -> Self {
        let msg = MidiMsg::ChannelVoice {
            channel: event.channel,
            msg: event.msg,
        };
        Self {
            bytes: msg.to_midi(),
        }
    }
}

impl From<&MidiEvent> for MidiOutputMessage {
    fn from(event: &MidiEvent) -> Self {
        Self::from_event(event)
    }
}

impl From<MidiEvent> for MidiOutputMessage {
    fn from(event: MidiEvent) -> Self {
        Self::from_event(&event)
    }
}

/// Information about an available MIDI output device
#[derive(Debug, Clone)]
pub struct MidiOutputDevice {
    /// Device index (for connection)
    pub index: usize,
    /// Device name
    pub name: String,
}

/// Commands sent to the MIDI output thread
enum MidiOutputCommand {
    Connect(usize),
    Disconnect,
    SendMessage(MidiOutputMessage),
    Shutdown,
}

/// MIDI output manager with async message sending.
#[derive(Clone)]
#[cfg(feature = "midi-io")]
pub struct MidiOutputManager {
    command_sender: Sender<MidiOutputCommand>,
    connected_device: Arc<arc_swap::ArcSwap<Option<String>>>,
    is_connected: Arc<AtomicBool>,
}

#[cfg(feature = "midi-io")]
impl MidiOutputManager {
    pub fn new() -> Self {
        let (command_sender, command_receiver) = bounded(1024);
        let connected_device = Arc::new(arc_swap::ArcSwap::new(Arc::new(None)));
        let is_connected = Arc::new(AtomicBool::new(false));

        let connected_device_clone = Arc::clone(&connected_device);
        let is_connected_clone = Arc::clone(&is_connected);

        thread::Builder::new()
            .name("midi-output-thread".to_string())
            .spawn(move || {
                Self::midi_output_thread(
                    command_receiver,
                    connected_device_clone,
                    is_connected_clone,
                );
            })
            .expect("Failed to spawn MIDI output thread");

        Self {
            command_sender,
            connected_device,
            is_connected,
        }
    }

    fn midi_output_thread(
        command_receiver: Receiver<MidiOutputCommand>,
        connected_device: Arc<arc_swap::ArcSwap<Option<String>>>,
        is_connected: Arc<AtomicBool>,
    ) {
        let mut connection: Option<MidiOutputConnection> = None;

        loop {
            match command_receiver.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(MidiOutputCommand::Connect(device_index)) => {
                    // Disconnect existing connection if any
                    if let Some(conn) = connection.take() {
                        drop(conn);
                    }

                    // Attempt to connect to new device
                    match Self::connect_to_device(device_index) {
                        Ok((conn, name)) => {
                            connection = Some(conn);
                            is_connected.store(true, Ordering::SeqCst);
                            connected_device.store(Arc::new(Some(name.clone())));
                        }
                        Err(_e) => {
                            is_connected.store(false, Ordering::SeqCst);
                            connected_device.store(Arc::new(None));
                        }
                    }
                }
                Ok(MidiOutputCommand::Disconnect) => {
                    if let Some(conn) = connection.take() {
                        drop(conn);
                        is_connected.store(false, Ordering::SeqCst);
                        connected_device.store(Arc::new(None));
                    }
                }
                Ok(MidiOutputCommand::SendMessage(msg)) => {
                    if let Some(ref mut conn) = connection {
                        let _ = conn.send(&msg.bytes);
                    } else {
                        debug!("Cannot send MIDI message: no device connected");
                    }
                }
                Ok(MidiOutputCommand::Shutdown) => {
                    // Clean up and exit thread
                    if let Some(conn) = connection.take() {
                        drop(conn);
                    }
                    break;
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    // No command, continue running
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    // Channel closed, exit thread
                    break;
                }
            }
        }
    }

    fn connect_to_device(device_index: usize) -> Result<(MidiOutputConnection, String), String> {
        let midi_output = MidiOutput::new("dawai-midi-output")
            .map_err(|e| format!("Failed to create MIDI output: {}", e))?;

        let ports = midi_output.ports();
        let port = ports
            .get(device_index)
            .ok_or_else(|| format!("MIDI output device {} not found", device_index))?;

        let port_name = midi_output
            .port_name(port)
            .unwrap_or_else(|_| format!("Device {}", device_index));

        let connection = midi_output
            .connect(port, "dawai-output")
            .map_err(|e| format!("Failed to connect to MIDI output device: {}", e))?;

        Ok((connection, port_name))
    }

    pub fn list_devices() -> Vec<MidiOutputDevice> {
        let mut devices = Vec::new();
        if let Ok(midi_output) = MidiOutput::new("dawai-device-list") {
            let ports = midi_output.ports();
            for (index, port) in ports.iter().enumerate() {
                let name = midi_output
                    .port_name(port)
                    .unwrap_or_else(|_| format!("Unknown Device {}", index));
                devices.push(MidiOutputDevice { index, name });
            }
        }
        devices
    }

    pub fn connect(&self, device_index: usize) -> Result<(), String> {
        self.command_sender
            .send(MidiOutputCommand::Connect(device_index))
            .map_err(|_| "MIDI output thread not running".to_string())
    }

    pub fn connect_by_name(&self, name: &str) -> Result<(), String> {
        let devices = Self::list_devices();
        let device = devices
            .iter()
            .find(|d| d.name.to_lowercase().contains(&name.to_lowercase()))
            .ok_or_else(|| format!("No MIDI output device found matching '{}'", name))?;
        self.connect(device.index)
    }

    pub fn disconnect(&self) {
        let _ = self.command_sender.send(MidiOutputCommand::Disconnect);
    }

    pub fn send_message(&self, message: MidiOutputMessage) {
        if let Err(e) = self
            .command_sender
            .try_send(MidiOutputCommand::SendMessage(message))
        {
            debug!("MIDI output command channel full or disconnected: {}", e);
        }
    }

    pub fn send_cc(&self, channel: u8, cc_number: u8, value: u8) {
        self.send_message(MidiOutputMessage::control_change(channel, cc_number, value));
    }

    pub fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::SeqCst)
    }

    pub fn connected_device_name(&self) -> Option<String> {
        self.connected_device.load().as_ref().clone()
    }
}

#[cfg(feature = "midi-io")]
impl Default for MidiOutputManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "midi-io")]
impl Drop for MidiOutputManager {
    fn drop(&mut self) {
        let _ = self.command_sender.send(MidiOutputCommand::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cc_message_creation() {
        let msg = MidiOutputMessage::control_change(0, 7, 127);
        assert_eq!(msg.bytes, vec![0xB0, 7, 127]);

        let msg = MidiOutputMessage::control_change(15, 64, 0);
        assert_eq!(msg.bytes, vec![0xBF, 64, 0]);
    }

    #[test]
    fn test_note_on_message() {
        let msg = MidiOutputMessage::note_on(0, 60, 100);
        assert_eq!(msg.bytes, vec![0x90, 60, 100]);
    }

    #[test]
    fn test_note_off_message() {
        let msg = MidiOutputMessage::note_off(0, 60, 64);
        assert_eq!(msg.bytes, vec![0x80, 60, 64]);
    }

    #[test]
    fn test_pitch_bend_message() {
        // Center (no bend)
        let msg = MidiOutputMessage::pitch_bend(0, 0);
        assert_eq!(msg.bytes[0], 0xE0);
        assert_eq!((msg.bytes[1] as u16) | ((msg.bytes[2] as u16) << 7), 8192);

        // Max bend up
        let msg = MidiOutputMessage::pitch_bend(0, 8191);
        assert_eq!(msg.bytes[0], 0xE0);
        assert_eq!((msg.bytes[1] as u16) | ((msg.bytes[2] as u16) << 7), 16383);

        // Max bend down
        let msg = MidiOutputMessage::pitch_bend(0, -8192);
        assert_eq!(msg.bytes[0], 0xE0);
        assert_eq!((msg.bytes[1] as u16) | ((msg.bytes[2] as u16) << 7), 0);
    }

    #[test]
    fn test_list_devices() {
        // This test may fail if no MIDI devices are available, but shouldn't crash
        let devices = MidiOutputManager::list_devices();
        println!("Found {} MIDI output devices", devices.len());
        for device in &devices {
            println!("  {}: {}", device.index, device.name);
        }
    }
}
