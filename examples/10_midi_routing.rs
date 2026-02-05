//! MIDI Routing Example
//!
//! Run with: cargo run --example midi_routing --features midi

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tutti::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = TuttiEngine::builder().midi().build()?;

    // Create gates to control each synth
    let lead_on = Arc::new(AtomicBool::new(false));
    let bass_on = Arc::new(AtomicBool::new(false));
    let pad_on = Arc::new(AtomicBool::new(false));

    let lead_gate = lead_on.clone();
    let bass_gate = bass_on.clone();
    let pad_gate = pad_on.clone();

    engine.graph(move |net| {
        // Gated synths - only produce sound when gate is true
        let lead = An(GatedSine::new(880.0, lead_gate.clone())) >> split::<U2>() * 0.3;
        let bass = An(GatedSine::new(110.0, bass_gate.clone())) >> split::<U2>() * 0.4;
        let pad = An(GatedSine::new(330.0, pad_gate.clone())) >> split::<U2>() * 0.3;

        let lead_node = net.add(Box::new(lead));
        let bass_node = net.add(Box::new(bass));
        let pad_node = net.add(Box::new(pad));

        net.pipe_output(lead_node);
        net.pipe_output(bass_node);
        net.pipe_output(pad_node);
    });

    engine.transport().play();

    // Play sequence: lead -> bass -> pad -> chord
    lead_on.store(true, Ordering::Relaxed);
    thread::sleep(Duration::from_millis(500));
    lead_on.store(false, Ordering::Relaxed);
    thread::sleep(Duration::from_millis(100));

    bass_on.store(true, Ordering::Relaxed);
    thread::sleep(Duration::from_millis(500));
    bass_on.store(false, Ordering::Relaxed);
    thread::sleep(Duration::from_millis(100));

    pad_on.store(true, Ordering::Relaxed);
    thread::sleep(Duration::from_millis(500));
    pad_on.store(false, Ordering::Relaxed);
    thread::sleep(Duration::from_millis(100));

    // All together
    lead_on.store(true, Ordering::Relaxed);
    bass_on.store(true, Ordering::Relaxed);
    pad_on.store(true, Ordering::Relaxed);
    thread::sleep(Duration::from_millis(1000));

    Ok(())
}

// Simple gated sine oscillator
#[derive(Clone)]
struct GatedSine {
    phase: f32,
    freq: f32,
    gate: Arc<AtomicBool>,
}

impl GatedSine {
    fn new(freq: f32, gate: Arc<AtomicBool>) -> Self {
        Self {
            phase: 0.0,
            freq,
            gate,
        }
    }
}

impl AudioNode for GatedSine {
    const ID: u64 = 0x12345678;
    type Inputs = U0;
    type Outputs = U1;

    fn tick(&mut self, _input: &Frame<f32, Self::Inputs>) -> Frame<f32, Self::Outputs> {
        if !self.gate.load(Ordering::Relaxed) {
            return [0.0].into();
        }
        let sample = (self.phase * std::f32::consts::TAU).sin();
        self.phase += self.freq / 44100.0;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        [sample].into()
    }

    fn reset(&mut self) {
        self.phase = 0.0;
    }
}
