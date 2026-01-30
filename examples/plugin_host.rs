//! Plugin hosting example: Load and use VST/CLAP plugins
//!
//! Demonstrates: Plugin discovery, loading, parameter control
//!
//! Run with: cargo run --example plugin_host

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use tutti::prelude::*;

    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    println!("Plugin hosting example");
    println!("Note: This is a skeleton example showing the API surface.");
    println!("To use actual plugins, you need to:");
    println!("  1. Install VST2/VST3/CLAP plugins");
    println!("  2. Configure plugin search paths");
    println!("  3. Use the plugin loading API");
    println!();

    // Example workflow (API not yet implemented):
    // let plugin = engine.load_plugin("/path/to/plugin.vst")?;
    // plugin.set_parameter(0, 0.5); // Set first parameter to 50%
    //
    // engine.graph(|net| {
    //     let input = net.add(Box::new(sine_hz::<f64>(440.0) * 0.5));
    //     let processed = net.add_plugin(plugin);
    //     net.pipe(input, processed);
    //     net.pipe_output(processed);
    // });

    // For now, just play a sine wave
    engine.graph(|net| {
        let sine = net.add(Box::new(sine_hz::<f64>(440.0) * 0.5));
        net.pipe_output(sine);
    });

    engine.transport().play();

    println!("Playing sine wave (plugin hosting not yet implemented)");
    println!("Press Ctrl+C to exit.");

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
