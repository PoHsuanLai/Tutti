//! Plugin Loading Example
//!
//! Loads VST3/CLAP plugins and plays audio through them.
//!
//! ## Setup
//!
//! Download free plugins and place in `assets/plugins/`:
//! - Dragonfly Room Reverb: https://github.com/michaelwillis/dragonfly-reverb/releases
//! - Surge XT: https://github.com/surge-synthesizer/releases-xt/releases

use std::time::Duration;
use tutti::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Runtime::new()?;

    // Create audio engine with plugin support
    let engine = TuttiEngine::builder()
        .sample_rate(44100.0)
        .plugin_runtime(runtime.handle().clone())
        .build()?;

    // Try to load a plugin (example paths - adjust to your system)
    let plugin_paths = [
        "/Library/Audio/Plug-Ins/VST3/DragonflyRoomReverb.vst3",
        "/usr/lib/vst3/DragonflyRoomReverb.vst3",
        "assets/plugins/DragonflyRoomReverb.vst3",
    ];

    let mut loaded = false;
    for path in &plugin_paths {
        if std::path::Path::new(path).exists() {
            match engine.load_vst3("reverb", path) {
                Ok(_) => {
                    println!("Loaded plugin from: {}", path);
                    loaded = true;
                    break;
                }
                Err(e) => println!("Failed to load {}: {}", path, e),
            }
        }
    }

    if !loaded {
        println!("No plugin found. Install DragonflyRoomReverb or adjust plugin_paths.");
        return Ok(());
    }

    // Create instances
    use tutti::dsp::sine_hz;
    let sine_id = engine.graph(|net| net.add(Box::new(sine_hz::<f32>(440.0))));
    let reverb = engine.instance("reverb", &params! {})?;

    // Connect graph
    engine.graph(|net| {
        net.pipe(sine_id, reverb);
        net.pipe_output(reverb);
    });

    println!("Playing sine through plugin reverb...");
    engine.transport().play();
    std::thread::sleep(Duration::from_secs(5));

    Ok(())
}
