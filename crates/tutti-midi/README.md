# Tutti MIDI

MIDI subsystem for the Tutti audio engine.

## Overview

Provides comprehensive MIDI functionality:

- **Port management** - Virtual MIDI ports for routing
- **Hardware I/O** - Device enumeration and real-time I/O (feature: `midi-io`)
- **MPE** - MIDI Polyphonic Expression (feature: `mpe`)
- **MIDI 2.0** - High-resolution messages (feature: `midi2`)
- **CC mapping** - MIDI learn and parameter control
- **Output collection** - Lock-free MIDI output from audio nodes

## Quick Start

```rust
use tutti_midi::MidiSystem;

// Basic system
let midi = MidiSystem::builder().build()?;

// Send MIDI event
midi.send_note_on(0, 60, 100)?;
```

## Features

- `midi-io` - Hardware device I/O (midir)
- `mpe` - MIDI Polyphonic Expression
- `midi2` - MIDI 2.0 support

## License

MIT OR Apache-2.0
