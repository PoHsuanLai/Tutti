//! # 21 - Load Audio
//!
//! Load and play audio files (WAV, FLAC, MP3, OGG).
//!
//! **Concepts:** `wav()`, `flac()`, `mp3()`, `ogg()`, sample playback
//!
//! ```bash
//! cargo run --example 21_load_audio --features "sampler,wav"
//! ```

use std::time::Duration;
use tutti::prelude::*;

fn main() -> tutti::Result<()> {
    let audio_path =
        std::env::var("AUDIO_FILE").unwrap_or_else(|_| "assets/audio/test.wav".to_string());

    if !std::path::Path::new(&audio_path).exists() {
        println!("Audio file not found: {}", audio_path);
        println!("Set AUDIO_FILE=/path/to/your.wav or place a file at assets/audio/test.wav");
        println!("\nRunning with generated tone instead...");
        return run_tone_demo();
    }

    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    // New fluent API: engine.wav(path).build() returns SamplerUnit
    let sampler = engine.wav(&audio_path).build()?;

    // Add to graph
    engine.graph(|net| {
        net.add(sampler).master();
    });

    engine.transport().play();
    println!("Playing: {}", audio_path);

    std::thread::sleep(Duration::from_secs(5));

    Ok(())
}

fn run_tone_demo() -> tutti::Result<()> {
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    engine.graph(|net| {
        net.add(sine_hz::<f32>(440.0) * 0.3).master();
    });

    engine.transport().play();
    println!("Playing 440Hz tone...");

    std::thread::sleep(Duration::from_secs(2));

    Ok(())
}
