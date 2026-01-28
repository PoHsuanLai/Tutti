//! MIDI Input Manager
//!
//! Handles MIDI device enumeration, connection, and event routing.
//! Uses dedicated thread for platform thread-safety.

#![cfg_attr(not(feature = "midi-io"), allow(unused_imports, dead_code))]

use super::async_port::InputProducerHandle;
use super::multi_port::MidiPortManager;
use crate::MidiEvent;
use crossbeam_channel::{bounded, Receiver, Sender};
#[cfg(feature = "midi-io")]
use midir::{MidiInput, MidiInputConnection};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use tracing::debug;

/// Information about an available MIDI input device
#[derive(Debug, Clone)]
pub struct MidiInputDevice {
    /// Device index (for connection)
    pub index: usize,
    /// Device name
    pub name: String,
}

/// Commands sent to the MIDI thread
enum MidiCommand {
    Connect(usize, usize, InputProducerHandle), // (device_index, port_index, producer_handle)
    Disconnect,
    Shutdown,
}

/// MIDI input manager with async connection handling.
#[cfg(feature = "midi-io")]
pub struct MidiInputManager {
    port_manager: Arc<MidiPortManager>,
    command_sender: Sender<MidiCommand>,
    connected_device: Arc<arc_swap::ArcSwap<Option<String>>>,
    connected_port: Arc<arc_swap::ArcSwap<Option<usize>>>,
    is_connected: Arc<AtomicBool>,
}

#[cfg(feature = "midi-io")]
impl MidiInputManager {
    pub fn new(port_manager: Arc<MidiPortManager>) -> Self {
        let (command_sender, command_receiver) = bounded(16);
        let connected_device = Arc::new(arc_swap::ArcSwap::new(Arc::new(None)));
        let connected_port = Arc::new(arc_swap::ArcSwap::new(Arc::new(None)));
        let is_connected = Arc::new(AtomicBool::new(false));

        let connected_device_clone = Arc::clone(&connected_device);
        let connected_port_clone = Arc::clone(&connected_port);
        let is_connected_clone = Arc::clone(&is_connected);

        thread::spawn(move || {
            Self::midi_thread(
                command_receiver,
                connected_device_clone,
                connected_port_clone,
                is_connected_clone,
            );
        });

        Self {
            port_manager,
            command_sender,
            connected_device,
            connected_port,
            is_connected,
        }
    }

    fn midi_thread(
        command_receiver: Receiver<MidiCommand>,
        connected_device: Arc<arc_swap::ArcSwap<Option<String>>>,
        connected_port: Arc<arc_swap::ArcSwap<Option<usize>>>,
        is_connected: Arc<AtomicBool>,
    ) {
        let mut connection: Option<MidiInputConnection<()>> = None;

        loop {
            match command_receiver.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(MidiCommand::Connect(device_index, port_index, producer_handle)) => {
                    // Disconnect existing
                    if let Some(conn) = connection.take() {
                        drop(conn);
                        is_connected.store(false, Ordering::SeqCst);
                        connected_device.store(Arc::new(None));
                        connected_port.store(Arc::new(None));
                    }

                    // Try to connect
                    match Self::connect_to_device(device_index, producer_handle) {
                        Ok((conn, name)) => {
                            connection = Some(conn);
                            is_connected.store(true, Ordering::SeqCst);
                            connected_device.store(Arc::new(Some(name.clone())));
                            connected_port.store(Arc::new(Some(port_index)));
                        }
                        Err(_e) => {
                            // Connection failed, state remains disconnected
                        }
                    }
                }
                Ok(MidiCommand::Disconnect) => {
                    if let Some(conn) = connection.take() {
                        drop(conn);
                        is_connected.store(false, Ordering::SeqCst);
                        connected_device.store(Arc::new(None));
                        connected_port.store(Arc::new(None));
                    }
                }
                Ok(MidiCommand::Shutdown) => {
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

    fn connect_to_device(
        device_index: usize,
        producer_handle: InputProducerHandle,
    ) -> Result<(MidiInputConnection<()>, String), String> {
        let midi_input = MidiInput::new("dawai-midi-input")
            .map_err(|e| format!("Failed to create MIDI input: {}", e))?;

        let ports = midi_input.ports();
        let port = ports
            .get(device_index)
            .ok_or_else(|| format!("MIDI device {} not found", device_index))?;

        let port_name = midi_input
            .port_name(port)
            .unwrap_or_else(|_| format!("Device {}", device_index));

        let connection = midi_input
            .connect(
                port,
                "dawai-input",
                move |_timestamp, message, _| {
                    // Parse raw MIDI bytes into structured MidiEvent
                    match MidiEvent::from_bytes(message) {
                        Ok(event) => {
                            if !producer_handle.push(event) {
                                debug!("MIDI input ring buffer full, dropping event");
                            }
                        }
                        Err(e) => {
                            debug!("Failed to parse MIDI event: {:?}", e);
                        }
                    }
                },
                (),
            )
            .map_err(|e| format!("Failed to connect to MIDI device: {}", e))?;

        Ok((connection, port_name))
    }

    pub fn list_devices() -> Vec<MidiInputDevice> {
        let mut devices = Vec::new();
        if let Ok(midi_input) = MidiInput::new("dawai-device-list") {
            let ports = midi_input.ports();
            for (index, port) in ports.iter().enumerate() {
                let name = midi_input
                    .port_name(port)
                    .unwrap_or_else(|_| format!("Unknown Device {}", index));
                devices.push(MidiInputDevice { index, name });
            }
        }
        devices
    }

    pub fn connect(&self, device_index: usize) -> Result<(), String> {
        let devices = Self::list_devices();
        let device_name = devices
            .get(device_index)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| format!("MIDI Device {}", device_index));

        let port_index = self.port_manager.create_input_port(&device_name);
        let producer_handle = self
            .port_manager
            .get_input_producer_handle(port_index)
            .ok_or_else(|| "Failed to get producer handle".to_string())?;

        self.command_sender
            .send(MidiCommand::Connect(
                device_index,
                port_index,
                producer_handle,
            ))
            .map_err(|_| "MIDI thread not running".to_string())
    }

    pub fn connect_by_name(&self, name: &str) -> Result<(), String> {
        let devices = Self::list_devices();
        let device = devices
            .iter()
            .find(|d| d.name.to_lowercase().contains(&name.to_lowercase()))
            .ok_or_else(|| format!("No MIDI device matching '{}' found", name))?;
        self.connect(device.index)
    }

    pub fn disconnect(&self) {
        let _ = self.command_sender.send(MidiCommand::Disconnect);
    }

    pub fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::SeqCst)
    }

    pub fn connected_device_name(&self) -> Option<String> {
        self.connected_device.load().as_ref().clone()
    }

    pub fn connected_port_index(&self) -> Option<usize> {
        *self.connected_port.load().as_ref()
    }
}

#[cfg(feature = "midi-io")]
impl Drop for MidiInputManager {
    fn drop(&mut self) {
        let _ = self.command_sender.send(MidiCommand::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midi_msg::{Channel, ChannelVoiceMsg, ControlChange};

    #[test]
    fn test_parse_note_on() {
        let bytes = [0x90, 60, 100]; // Note On, channel 0, note 60, velocity 100
        let event = MidiEvent::from_bytes(&bytes).unwrap();
        assert_eq!(event.channel, Channel::Ch1);
        match event.msg {
            ChannelVoiceMsg::NoteOn { note, velocity } => {
                assert_eq!(note, 60);
                assert_eq!(velocity, 100);
            }
            _ => panic!("Expected NoteOn"),
        }
    }

    #[test]
    fn test_parse_note_on_velocity_zero() {
        let bytes = [0x90, 60, 0]; // Note On with velocity 0 = Note Off semantically
        let event = MidiEvent::from_bytes(&bytes).unwrap();
        assert_eq!(event.channel, Channel::Ch1);
        // Note: midi-msg parses this as NoteOn with velocity 0, not NoteOff
        // Our is_note_off() helper treats it as note off
        assert!(event.is_note_off());
        assert_eq!(event.note(), Some(60));
    }

    #[test]
    fn test_parse_note_off() {
        let bytes = [0x80, 60, 64]; // Note Off, channel 0, note 60
        let event = MidiEvent::from_bytes(&bytes).unwrap();
        assert_eq!(event.channel, Channel::Ch1);
        match event.msg {
            ChannelVoiceMsg::NoteOff { note, .. } => {
                assert_eq!(note, 60);
            }
            _ => panic!("Expected NoteOff"),
        }
    }

    #[test]
    fn test_parse_control_change() {
        let bytes = [0xB0, 7, 100]; // CC, channel 0, control 7 (volume), value 100
        let event = MidiEvent::from_bytes(&bytes).unwrap();
        assert_eq!(event.channel, Channel::Ch1);
        match event.msg {
            ChannelVoiceMsg::ControlChange { control } => match control {
                ControlChange::CC { control: cc, value } => {
                    assert_eq!(cc, 7);
                    assert_eq!(value, 100);
                }
                _ => panic!("Expected CC"),
            },
            _ => panic!("Expected ControlChange"),
        }
    }

    #[test]
    fn test_parse_pitch_bend() {
        let bytes = [0xE0, 0, 64]; // Pitch bend center
        let event = MidiEvent::from_bytes(&bytes).unwrap();
        assert_eq!(event.channel, Channel::Ch1);
        match event.msg {
            ChannelVoiceMsg::PitchBend { bend } => {
                // 14-bit value: LSB=0, MSB=64 → 64 << 7 = 8192
                assert_eq!(bend, 8192);
            }
            _ => panic!("Expected PitchBend"),
        }
    }

    #[test]
    fn test_parse_program_change() {
        let bytes = [0xC0, 5]; // Program change, channel 0, program 5
        let event = MidiEvent::from_bytes(&bytes).unwrap();
        assert_eq!(event.channel, Channel::Ch1);
        match event.msg {
            ChannelVoiceMsg::ProgramChange { program } => {
                assert_eq!(program, 5);
            }
            _ => panic!("Expected ProgramChange"),
        }
    }

    #[test]
    fn test_list_devices() {
        // This test just verifies the function doesn't crash
        // Actual device availability depends on the system
        let devices = MidiInputManager::list_devices();
        // devices might be empty on CI or systems without MIDI
        println!("Found {} MIDI devices", devices.len());
    }

    #[test]
    fn test_manager_creation() {
        let port_manager = Arc::new(MidiPortManager::default());
        let manager = MidiInputManager::new(port_manager);
        assert!(!manager.is_connected());
        assert!(manager.connected_device_name().is_none());
        assert!(manager.connected_port_index().is_none());
        // Give thread time to start
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
