//! # 25 - Neural Models
//!
//! Load and play pre-trained neural audio models (.mpk format).
//!
//! **Concepts:** `neural_synth`, Burn model format, GPU inference
//!
//! ```bash
//! cargo run --example 25_neural_models --features neural,burn,midi
//! ```
//!
//! ## Setup
//!
//! Create a test model:
//! ```bash
//! cd assets/models
//! python create_test_model.py
//! onnx2burn simple_synth.onnx simple_synth.mpk
//! ```

use std::time::Duration;
use tutti::prelude::*;
use tutti::TuttiNet;

fn main() -> tutti::Result<()> {
    let model_path = "assets/models/simple_synth.mpk";

    if !std::path::Path::new(model_path).exists() {
        println!("Model not found: {}", model_path);
        println!("Create: cd assets/models && python create_test_model.py && onnx2burn simple_synth.onnx simple_synth.mpk");
        return Ok(());
    }

    let engine = TuttiEngine::builder().build()?;

    // New fluent API: engine.neural_synth(path).build() returns (Box<dyn AudioUnit>, NeuralModelId)
    let (synth_unit, _model_id) = engine.neural_synth(model_path).build()?;

    engine.graph_mut(|net: &mut TuttiNet| {
        net.add_boxed(synth_unit).master();
    });

    engine.transport().play();
    println!("Playing neural synth...");
    std::thread::sleep(Duration::from_secs(3));

    Ok(())
}
