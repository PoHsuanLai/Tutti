//! # 17 - Plugin Loading
//!
//! Load and use VST3/CLAP plugins in the audio graph.
//!
//! **Concepts:** `vst3()`, plugin hosting, tokio runtime
//!
//! ```bash
//! cargo run --example 17_plugin_loading --features plugin
//! ```
//!
//! ## Setup
//!
//! Install a free VST3 plugin:
//! - [Dragonfly Reverb](https://github.com/michaelwillis/dragonfly-reverb/releases)
//! - [Surge XT](https://github.com/surge-synthesizer/releases-xt/releases)

use std::time::Duration;
use tutti::prelude::*;

fn main() -> tutti::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;

    let engine = TuttiEngine::builder()
        .sample_rate(44100.0)
        .plugin_runtime(runtime.handle().clone())
        .build()?;

    let plugin_paths = [
        "/Library/Audio/Plug-Ins/VST3/DragonflyRoomReverb.vst3",
        "/usr/lib/vst3/DragonflyRoomReverb.vst3",
        "assets/plugins/DragonflyRoomReverb.vst3",
    ];

    // New fluent API: engine.vst3(path).build() returns AudioUnit
    let mut reverb_unit = None;
    for path in &plugin_paths {
        if std::path::Path::new(path).exists() {
            if let Ok(unit) = engine.vst3(path).build() {
                println!("Loaded: {}", path);
                reverb_unit = Some(unit);
                break;
            }
        }
    }

    let reverb_unit = match reverb_unit {
        Some(unit) => unit,
        None => {
            println!("No plugin found. Install DragonflyRoomReverb or adjust plugin_paths.");
            return Ok(());
        }
    };

    let sine_id = engine.graph(|net| net.add(sine_hz::<f32>(440.0) * 0.3).id());
    let reverb = engine.graph(|net| net.add_boxed(reverb_unit).id());

    engine.graph(|net| {
        net.pipe(sine_id, reverb);
        net.pipe_output(reverb);
    });

    engine.transport().play();
    println!("Playing sine → reverb...");
    std::thread::sleep(Duration::from_secs(5));

    Ok(())
}
