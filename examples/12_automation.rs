//! # 12 - Automation
//!
//! Automate a synth pad's volume over time using an automation envelope.
//! The automation lane reads the transport beat position and outputs a control
//! signal that modulates the pad amplitude.
//!
//! **Concepts:** AutomationEnvelope, AutomationPoint, CurveType, automation_lane, graph routing
//!
//! ```bash
//! cargo run --example 12_automation --features automation
//! ```

use std::time::Duration;
use tutti::prelude::*;
use tutti::TuttiNet;

fn main() -> tutti::Result<()> {
    let engine = TuttiEngine::builder().build()?;

    // Automation envelope: fade in → hold → fade out
    //
    //   1.0 |        ___________
    //       |      /             \
    //   0.5 |    /                 \
    //       |  /                     \
    //   0.0 |/________________________\___
    //       0    4    8         16   20  24
    //                  beats
    let mut envelope = AutomationEnvelope::new("volume");
    envelope
        .add_point(AutomationPoint::new(0.0, 0.0))
        .add_point(AutomationPoint::with_curve(4.0, 1.0, CurveType::SCurve))
        .add_point(AutomationPoint::new(8.0, 1.0))
        .add_point(AutomationPoint::new(16.0, 1.0))
        .add_point(AutomationPoint::with_curve(20.0, 0.0, CurveType::SCurve));

    println!("Envelope shape:");
    for beat in [0.0, 2.0, 4.0, 8.0, 12.0, 16.0, 18.0, 20.0] {
        let lane_preview = engine.automation_lane(envelope.clone());
        println!(
            "  Beat {:5.1}: {:.2}",
            beat,
            lane_preview.get_value_at(beat)
        );
    }

    let lane = engine.automation_lane(envelope);

    // Build the graph:
    //   pad (saw chord) ──┐
    //                     ├─ multiply ─→ master output
    //   automation lane ──┘
    //
    // The multiplier (pass() * pass()) takes 2 inputs and outputs their product.
    let pad = (saw_hz(220.0) + saw_hz(330.0) + saw_hz(440.0)) * 0.15;
    let mult = pass() * pass();

    engine.graph_mut(|net: &mut TuttiNet| {
        let pad_id = net.add(pad).id();
        let lane_id = net.add(lane).id();
        let mult_id = net.add(mult).id();

        // pad → mult input 0 (audio signal)
        net.connect_ports(pad_id, 0, mult_id, 0);
        // automation → mult input 1 (volume envelope)
        net.connect_ports(lane_id, 0, mult_id, 1);
        // mult → master output
        net.pipe_output(mult_id);
    });

    engine.transport().tempo(120.0).play();
    println!("\nPlaying: saw chord with automated volume (120 BPM, 20 beats = 10s)");
    println!("  0-4 beats:   fade in (S-curve)");
    println!("  4-16 beats:  full volume");
    println!("  16-20 beats: fade out (S-curve)");

    std::thread::sleep(Duration::from_secs(10));
    println!("Done.");

    Ok(())
}
