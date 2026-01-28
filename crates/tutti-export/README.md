# Tutti Export

Offline rendering and audio export.

## What this is

Renders audio graphs offline (faster than real-time) and exports to WAV/FLAC. Includes sample rate conversion, dithering (rectangular, triangular, noise-shaped), and loudness normalization (EBU R128).

Uses [hound](https://crates.io/crates/hound) for WAV, [flacenc](https://crates.io/crates/flacenc) for FLAC, [rubato](https://crates.io/crates/rubato) for resampling, and [ebur128](https://crates.io/crates/ebur128) for loudness metering.

## Quick Start

```rust
use tutti_export::{OfflineRenderer, ExportOptions, AudioFormat};

// Create renderer
let renderer = OfflineRenderer::new(44100);

// Render audio (simplified example)
let left = vec![0.0f32; 44100];  // 1 second
let right = vec![0.0f32; 44100];

// Export to file
let options = ExportOptions::default();
tutti_export::export_wav("output.wav", &left, &right, &options)?;
```

## Feature Flags

- `wav` (default) - WAV export via hound
- `flac` (default) - FLAC export via flacenc
- `butler` - Butler thread integration for async disk I/O

## License

MIT OR Apache-2.0
