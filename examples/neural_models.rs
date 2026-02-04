//! Neural Models Example
//!
//! Shows how to register and use neural audio models.
//!
//! ## Setup
//!
//! Create a test model:
//! ```bash
//! cd assets/models
//! python create_test_model.py
//! ```
//!
//! Requirements: `pip install torch onnxscript`

use std::time::Duration;
use tutti::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Try to load the test model
    let model_path = "assets/models/simple_synth.mpk";

    if !std::path::Path::new(model_path).exists() {
        println!("Model not found at: {}", model_path);
        println!();
        println!("To create the model:");
        println!("  cd assets/models");
        println!("  python create_test_model.py");
        println!("  onnx2burn simple_synth.onnx simple_synth.mpk");
        return Ok(());
    }

    // Create engine (neural enabled via cargo feature)
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    // Load neural synth model
    engine.load_synth_mpk("simple_synth", model_path)?;

    println!("Loaded neural model: simple_synth");

    // Instantiate and add to graph
    let synth = engine.instance("simple_synth", &params! {})?;

    engine.graph(|net| {
        net.pipe_output(synth);
    });

    println!("Playing neural synth for 3 seconds...");
    engine.transport().play();
    std::thread::sleep(Duration::from_secs(3));

    Ok(())
}
