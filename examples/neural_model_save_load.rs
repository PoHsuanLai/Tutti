//! Neural Model Workflow Example
//!
//! Demonstrates the workflow for using neural models with Tutti.
//! Shows how models are registered and used in the audio graph.

use tutti::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Neural Model Workflow ===\n");

    // Create neural system
    let _neural = NeuralSystem::builder().sample_rate(44100.0).build()?;

    println!("Neural system created successfully!\n");

    // Show the workflow for using custom models
    println!("To use custom neural models:");
    println!();
    println!("1. Train your model in Python/PyTorch");
    println!("   Example: DDSP synthesizer, neural vocoder, etc.");
    println!();
    println!("2. Convert to Burn .mpk format:");
    println!("   $ burn-import onnx model.onnx --out-type burn model.mpk");
    println!();
    println!("3. Register with Tutti:");
    println!("   let registry = NodeRegistry::default();");
    println!("   register_neural_model(&registry, &neural, \"my_synth\", \"model.mpk\")?;");
    println!();
    println!("4. Use in audio graph:");
    println!("   engine.graph(|net| {{");
    println!("       let synth = registry.create(\"my_synth\", &params! {{}}).unwrap();");
    println!("       let synth_id = net.add(synth);");
    println!("       net.pipe_output(synth_id);");
    println!("   }});");
    println!();
    println!("Alternative: Load SafeTensors format");
    println!("   $ Enable feature: tutti = {{ features = [\"neural\", \"safetensors\"] }}");
    println!(
        "   $ register_neural_model(&registry, &neural, \"my_synth\", \"model.safetensors\")?;"
    );
    println!();
    println!("✓ Neural system is ready for model loading!");

    Ok(())
}
