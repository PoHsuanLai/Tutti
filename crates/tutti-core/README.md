# Tutti Core

Audio graph runtime and transport management.

## What this is

Core audio processing infrastructure for DAW applications. Built on FunDSP's Net for the audio graph, with transport control (tempo, time signatures, BBT positioning), real-time level metering, and plugin delay compensation. Uses atomic types (AtomicFloat, AtomicDouble) for parameter updates from the UI thread.

Most users should use the [tutti] umbrella crate instead.

## Quick Start

```rust
use tutti_core::TuttiSystem;

let system = TuttiSystem::builder()
    .sample_rate(44100.0)
    .build()?;

// Build audio graph
system.graph(|net| {
    use fundsp::prelude::*;
    let osc = net.add(Box::new(sine_hz(440.0)));
    net.pipe_output(osc);
});

// Transport control
system.transport().play();
```

## Features

- `default` - Core functionality
- `midi` - MIDI event types and conversion
- `neural` - Neural audio integration

## Architecture

```
TuttiSystem
├── TuttiNet (FunDSP wrapper)
├── TransportManager
├── MeteringManager
├── PdcManager
└── Metronome
```

## License

MIT OR Apache-2.0

[tutti]: https://crates.io/crates/tutti
