//! MIDI utility functions

#[inline]
pub fn note_to_hz(note: f32) -> f32 {
    440.0 * 2.0_f32.powf((note - 69.0) / 12.0)
}

#[inline]
pub fn hz_to_note(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

#[inline]
pub fn velocity_to_gain(velocity: u8) -> f32 {
    velocity as f32 / 127.0
}

#[inline]
pub fn gain_to_velocity(gain: f32) -> u8 {
    (gain.clamp(0.0, 1.0) * 127.0).round() as u8
}
