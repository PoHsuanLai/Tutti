<div align="center">
  <img src="logo/logo.png" alt="Tutti Logo" width="200"/>
</div>

# Tutti

[![Crates.io](https://img.shields.io/crates/v/tutti.svg)](https://crates.io/crates/tutti)
[![Documentation](https://docs.rs/tutti/badge.svg)](https://docs.rs/tutti)
[![License](https://img.shields.io/crates/l/tutti.svg)](https://github.com/PoHsuanLai/Tutti#license)
[![CI](https://github.com/PoHsuanLai/tutti/workflows/CI/badge.svg)](https://github.com/PoHsuanLai/Tutti/actions)

A real-time audio engine for DAW applications in Rust. Tutti provides an audio graph runtime, MIDI processing, sample playback, plugin hosting, and neural audio synthesis.

For audio UI components, see [Armas](https://github.com/PoHsuanLai/Armas).

## Overview

Umbrella crate that coordinates multiple audio subsystems:

- **[tutti-core]** - Audio graph runtime (Net, Transport, Metering, PDC)
- **[tutti-midi]** - MIDI subsystem (I/O, MPE, MIDI 2.0, CC mapping)
- **[tutti-sampler]** - Sample playback (Butler, streaming, recording, time-stretch)
- **[tutti-dsp]** - DSP nodes (LFO, dynamics, envelope follower, spatial audio)
- **[tutti-plugin]** - Plugin hosting (VST2, VST3, CLAP)
- **[tutti-neural]** - Neural audio (GPU synthesis and effects)
- **[tutti-analysis]** - Audio analysis (waveform, transient, pitch, correlation)
- **[tutti-export]** - Offline rendering and export

## Quick Start

```rust
use tutti::prelude::*;

let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;
let registry = NodeRegistry::default();

// Build audio graph with macros
engine.graph(|net| {
    // Create nodes from registry
    let sine = registry.create("sine", &params! {
        "frequency" => 440.0
    }).unwrap();

    let filter = registry.create("lowpass", &params! {
        "cutoff" => 2000.0,
        "q" => 1.0
    }).unwrap();

    let reverb = registry.create("reverb_stereo", &params! {
        "room_size" => 0.8,
        "time" => 3.0
    }).unwrap();

    // Chain nodes: sine → filter → reverb → output
    let sine_id = net.add(sine);
    let filter_id = net.add(filter);
    let reverb_id = net.add(reverb);

    chain!(net, sine_id, filter_id, reverb_id => output);
});

engine.transport().play();
```

## Features

- `default` - Core audio engine only
- `full` - Everything enabled (excludes `neural`)
- `midi` - MIDI subsystem
- `sampler` - Sample playback and recording
- `soundfont` - SoundFont support (requires `sampler`)
- `plugin` - Plugin hosting (VST2/VST3/CLAP)
- `neural` - Neural audio processing (GPU synthesis and effects)
- `analysis` - Audio analysis tools
- `export` - Offline rendering
- `spatial-audio` - VBAP and binaural panning

## Architecture

Tutti uses a modular architecture where each subsystem is an independent crate. The `NodeRegistry` provides dynamic node creation from plugins, neural models, and builtin DSP nodes.

## Examples

### NodeRegistry - Plugins and Neural Models

```rust
use tutti::prelude::*;

let registry = NodeRegistry::default();
let neural = NeuralSystem::builder().sample_rate(44100.0).build()?;

// Register plugins and neural models
register_plugin_directory(&registry, tokio_handle, "path/to/plugins")?;
register_neural_model(&registry, &neural, "my_synth", "model.mpk")?;

// Create nodes dynamically by name
let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;
engine.graph(|net| {
    let synth = registry.create("my_synth", &params! {}).unwrap();
    let reverb = registry.create("reverb_plugin", &params! {
        "room_size" => 0.9
    }).unwrap();

    let synth_id = net.add(synth);
    let reverb_id = net.add(reverb);

    chain!(net, synth_id, reverb_id => output);
});
```

### With MIDI and Sampler

```rust
use tutti::prelude::*;

let engine = TuttiEngine::builder()
    .midi()
    .sampler()
    .build()?;

// Access subsystems
if let Some(midi) = engine.midi() {
    // MIDI operations
}

if let Some(sampler) = engine.sampler() {
    // Sample playback
}
```

### Using Individual Crates

You can also use subsystems independently without the umbrella crate:

```rust
// Just core audio graph
use tutti_core::TuttiSystem;

let system = TuttiSystem::builder().build()?;
system.graph(|net| {
    // Build DSP graph
});
```

```rust
// Just MIDI
use tutti_midi::MidiSystem;

let midi = MidiSystem::new().build()?;
```

## Testing

See [TESTING.md](TESTING.md) for setup instructions.

Quick examples:

```bash
# Plugin loading (see example docs for setup)
cargo run --example plugin_loading --features plugin

# Neural models (run assets/models/create_test_model.py first)
cargo run --example neural_models --features neural

# MIDI synthesizer
cargo run --example midi_synth --features "midi,synth"
```

## License

MIT OR Apache-2.0

[tutti-core]: https://crates.io/crates/tutti-core
[tutti-midi]: https://crates.io/crates/tutti-midi
[tutti-sampler]: https://crates.io/crates/tutti-sampler
[tutti-dsp]: https://crates.io/crates/tutti-dsp
[tutti-plugin]: https://crates.io/crates/tutti-plugin
[tutti-neural]: https://crates.io/crates/tutti-neural
[tutti-analysis]: https://crates.io/crates/tutti-analysis
[tutti-export]: https://crates.io/crates/tutti-export
