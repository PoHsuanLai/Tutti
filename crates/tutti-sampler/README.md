# Tutti Sampler

Audio streaming, recording, and sample playback for the Tutti audio engine.

## Overview

Provides disk streaming, audio input recording, and time-stretching:

- **Disk streaming** - Butler thread for asynchronous I/O with ring buffers
- **Audio input** - Hardware capture with lock-free MPMC channels
- **Recording** - MIDI, audio, and pattern recording
- **Time-stretching** - Real-time pitch and tempo manipulation via phase vocoder
- **SoundFont** - SoundFont synthesis (feature: `soundfont`)

## Quick Start

```rust
use tutti_sampler::SamplerSystem;

let sampler = SamplerSystem::builder(44100.0).build()?;

// Stream audio file
sampler.stream_file(0, "audio.wav").gain(0.8).start();
```

## Features

- `soundfont` - SoundFont synthesis support

## License

MIT OR Apache-2.0
