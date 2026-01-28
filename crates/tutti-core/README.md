# Tutti Core

Low-level audio graph runtime for the Tutti audio engine.

## Overview

`tutti-core` provides the foundational audio processing infrastructure:

- **Audio Graph** - FunDSP Net for DSP graph execution
- **Transport** - Playback control with tempo, time signatures, and BBT
- **Metering** - Real-time level monitoring with LUFS/EBU R128 support
- **PDC** - Plugin delay compensation
- **Lock-free primitives** - AtomicFloat, AtomicDouble for RT-safe parameter control

This is a **low-level crate**. Most users should use the [tutti] umbrella crate instead.

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
