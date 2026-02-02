//! Example: Node Registry for Dynamic Node Creation
//!
//! Demonstrates creating audio nodes from string identifiers and parameters
//! using the engine's built-in registry.

use std::thread;
use std::time::Duration;
use tutti::prelude::*;

fn main() -> tutti::Result<()> {
    let engine = TuttiEngine::builder()
        .sample_rate(44100.0)
        .outputs(2)
        .build()?;

    // Register custom DSP nodes
    engine.add_node("sine", |params| {
        use tutti::dsp::*;
        let freq = get_param_or(params, "frequency", 440.0, |v| v.as_f32());
        Ok(Box::new(sine_hz::<f32>(freq)))
    });

    engine.add_node("lowpass", |params| {
        use tutti::dsp::*;
        let cutoff = get_param_or(params, "cutoff", 2000.0, |v| v.as_f32());
        let q = get_param_or(params, "q", 1.0, |v| v.as_f32());
        Ok(Box::new(lowpass_hz::<f32>(cutoff, q)))
    });

    engine.add_node("mul", |params| {
        use tutti::dsp::*;
        let value = get_param_or(params, "value", 1.0, |v| v.as_f32());
        Ok(Box::new(dc(value)))
    });

    engine.add_node("reverb_stereo", |params| {
        use tutti::dsp::*;
        let room_size = get_param_or(params, "room_size", 0.5, |v| v.as_f64());
        let time = get_param_or(params, "time", 2.0, |v| v.as_f64());
        let diffusion = get_param_or(params, "diffusion", 0.5, |v| v.as_f64());
        Ok(Box::new(reverb_stereo(room_size, time, diffusion)))
    });

    engine.add_node("custom_oscillator", |params| {
        use tutti::dsp::*;
        let freq = get_param_or(params, "frequency", 440.0, |v| v.as_f32());
        let detune = get_param_or(params, "detune", 5.0, |v| v.as_f32());
        Ok(Box::new(
            sine_hz::<f32>(freq) * 0.5 + sine_hz::<f32>(freq + detune) * 0.5,
        ))
    });

    // Instantiate nodes (creates instances and returns NodeIds)
    let sine_id = engine.instance("sine", &params! { "frequency" => 440.0 })?;
    let filter_id = engine.instance("lowpass", &params! { "cutoff" => 2000.0 })?;
    let gain_id = engine.instance("mul", &params! { "value" => 0.3 })?;
    let reverb_id = engine.instance(
        "reverb_stereo",
        &params! {
            "room_size" => 0.7,
            "time" => 3.0
        },
    )?;

    // Build audio graph
    engine.graph(|net| {
        chain!(net, sine_id, filter_id, gain_id, reverb_id => output);
    });

    engine.transport().play();
    thread::sleep(Duration::from_secs(5));
    engine.transport().stop();

    Ok(())
}
