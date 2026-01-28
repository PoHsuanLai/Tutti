# Tutti

Real-time audio engine built from modular subsystems.

## Overview

Tutti is an umbrella crate that coordinates multiple audio subsystems into a unified engine:

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

// Create engine (capabilities depend on enabled features)
let engine = TuttiEngine::builder()
    .sample_rate(44100.0)
    .build()?;

// Build audio graph
engine.graph(|net| {
    let osc = net.add(Box::new(sine_hz(440.0)));
    net.pipe_output(osc);
});

// Control transport
engine.transport().play();
```

## Features

- `default` - Core audio engine only
- `full` - Everything enabled
- `midi` - MIDI subsystem
- `sampler` - Sample playback and recording
- `soundfont` - SoundFont support (requires `sampler`)
- `plugin` - Plugin hosting (VST2/VST3/CLAP)
- `neural` - Neural audio processing
- `analysis` - Audio analysis tools
- `export` - Offline rendering
- `spatial-audio` - VBAP and binaural panning

## Architecture

Tutti uses a modular architecture where each subsystem is an independent crate:

```
TuttiEngine
├── Core (always present)
│   ├── Audio graph (FunDSP Net)
│   ├── Transport
│   ├── Metering
│   └── PDC
├── MIDI (optional)
├── Sampler (optional)
├── Neural (optional)
└── ... other subsystems
```

## Examples

### With MIDI and Sampler

```rust
use tutti::prelude::*;

let engine = TuttiEngine::builder()
    .with_midi()
    .with_sampler()
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

let midi = MidiSystem::builder().build()?;
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
