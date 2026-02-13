//! MIDI output: device enumeration, connection, and message sending via a dedicated thread.

#![cfg_attr(not(feature = "midi-io"), allow(unused_imports, dead_code))]

use crate::MidiEvent;
use crate::MidiMsg;
use crossbeam_channel::{bounded, Receiver, Sender};
#[cfg(feature = "midi-io")]
use midir::{MidiOutput, MidiOutputConnection};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use tracing::debug;

#[derive(Debug, Clone)]
pub struct MidiOutputMessage {
    pub bytes: Vec<u8>,
}

impl MidiOutputMessage {
    pub fn control_change(channel: u8, cc_number: u8, value: u8) -> Self {
        let channel = channel.min(15); // MIDI channels are 0-15
        let status = 0xB0 | channel;
        Self {
            bytes: vec![status, cc_number & 0x7F, value & 0x7F],
        }
    }

    pub fn note_on(channel: u8, note: u8, velocity: u8) -> Self {
        let channel = channel.min(15);
        let status = 0x90 | channel;
        Self {
            bytes: vec![status, note & 0x7F, velocity & 0x7F],
        }
    }

    pub fn note_off(channel: u8, note: u8, velocity: u8) -> Self {
        let channel = channel.min(15);
        let status = 0x80 | channel;
        Self {
            bytes: vec![status, note & 0x7F, velocity & 0x7F],
        }
    }

    pub fn program_change(channel: u8, program: u8) -> Self {
        let channel = channel.min(15);
        let status = 0xC0 | channel;
        Self {
            bytes: vec![status, program & 0x7F],
        }
    }

    /// `value`: signed 14-bit (-8192 to 8191).
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

#[derive(Debug, Clone)]
pub struct MidiOutputDevice {
    pub index: usize,
    pub name: String,
}

enum MidiOutputCommand {
    Connect(usize),
    Disconnect,
    SendMessage(MidiOutputMessage),
    Shutdown,
}

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
                    if let Some(conn) = connection.take() {
                        drop(conn);
                    }

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
                    if let Some(conn) = connection.take() {
                        drop(conn);
                    }
                    break;
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }
        }
    }

    fn connect_to_device(
        device_index: usize,
    ) -> Result<(MidiOutputConnection, String), crate::error::Error> {
        let midi_output = MidiOutput::new("dawai-midi-output")?;

        let ports = midi_output.ports();
        let port = ports.get(device_index).ok_or_else(|| {
            crate::error::Error::MidiDevice(format!(
                "MIDI output device {} not found",
                device_index
            ))
        })?;

        let port_name = midi_output
            .port_name(port)
            .unwrap_or_else(|_| format!("Device {}", device_index));

        let connection = midi_output.connect(port, "dawai-output")?;

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

    pub fn connect(&self, device_index: usize) -> crate::error::Result<()> {
        self.command_sender
            .send(MidiOutputCommand::Connect(device_index))
            .map_err(|_| {
                crate::error::Error::MidiDevice("MIDI output thread not running".to_string())
            })
    }

    pub fn connect_by_name(&self, name: &str) -> crate::error::Result<()> {
        let devices = Self::list_devices();
        let device = devices
            .iter()
            .find(|d| d.name.to_lowercase().contains(&name.to_lowercase()))
            .ok_or_else(|| {
                crate::error::Error::MidiDevice(format!(
                    "No MIDI output device found matching '{}'",
                    name
                ))
            })?;
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
    fn test_program_change_message() {
        let msg = MidiOutputMessage::program_change(0, 42);
        assert_eq!(msg.bytes, vec![0xC0, 42]);

        let msg = MidiOutputMessage::program_change(9, 0);
        assert_eq!(msg.bytes, vec![0xC9, 0]);

        let msg = MidiOutputMessage::program_change(15, 127);
        assert_eq!(msg.bytes, vec![0xCF, 127]);
    }

    #[test]
    fn test_channel_clamping_all_constructors() {
        // Channel > 15 should clamp to 15
        let msg = MidiOutputMessage::control_change(200, 7, 127);
        assert_eq!(msg.bytes[0], 0xBF); // 0xB0 | 15

        let msg = MidiOutputMessage::note_on(255, 60, 100);
        assert_eq!(msg.bytes[0], 0x9F); // 0x90 | 15

        let msg = MidiOutputMessage::note_off(16, 60, 0);
        assert_eq!(msg.bytes[0], 0x8F); // 0x80 | 15

        let msg = MidiOutputMessage::program_change(200, 42);
        assert_eq!(msg.bytes[0], 0xCF); // 0xC0 | 15

        let msg = MidiOutputMessage::pitch_bend(128, 0);
        assert_eq!(msg.bytes[0], 0xEF); // 0xE0 | 15
    }

    #[test]
    fn test_data_byte_masking() {
        // Data bytes > 127 should be masked to 7-bit
        let msg = MidiOutputMessage::control_change(0, 0xFF, 0xFF);
        assert_eq!(msg.bytes[1], 0x7F);
        assert_eq!(msg.bytes[2], 0x7F);

        let msg = MidiOutputMessage::note_on(0, 0xFF, 0xFF);
        assert_eq!(msg.bytes[1], 0x7F);
        assert_eq!(msg.bytes[2], 0x7F);

        let msg = MidiOutputMessage::program_change(0, 0xFF);
        assert_eq!(msg.bytes[1], 0x7F);
    }

    #[test]
    fn test_from_event_note_on() {
        let event = MidiEvent::note_on(0, 5, 60, 100);
        let msg = MidiOutputMessage::from_event(&event);

        // Status: 0x90 | 5 = 0x95
        assert_eq!(msg.bytes[0], 0x95);
        assert_eq!(msg.bytes[1], 60);
        assert_eq!(msg.bytes[2], 100);
    }

    #[test]
    fn test_from_event_note_off() {
        let event = MidiEvent::note_off(0, 3, 64, 0);
        let msg = MidiOutputMessage::from_event(&event);

        assert_eq!(msg.bytes[0], 0x83); // 0x80 | 3
        assert_eq!(msg.bytes[1], 64);
        assert_eq!(msg.bytes[2], 0);
    }

    #[test]
    fn test_from_event_cc() {
        let event = MidiEvent::control_change(0, 0, 7, 127);
        let msg = MidiOutputMessage::from_event(&event);

        assert_eq!(msg.bytes[0], 0xB0);
        assert_eq!(msg.bytes[1], 7);
        assert_eq!(msg.bytes[2], 127);
    }

    #[test]
    fn test_from_trait_impls() {
        let event = MidiEvent::note_on(0, 0, 60, 100);

        // From<&MidiEvent>
        let msg: MidiOutputMessage = (&event).into();
        assert_eq!(msg.bytes[0], 0x90);
        assert_eq!(msg.bytes[1], 60);

        // From<MidiEvent>
        let msg: MidiOutputMessage = event.into();
        assert_eq!(msg.bytes[0], 0x90);
        assert_eq!(msg.bytes[1], 60);
    }

    #[test]
    fn test_pitch_bend_clamping() {
        // Values beyond range should clamp
        let msg = MidiOutputMessage::pitch_bend(0, 10000);
        let unsigned = (msg.bytes[1] as u16) | ((msg.bytes[2] as u16) << 7);
        assert_eq!(unsigned, 16383, "Should clamp to max");

        let msg = MidiOutputMessage::pitch_bend(0, -10000);
        let unsigned = (msg.bytes[1] as u16) | ((msg.bytes[2] as u16) << 7);
        assert_eq!(unsigned, 0, "Should clamp to min");
    }
}
