//! # 07 - Node Registry
//!
//! Create DSP nodes directly in the audio graph.
//!
//! **Concepts:** `graph()`, `chain!` macro, direct node creation
//!
//! ```bash
//! cargo run --example 07_node_registry
//! ```

use std::time::Duration;
use tutti::prelude::*;
use tutti::TuttiNet;

fn main() -> tutti::Result<()> {
    let engine = TuttiEngine::builder().outputs(2).build()?;

    // Create nodes directly in the graph
    let sine_id = engine.graph_mut(|net: &mut TuttiNet| net.add(sine_hz::<f32>(440.0)).id());
    let filter_id =
        engine.graph_mut(|net: &mut TuttiNet| net.add(lowpass_hz::<f32>(2000.0, 1.0)).id());
    let reverb_id =
        engine.graph_mut(|net: &mut TuttiNet| net.add(reverb_stereo(0.7, 2.0, 0.5)).id());

    engine.graph_mut(|net: &mut TuttiNet| {
        chain!(net, sine_id, filter_id, reverb_id => output);
    });

    engine.transport().play();
    println!("Playing: sine → lowpass → reverb...");
    std::thread::sleep(Duration::from_secs(5));

    Ok(())
}
