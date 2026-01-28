# Tutti Neural

Neural audio synthesis and effects on the GPU.

## What this is

Neural audio processing for DAW applications. Supports synthesis (DDSP, vocoders) where inference generates control parameters on a separate thread, and effects (amp sims, compressors) that process audio directly.

Uses [Burn](https://github.com/tracel-ai/burn) for ML inference with wgpu backend. Lock-free queues move control parameters from inference thread to audio thread.

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

## How it works

**Synthesis**: Neural model runs on inference thread, outputs control parameters (pitch, amplitude, filter coefficients) which go through a lock-free queue to the audio thread. Audio thread renders DSP based on these parameters.

**Effects**: Neural model processes audio directly on audio thread. Must complete within buffer deadline.

**Batching**: Multiple voices/tracks can be batched into a single GPU call for efficiency.

## License

MIT OR Apache-2.0
