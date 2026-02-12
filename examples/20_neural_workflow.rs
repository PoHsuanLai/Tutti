//! # 20 - Neural Workflow
//!
//! Complete workflow for neural model creation, conversion, and usage.
//!
//! **Concepts:** Model pipeline, Burn backend, SafeTensors format
//!
//! ```bash
//! cargo run --example 20_neural_workflow --features neural,burn
//! ```

use tutti::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _neural = NeuralSystem::builder()
        .sample_rate(44100.0)
        .backend(tutti::tutti_burn::burn_backend_factory())
        .build()?;

    println!("Neural workflow:");
    println!("  1. Train model (PyTorch)");
    println!("  2. Export: torch.onnx.export(model, input, \"model.onnx\")");
    println!("  3. Convert: burn-import onnx model.onnx --out-type burn model.mpk");
    println!("  4. Load: let (synth, _id) = engine.neural_synth(\"model.mpk\").build()?;");
    println!("  5. Use: engine.graph(|net| net.add_boxed(synth).master());");

    Ok(())
}
