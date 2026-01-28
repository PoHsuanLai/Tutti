# Tutti DSP

Real-time DSP building blocks for the Tutti audio engine.

## Overview

`tutti-dsp` provides AudioUnit nodes for:

- **LFO** - Low frequency oscillators with multiple waveforms
- **Envelope Follower** - Peak and RMS envelope detection
- **Dynamics** - Compressors and gates with sidechain support
- **Spatial Audio** - VBAP and binaural panning for immersive audio

All nodes are RT-safe and use lock-free atomics for parameter control.

## Quick Start

```rust
use tutti_dsp::{LfoNode, LfoShape};
use tutti_core::AudioUnit;

// Create an LFO
let mut lfo = LfoNode::new(LfoShape::Sine, 2.0);
lfo.set_sample_rate(44100.0);
lfo.set_depth(0.8);

// Process audio
let mut output = [0.0f32; 1];
lfo.tick(&[], &mut output);
```

## Examples

### Envelope Follower

```rust
use tutti_dsp::{EnvelopeFollowerNode, EnvelopeMode};

// Peak envelope detection
let mut env = EnvelopeFollowerNode::new(0.001, 0.1);  // 1ms attack, 100ms release
env.set_sample_rate(44100.0);

// Or RMS mode
let mut env_rms = EnvelopeFollowerNode::new_rms(0.001, 0.1, 10.0);  // 10ms window
```

### Sidechain Compressor

```rust
use tutti_dsp::SidechainCompressor;

let mut comp = SidechainCompressor::new(-20.0, 4.0, 0.001, 0.05);
comp.set_sample_rate(44100.0);

// Process: audio input on channel 0, sidechain on channel 1
let input = [audio_sample, sidechain_sample];
let mut output = [0.0f32];
comp.tick(&input, &mut output);
```

## Features

- `spatial-audio` - VBAP and binaural panning

## License

MIT OR Apache-2.0
