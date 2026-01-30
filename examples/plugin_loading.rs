//! Plugin Loading Example
//!
//! Loads VST3/CLAP plugins and plays audio through them.
//!
//! ## Setup
//!
//! Download free plugins and place in `assets/plugins/`:
//! - Dragonfly Room Reverb: https://github.com/michaelwillis/dragonfly-reverb/releases
//! - Surge XT: https://github.com/surge-synthesizer/releases-xt/releases

use std::path::PathBuf;
use std::time::Duration;
use tutti::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Runtime::new()?;
    let handle = runtime.handle();
    let registry = NodeRegistry::default();

    // Try assets/plugins first, then system directories
    let assets_path = PathBuf::from("assets/plugins");
    let mut plugins = if assets_path.exists() {
        register_plugin_directory(&registry, &handle, &assets_path).ok()
    } else {
        None
    };

    if plugins.as_ref().map_or(true, |p| p.is_empty()) {
        plugins = register_all_system_plugins(&registry, &handle).ok();
    }

    if let Some(ref p) = plugins {
        println!("Loaded {} plugins", p.len());
    }

    // Create audio engine with sine -> reverb -> output
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    engine.graph(|net| {
        let sine = registry
            .create("sine", &params! { "frequency" => 440.0 })
            .unwrap();
        let sine_id = net.add(sine);

        // Try plugin reverb, fallback to builtin
        let reverb_id = ["DragonflyRoomReverb", "ValhallaFreqEcho", "CloudReverb"]
            .iter()
            .find_map(|name| {
                registry
                    .create(name, &params! { "sample_rate" => 44100.0 })
                    .ok()
                    .map(|r| net.add(r))
            })
            .unwrap_or_else(|| {
                let r = registry
                    .create(
                        "reverb_stereo",
                        &params! {
                            "room_size" => 0.8,
                            "time" => 3.0
                        },
                    )
                    .unwrap();
                net.add(r)
            });

        net.pipe(sine_id, reverb_id);
        net.pipe_output(reverb_id);
    });

    println!("Playing for 3 seconds...");
    engine.transport().play();
    std::thread::sleep(Duration::from_secs(3));

    Ok(())
}
