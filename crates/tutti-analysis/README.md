# Tutti Analysis

Audio analysis tools for the Tutti audio engine.

## Overview

Efficient algorithms for audio analysis:

- **Waveform thumbnails** - Multi-resolution min/max/RMS summaries for visualization
- **Transient detection** - Onset/beat detection using spectral flux
- **Pitch detection** - Monophonic pitch tracking using the YIN algorithm
- **Stereo correlation** - Phase correlation, stereo width, and balance analysis

All functions operate on raw `&[f32]` sample buffers - no framework dependencies.

## Quick Start

```rust
use tutti_analysis::{
    waveform::compute_summary,
    TransientDetector,
    PitchDetector,
    CorrelationMeter,
};

let samples: Vec<f32> = vec![0.0; 44100]; // 1 second of audio
let sample_rate = 44100.0;

// Waveform thumbnail
let summary = compute_summary(&samples, 1, 512);

// Transient detection
let mut detector = TransientDetector::new(sample_rate);
let transients = detector.detect(&samples);

// Pitch detection
let mut pitch_detector = PitchDetector::new(sample_rate);
let pitch = pitch_detector.detect(&samples);

// Stereo correlation
let left = &samples[..];
let right = &samples[..];
let mut meter = CorrelationMeter::new(sample_rate);
let analysis = meter.process(left, right);
```

## Features

- `cache` - LRU cache for waveform thumbnails
- `serialization` - Serde support for analysis results

## License

MIT OR Apache-2.0
