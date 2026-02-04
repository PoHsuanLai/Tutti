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
    println!("3. Load with Tutti engine:");
    println!("   engine.load_synth_mpk(\"my_synth\", \"model.mpk\")?;");
    println!();
    println!("4. Use in audio graph:");
    println!("   let synth = engine.instance(\"my_synth\", &params! {{}})?;");
    println!("   engine.graph(|net| {{");
    println!("       net.pipe_output(synth);");
    println!("   }});");
    println!();
    println!("Alternative: Load SafeTensors format");
    println!("   $ Enable feature: tutti = {{ features = [\"neural\", \"safetensors\"] }}");
    println!("   $ engine.load_safetensors(\"my_synth\", \"model.safetensors\")?;");
    println!();
    println!("✓ Neural system is ready for model loading!");

    Ok(())
}
