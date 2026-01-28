# Tutti Neural

GPU-accelerated neural audio synthesis and effects for Tutti.

## Overview

Neural audio processing with GPU acceleration:

- **Neural synthesis** - GPU-accelerated audio synthesis (DDSP, vocoders)
- **Neural effects** - Real-time effects processing (amp sims, compressors)
- **Lock-free queues** - GPU → audio thread communication
- **Model management** - Model caching and loading

## Quick Start

```rust
use tutti_neural::NeuralSystem;

let neural = NeuralSystem::builder()
    .sample_rate(44100.0)
    .buffer_size(512)
    .build()?;

let model = neural.load_synth_model("violin.onnx")?;
let voice = neural.synth().build_voice(&model)?;
```

## Architecture

Two-tier for performance and safety:
1. **WASM Extensions** - Sandboxed, validated WebGPU-style API (~1.5ms latency)
2. **Native processing** - Direct GPU access for maximum performance (~0.5ms latency)

### Synthesis (DDSP-like)

Neural models generate control parameters:
- Inference thread → Lock-free queue → Audio thread
- Examples: DDSP, WaveRNN, neural vocoders
- ~0.5ms latency with look-ahead prefetching

### Effects (Direct Processing)

Neural models process audio samples directly on audio thread:
- Must be RT-safe (<1ms processing time)
- Examples: Amp simulators, compressors, neural reverbs

## Performance

- **Batch processing** - 8 tracks in 1 GPU call = 8x speedup
- **Lock-free queues** - Zero audio thread blocking
- **Look-ahead prefetch** - Inference runs ahead of audio callback

## License

MIT OR Apache-2.0
