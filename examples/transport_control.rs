//! Transport control example demonstrating the fluent API

use std::error::Error;
use std::thread;
use std::time::Duration;
use tutti::prelude::*;

fn main() -> Result<(), Box<dyn Error>> {
    // Create engine
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    // Create a simple sine wave
    engine.add_node("sine", |params| {
        let freq: f32 = get_param_or(params, "frequency", 440.0);
        Ok(Box::new(sine_hz::<f32>(freq)))
    });

    let sine = engine.instance("sine", &params! { "frequency" => 440.0 })?;

    engine.graph(|net| {
        net.pipe_output(sine);
    });

    println!("=== Fluent Transport API Demo ===\n");

    // Configure transport and metronome with fluent API
    println!("Setting tempo to 120 BPM with loop from 0 to 4 beats...");
    engine
        .transport()
        .tempo(120.0)
        .loop_range(0.0, 4.0)
        .enable_loop();

    // Configure metronome
    println!("Configuring metronome (volume 0.5, accent every 4 beats, always on)...");
    engine
        .transport()
        .metronome()
        .volume(0.5)
        .accent_every(4)
        .always();

    // Start playback
    println!("Starting playback...");
    engine.transport().play();

    // Monitor state
    for i in 0..10 {
        thread::sleep(Duration::from_millis(500));

        let transport = engine.transport();
        let beat = transport.current_beat();
        let tempo = transport.get_tempo();
        let is_looping = transport.is_loop_enabled();

        println!(
            "[{}] Beat: {:.2} | Tempo: {} BPM | Playing: {} | Loop: {}",
            i,
            beat,
            tempo,
            transport.is_playing(),
            is_looping
        );
    }

    // Seek to beat 2
    println!("\nSeeking to beat 2...");
    engine.transport().locate(2.0);
    thread::sleep(Duration::from_secs(2));

    // Change tempo
    println!("Changing tempo to 140 BPM...");
    engine.transport().tempo(140.0);
    thread::sleep(Duration::from_secs(2));

    // Stop with declick
    println!("Stopping with declick...");
    engine.transport().stop();
    thread::sleep(Duration::from_millis(500));

    println!("\n=== Demo Complete ===");

    Ok(())
}
