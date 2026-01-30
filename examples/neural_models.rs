//! Neural Models Example
//!
//! Shows how to register and use neural audio models.

use tutti::prelude::*;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let neural = NeuralSystem::builder().sample_rate(44100.0).build()?;
    let registry = NodeRegistry::default();

    // Register models (uncomment when you have model files):
    // register_neural_model(&registry, &neural, "violin", "/path/to/violin.onnx")?;
    // register_neural_directory(&registry, &neural, "/path/to/models")?;
    // register_neural_synth_models(&registry, &neural, "/path/to/synth")?;
    // register_neural_effects(&registry, &neural, "/path/to/effects")?;

    println!("Registered {} nodes (builtin + models)", registry.list_types().len());

    // Create engine and play sine wave
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    engine.graph(|net| {
        let sine = registry.create("sine", &params! { "frequency" => 440.0 }).unwrap();
        let sine_id = net.add(sine);
        net.pipe_output(sine_id);
    });

    println!("Playing for 3 seconds...");
    engine.transport().play();
    std::thread::sleep(Duration::from_secs(3));

    Ok(())
}
