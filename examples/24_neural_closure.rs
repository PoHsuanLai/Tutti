//! # 18 - Neural Closure
//!
//! Register neural synth and effect from plain closures (no model files).
//!
//! **Concepts:** `neural_synth_fn`, `neural_effect_fn`, MIDI-driven inference
//!
//! ```bash
//! cargo run --example 18_neural_closure --features neural,burn,midi
//! ```

use std::time::Duration;
use tutti::prelude::*;

fn main() -> tutti::Result<()> {
    let buffer_size = 512;

    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    // Neural synth: MIDI features → control parameters
    // Returns (Box<dyn AudioUnit>, NeuralModelId)
    let buf = buffer_size;
    let (synth_unit, _synth_model_id) = engine
        .neural_synth_fn(move |features: &[f32]| {
            let f0 = if !features.is_empty() {
                features[0]
            } else {
                440.0
            };
            let amp = if features.len() > 1 { features[1] } else { 0.5 };

            let mut out = Vec::with_capacity(buf * 2);
            out.extend(std::iter::repeat(f0).take(buf));
            out.extend(std::iter::repeat(amp).take(buf));
            out
        })
        .build()?;

    // Neural effect: soft-clip distortion
    let (effect_unit, _effect_model_id) = engine
        .neural_effect_fn(move |audio: &[f32]| audio.iter().map(|&x| x.tanh()).collect())
        .build()?;

    // Add to graph - neural nodes return Box<dyn AudioUnit>, use add_boxed()
    let synth = engine.graph(|net| net.add_boxed(synth_unit).id());
    engine.graph(|net| {
        let fx_id = net.add_boxed(effect_unit).id();
        net.pipe(synth, fx_id);
        net.pipe_output(fx_id);
    });

    engine.transport().play();
    println!("Playing neural synth arpeggio...");

    let notes = [60u8, 64, 67, 72];
    for &note in &notes {
        engine.note_on(synth, note, 100);
        std::thread::sleep(Duration::from_millis(500));
        engine.note_off(synth, note);
        std::thread::sleep(Duration::from_millis(100));
    }

    std::thread::sleep(Duration::from_millis(500));

    Ok(())
}
