//! # 14 - Streaming
//!
//! Stream large audio files from disk using the Butler thread.
//!
//! **Concepts:** `sampler()`, `stream()`, Butler thread, disk I/O
//!
//! ```bash
//! cargo run --example 14_streaming --features sampler
//! ```

use std::time::Duration;
use tutti::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let audio_path =
        std::env::var("AUDIO_FILE").unwrap_or_else(|_| "assets/audio/test.wav".to_string());

    if !std::path::Path::new(&audio_path).exists() {
        println!("Audio file not found: {}", audio_path);
        println!("Set AUDIO_FILE=/path/to/audio.wav");
        return run_synth_demo();
    }

    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    let sampler = engine.sampler();
    if !sampler.is_enabled() {
        println!("Sampler not enabled, using synth fallback...");
        return run_synth_demo();
    }

    sampler.run();
    sampler.stream(&audio_path).start();

    engine.transport().play();
    println!("Streaming: {}", audio_path);

    std::thread::sleep(Duration::from_secs(5));
    sampler.shutdown();

    Ok(())
}

fn run_synth_demo() -> Result<(), Box<dyn std::error::Error>> {
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    engine.graph(|net| {
        let pad = sine_hz::<f64>(220.0) * 0.2 + sine_hz::<f64>(330.0) * 0.15;
        net.add(pad >> split::<U2>()).to_master();
    });

    engine.transport().play();
    println!("Playing synth fallback...");
    std::thread::sleep(Duration::from_secs(3));

    Ok(())
}
