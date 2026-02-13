//! # 13 - Metering
//!
//! Monitor audio levels: peak, RMS, LUFS loudness, stereo correlation.
//!
//! **Concepts:** `metering()`, amplitude, LUFS, stereo analysis, CPU meter
//!
//! ```bash
//! cargo run --example 13_metering
//! ```

use std::time::Duration;
use tutti::prelude::*;
use tutti::TuttiNet;

fn main() -> tutti::Result<()> {
    let engine = TuttiEngine::builder().build()?;

    // Enable meters with fluent API
    engine.metering().amp().lufs().correlation().cpu();

    // Create a test signal: stereo sine with slight detune (creates movement)
    engine.graph_mut(|net: &mut TuttiNet| {
        let left = sine_hz::<f64>(440.0) * 0.5;
        let right = sine_hz::<f64>(442.0) * 0.5; // Slight detune
        net.add(left | right).master();
    });

    engine.transport().play();
    println!("Monitoring levels...");

    for i in 0..10 {
        std::thread::sleep(Duration::from_millis(500));

        let m = engine.metering();

        // Get amplitude (peak and RMS)
        let (l_peak, r_peak, l_rms, r_rms) = m.amplitude();

        // Get stereo analysis
        let stereo = m.stereo_analysis();

        // Get LUFS (may not be ready immediately)
        let lufs = m.loudness_shortterm().unwrap_or(-100.0);

        // Get CPU load
        let cpu = m.cpu_average();

        println!(
            "[{:2}] Peak L/R: {:5.2}/{:5.2} | RMS: {:5.2}/{:5.2} | LUFS: {:6.1} | Corr: {:5.2} | CPU: {:4.1}%",
            i,
            l_peak,
            r_peak,
            l_rms,
            r_rms,
            lufs,
            stereo.correlation,
            cpu
        );
    }

    // Final loudness summary
    let m = engine.metering();
    if let Ok(global) = m.loudness_global() {
        print!("\nIntegrated: {:.1} LUFS", global);
    }
    if let Ok(range) = m.loudness_range() {
        print!(" | Range: {:.1} LU", range);
    }
    println!();

    Ok(())
}
