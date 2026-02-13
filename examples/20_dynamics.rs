//! # 20 - Dynamics Processing
//!
//! Sidechain compression and gating with builder pattern and runtime control.
//!
//! **Concepts:** SidechainCompressor, SidechainGate, builder, atomic runtime control
//!
//! ```bash
//! cargo run --example 20_dynamics --features dsp
//! ```

use std::time::Duration;
use tutti::dsp_nodes::{SidechainCompressor, SidechainGate};
use tutti::prelude::*;
use tutti::TuttiNet;

fn main() -> tutti::Result<()> {
    let engine = TuttiEngine::builder().build()?;

    // --- Sidechain Compressor ---
    // Builder pattern with fluent API
    let comp = SidechainCompressor::builder()
        .threshold_db(-20.0)
        .ratio(4.0)
        .attack_seconds(0.001)
        .release_seconds(0.05)
        .soft_knee_db(6.0)
        .makeup_gain_db(3.0)
        .build();

    // Atomic runtime control via shared handles
    let threshold = comp.threshold();
    let ratio = comp.ratio();

    println!("Sidechain Compressor:");
    println!("  Threshold: {:.1} dB", threshold.get());
    println!("  Ratio:     {:.1}:1", ratio.get());

    // --- Sidechain Gate ---
    let gate = SidechainGate::builder()
        .threshold_db(-40.0)
        .attack_seconds(0.001)
        .hold_seconds(0.01)
        .release_seconds(0.1)
        .build();

    println!("\nSidechain Gate:");
    println!("  Threshold: {:.1} dB", gate.threshold().get());

    // --- Audio graph ---
    // Source: sustained pad (input 0 of compressor)
    // Sidechain: kick-like pulse (input 1 of compressor)
    let pad = sine_hz::<f32>(220.0) * 0.6;
    let kick = sine_hz::<f32>(60.0) * 0.8;

    engine.graph_mut(|net: &mut TuttiNet| {
        let pad_id = net.add(pad).id();
        let kick_id = net.add(kick).id();
        let comp_id = net.add(comp).id();

        // Wire: pad → comp input 0, kick → comp input 1 (sidechain)
        net.connect_ports(pad_id, 0, comp_id, 0);
        net.connect_ports(kick_id, 0, comp_id, 1);
        net.pipe_output(comp_id);
    });

    engine.transport().play();
    println!("\nPlaying: pad through sidechain compressor (kicked by 60 Hz)");

    // Adjust parameters at runtime via atomics
    std::thread::sleep(Duration::from_secs(2));
    println!("→ Lowering threshold to -30 dB, increasing ratio to 8:1");
    threshold.set(-30.0);
    ratio.set(8.0);

    std::thread::sleep(Duration::from_secs(2));
    println!("→ Heavy compression: threshold -40 dB, ratio 20:1");
    threshold.set(-40.0);
    ratio.set(20.0);

    std::thread::sleep(Duration::from_secs(2));
    println!("Done.");

    Ok(())
}
