//! # 05 - Effect Chain
//!
//! Process audio through multiple effects: oscillator → filter → reverb.
//!
//! **Concepts:** Effect nodes, audio graph routing, signal flow
//!
//! ```bash
//! cargo run --example 05_effect_chain
//! ```

use std::time::Duration;
use tutti::prelude::*;

fn main() -> tutti::Result<()> {
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    engine.graph(|net| {
        let saw = net.add(saw_hz(110.0) * 0.3).id();
        let filter = net.add(lowpole_hz::<f64>(800.0)).id();
        let stereo = net.add_split();
        let reverb = net.add(reverb_stereo(10.0, 2.0, 0.5)).id();

        net.pipe(saw, filter);
        net.pipe(filter, stereo);
        net.pipe_all(stereo, reverb);
        net.pipe_output(reverb);
    });

    engine.transport().play();
    println!("Playing: saw → lowpass → reverb...");
    std::thread::sleep(Duration::from_secs(5));

    Ok(())
}
