//! Example: Node Introspection and Tagging
//!
//! Demonstrates the new node introspection and tagging APIs.

use std::thread;
use std::time::Duration;
use tutti::prelude::*;

fn main() -> tutti::Result<()> {
    println!("=== Node Introspection Example ===\n");

    // Create engine
    let engine = TuttiEngine::builder()
        .sample_rate(44100.0)
        .outputs(2)
        .build()?;

    // Build a simple graph with tags
    engine.graph(|net| {
        use fundsp::prelude::*;

        // Add nodes with descriptive tags
        let osc1 = net.add_tagged(Box::new(sine_hz::<f32>(220.0) * 0.3), "bass_oscillator");

        let osc2 = net.add_tagged(Box::new(sine_hz::<f32>(440.0) * 0.2), "lead_oscillator");

        // Mix the oscillators using mix! macro
        let mixed = mix!(net, osc1, osc2);

        // Add reverb
        let reverb = net.add_tagged(Box::new(reverb_stereo(0.5, 5.0, 1.0)), "master_reverb");

        // Chain using macro
        chain!(net, mixed, reverb => output);

        println!("Built audio graph with tagged nodes\n");
    });

    // Query node information
    println!("=== Node Information ===\n");
    engine.graph(|net| {
        // List all nodes
        let nodes = net.nodes();
        println!("Found {} tagged nodes:", nodes.len());

        for node_info in &nodes {
            println!(
                "  - {} ({})",
                node_info.name,
                node_info.tag.as_deref().unwrap_or("no tag")
            );
            println!("    ID: {:?}", node_info.id);
            println!(
                "    Inputs: {}, Outputs: {}",
                node_info.inputs, node_info.outputs
            );
            println!("    Type: {}", node_info.type_name);
            println!();
        }
    });

    // Find specific nodes by tag
    println!("=== Finding Nodes by Tag ===\n");
    engine.graph(|net| {
        if let Some(bass_id) = net.find_by_tag("bass_oscillator") {
            println!("Found bass oscillator: {:?}", bass_id);

            if let Some(info) = net.node_info(bass_id) {
                println!("  Name: {}", info.name);
                println!("  Inputs: {}, Outputs: {}", info.inputs, info.outputs);
            }
        }

        if let Some(reverb_id) = net.find_by_tag("master_reverb") {
            println!("\nFound master reverb: {:?}", reverb_id);

            if let Some(info) = net.node_info(reverb_id) {
                println!("  Name: {}", info.name);
                println!("  Type: {}", info.type_name);
            }
        }
    });

    // Start playback
    println!("\n=== Starting Playback ===\n");
    engine.transport().play();

    println!("Playing for 3 seconds...");
    thread::sleep(Duration::from_secs(3));

    println!("\nStopping playback");
    engine.transport().stop();

    println!("\nExample complete!");

    Ok(())
}
