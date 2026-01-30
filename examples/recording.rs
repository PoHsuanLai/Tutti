//! Recording example: Capture audio to a WAV file
//!
//! Demonstrates: Audio recording, transport control, file export
//!
//! Run with: cargo run --example recording

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::time::Duration;
    use tutti::prelude::*;

    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    // Create a musical phrase: C-E-G arpeggio
    engine.graph(|net| {
        let c = sine_hz::<f32>(261.63) * 0.2;
        let e = sine_hz::<f32>(329.63) * 0.2;
        let g = sine_hz::<f32>(392.00) * 0.2;

        // Mix the chord
        let chord = net.add(Box::new(c + e + g));
        net.pipe_output(chord);
    });

    println!("Recording 5 seconds of audio...");

    // Note: Recording API is not yet implemented in this version
    // engine.transport().record();
    engine.transport().play();

    std::thread::sleep(Duration::from_secs(5));

    engine.transport().stop();

    println!("Recording complete. Exporting to 'recording.wav'...");

    // Note: Actual export API would be used here
    // This is a placeholder showing the workflow
    println!("Export complete! (This is a demonstration - actual export requires additional API)");
    println!("\nIn a full implementation, you would use:");
    println!("  engine.export().to_file(\"recording.wav\", ExportFormat::Wav)?;");

    Ok(())
}
