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
    let neural = NeuralSystem::builder().sample_rate(44100.0).build()?;
    let registry = NodeRegistry::default();

    // Try to register the test model
    let model_path = std::path::PathBuf::from("assets/models/simple_synth.mpk");

    if !model_path.exists() {
        println!("Model not found at: {}", model_path.display());
        println!();
        println!("To create the model:");
        println!("  cd assets/models");
        println!("  python create_test_model.py");
        println!("  onnx2burn simple_synth.onnx simple_synth.mpk");
        return Ok(());
    }

    // Register the model
    register_neural_model(&registry, &neural, "simple_synth", &model_path)?;

    println!(
        "Registered {} nodes (builtin + neural)",
        registry.list_types().len()
    );
    println!("Neural model: simple_synth");

    // Create engine with neural synth
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    engine.graph(|net| {
        // Use the neural synth instead of basic sine
        let synth = registry.create("simple_synth", &params! {}).unwrap();
        let synth_id = net.add(synth);
        net.pipe_output(synth_id);
    });

    println!("Playing neural synth for 3 seconds...");
    engine.transport().play();
    std::thread::sleep(Duration::from_secs(3));

    Ok(())
}
