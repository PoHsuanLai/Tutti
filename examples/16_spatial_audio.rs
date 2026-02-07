//! # 16 - Spatial Audio
//!
//! 3D audio positioning with binaural (headphones) and VBAP (surround) panning.
//!
//! **Concepts:** `BinauralPannerNode`, `SpatialPannerNode`, azimuth, elevation
//!
//! ```bash
//! cargo run --example 16_spatial_audio
//! ```

use std::time::Duration;
use tutti::prelude::*;
use tutti::BinauralPannerNode;

fn main() -> tutti::Result<()> {
    let engine = TuttiEngine::builder()
        .sample_rate(44100.0)
        .outputs(2)
        .build()?;

    let panner = BinauralPannerNode::new(44100.0);

    // Create audio source directly in graph
    let source = engine.graph(|net| net.add(sine_hz::<f32>(880.0) * 0.4).id());

    engine.graph(|net| {
        let panner_id = net.add(panner.clone()).id();
        net.pipe(source, panner_id);
        net.pipe_output(panner_id);
    });

    engine.transport().play();
    println!("Binaural panning (use headphones)...");

    // Rotate around listener
    for i in 0..16 {
        let azimuth = (i as f32) * 22.5;
        panner.set_position(azimuth, 0.0);
        std::thread::sleep(Duration::from_millis(500));
    }

    // Elevation demo
    panner.set_position(0.0, 0.0);
    for elevation in [-45.0_f32, 0.0, 45.0, 0.0, -45.0] {
        panner.set_position(0.0, elevation);
        std::thread::sleep(Duration::from_millis(800));
    }

    Ok(())
}
