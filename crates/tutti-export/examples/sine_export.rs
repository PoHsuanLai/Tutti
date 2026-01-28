//! Export a simple sine wave to WAV

use fundsp::prelude::*;
use tutti_export::{
    export_wav, ExportOptions, OfflineRenderer, RenderJob, RenderNote, RenderTrack,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sample_rate = 44100;
    let mut renderer = OfflineRenderer::new(sample_rate);

    // Register a simple sine wave synth (stereo)
    let synth_idx = renderer.register_synth(Box::new(|note, _vel, _params| {
        let freq = 440.0 * 2.0f32.powf((note as f32 - 69.0) / 12.0);
        Box::new(sine_hz::<f32>(freq) | sine_hz::<f32>(freq))
    }));

    // Create a 1-second render job
    let job = RenderJob::new(sample_rate, sample_rate as usize).with_track(
        RenderTrack::new(0).with_note(RenderNote {
            synth_index: synth_idx,
            midi_note: 60, // C4
            velocity: 100,
            start_sample: 0,
            duration_samples: sample_rate as usize,
            params: None,
        }),
    );

    // Render
    let result = renderer.render(job, None)?;

    // Export
    let options = ExportOptions::default();
    export_wav("sine.wav", &result.left, &result.right, &options)?;

    println!("Exported sine.wav ({:.2}s)", result.duration_seconds());
    Ok(())
}
