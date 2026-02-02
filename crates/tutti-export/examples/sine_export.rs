//! Export a simple sine wave to WAV

use tutti_export::{export_wav, ExportOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sample_rate = 44100;
    let duration_seconds = 2.0;
    let num_samples = (sample_rate as f64 * duration_seconds) as usize;

    // Generate 440Hz sine wave (stereo)
    let mut left = Vec::with_capacity(num_samples);
    let mut right = Vec::with_capacity(num_samples);

    for i in 0..num_samples {
        let t = i as f64 / sample_rate as f64;
        let sample = (t * 440.0 * 2.0 * std::f64::consts::PI).sin() as f32;
        
        // Simple fade out
        let env = if t > duration_seconds - 0.1 {
            (duration_seconds - t) / 0.1
        } else {
            1.0
        } as f32;

        left.push(sample * 0.5 * env);
        right.push(sample * 0.5 * env);
    }

    // Export
    let options = ExportOptions::default();
    let filename = "sine.wav";
    export_wav(filename, &left, &right, &options)?;

    println!("Exported {} ({:.2}s)", filename, duration_seconds);
    Ok(())
}
