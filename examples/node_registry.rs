//! Example: Node Registry for Dynamic Node Creation
//!
//! Demonstrates creating audio nodes from string identifiers and parameters.

use std::thread;
use std::time::Duration;
use tutti::prelude::*;

fn main() -> tutti::Result<()> {
    let registry = NodeRegistry::default();
    let engine = TuttiEngine::builder()
        .sample_rate(44100.0)
        .outputs(2)
        .build()?;

    // Build a graph using macros
    engine.graph(|net| {
        // Create nodes using node! macro
        let sine_id = node!(net, registry, "sine", "sine_440", {
            "frequency" => 440.0
        });

        let filter_id = node!(net, registry, "lowpass", "lowpass_filter", {
            "cutoff" => 2000.0
        });

        let gain_id = node!(net, registry, "mul", "gain", {
            "value" => 0.3
        });

        let reverb_id = node!(net, registry, "reverb_stereo", "reverb", {
            "room_size" => 0.7,
            "time" => 3.0,
            "diffusion" => 0.9
        });

        // Connect using chain! macro
        chain!(net, sine_id, filter_id, gain_id, reverb_id => output);
    });

    // Register a custom node type
    registry.register("custom_oscillator", |params| {
        use tutti::dsp::*;
        let freq = get_param_or(params, "frequency", 440.0, |v| v.as_f32());
        let detune = get_param_or(params, "detune", 5.0, |v| v.as_f32());
        Ok(Box::new(
            sine_hz::<f32>(freq) * 0.5 + sine_hz::<f32>(freq + detune) * 0.5,
        ))
    });

    engine.transport().play();
    thread::sleep(Duration::from_secs(5));
    engine.transport().stop();

    Ok(())
}
