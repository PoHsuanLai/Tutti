//! Neural synthesis example: GPU-accelerated DDSP synthesis
//!
//! Demonstrates: Neural audio processing, GPU acceleration, DDSP synthesis
//!
//! Run with: cargo run --example neural_synth --features=neural

#[cfg(not(feature = "neural"))]
fn main() {
    eprintln!("This example requires the 'neural' feature.");
    eprintln!("Run with: cargo run --example neural_synth --features=neural");
}

#[cfg(feature = "neural")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use tutti::prelude::*;

    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    println!("Neural synthesis example");
    println!("Demonstrates GPU-accelerated DDSP synthesis");
    println!();

    // Example workflow (neural API in development):
    // let ddsp_model = engine.neural().load_model("path/to/ddsp_model.onnx")?;
    //
    // engine.graph(|net| {
    //     // DDSP takes pitch and loudness as input
    //     let pitch = constant(440.0); // A4
    //     let loudness = constant(-20.0); // dB
    //
    //     let synth = net.add_neural(ddsp_model, vec![pitch, loudness]);
    //     net.pipe_output(synth);
    // });

    println!("Neural synthesis architecture:");
    println!("  1. Control parameters (pitch, loudness) generated on CPU");
    println!("  2. Neural model runs on GPU (batch processing)");
    println!("  3. Results queued to audio thread via lock-free queue");
    println!("  4. Audio thread renders DSP (FunDSP) with neural parameters");
    println!();
    println!("This ensures RT-safe audio processing with GPU acceleration");
    println!();

    // For now, use regular synthesis
    engine.graph(|net| {
        let sine = net.add(Box::new(sine_hz::<f64>(440.0) * 0.5));
        net.pipe_output(sine);
    });

    engine.transport().play();

    println!("Playing sine wave (neural synthesis in development)");
    println!("Press Ctrl+C to exit.");

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
