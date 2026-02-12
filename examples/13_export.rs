//! # 13 - Export
//!
//! Render audio to file: WAV, FLAC, MP3 with normalization options.
//!
//! **Concepts:** `export()`, `AudioFormat`, `NormalizationMode`, progress callback
//!
//! ```bash
//! cargo run --example 13_export --features export
//! ```

use tutti::export::ExportPhase;
use tutti::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    engine.graph(|net| {
        let c = sine_hz::<f64>(261.63) * 0.2;
        let e = sine_hz::<f64>(329.63) * 0.2;
        let g = sine_hz::<f64>(392.00) * 0.2;
        net.add((c + e + g) >> split::<U2>()).master();
    });

    // Export to WAV
    engine
        .export()
        .duration_seconds(3.0)
        .to_file("/tmp/tutti_export_demo.wav")?;
    println!("Exported: /tmp/tutti_export_demo.wav");

    // Export to FLAC with normalization
    engine
        .export()
        .duration_seconds(3.0)
        .format(AudioFormat::Flac)
        .normalize(NormalizationMode::lufs(-14.0))
        .to_file("/tmp/tutti_export_demo.flac")?;
    println!("Exported: /tmp/tutti_export_demo.flac (-14 LUFS)");

    // Export with progress
    engine
        .export()
        .duration_seconds(5.0)
        .compensate_latency(true)
        .to_file_with_progress("/tmp/tutti_export_progress.wav", |progress| {
            let phase = match progress.phase {
                ExportPhase::Rendering => "Rendering",
                ExportPhase::Processing => "Processing",
                ExportPhase::Encoding => "Encoding",
            };
            print!("\r{} {:.0}%", phase, progress.progress * 100.0);
        })?;
    println!("\nExported: /tmp/tutti_export_progress.wav");

    // Render to memory
    let (left, _right, sample_rate) = engine.export().duration_seconds(1.0).render()?;
    println!("Rendered: {} samples @ {} Hz", left.len(), sample_rate);

    Ok(())
}
