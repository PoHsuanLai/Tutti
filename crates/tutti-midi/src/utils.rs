//! MIDI utility functions

use libm::{log2f, powf, roundf};

#[inline]
pub fn note_to_hz(note: f32) -> f32 {
    440.0 * powf(2.0, (note - 69.0) / 12.0)
}

#[inline]
pub fn hz_to_note(hz: f32) -> f32 {
    69.0 + 12.0 * log2f(hz / 440.0)
}

#[inline]
pub fn velocity_to_gain(velocity: u8) -> f32 {
    velocity as f32 / 127.0
}

#[inline]
pub fn gain_to_velocity(gain: f32) -> u8 {
    roundf(gain.clamp(0.0, 1.0) * 127.0) as u8
}
