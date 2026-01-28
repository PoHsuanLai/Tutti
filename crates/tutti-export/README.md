# Tutti Export

Offline audio export and rendering for the Tutti audio engine.

## Overview

Provides offline rendering and export to WAV and FLAC formats with DSP processing:

- **Offline rendering** - Render audio graphs to memory buffers
- **Format encoding** - Export to WAV, FLAC (pure Rust)
- **DSP utilities** - Resampling, dithering, loudness normalization (LUFS)

The export system is instruction-driven and framework-agnostic.

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

## Features

- **Resampling** - Sample rate conversion using rubato
- **Dithering** - Rectangular, triangular, and noise-shaped dithering
- **Normalization** - Peak and LUFS/EBU R128 loudness normalization

## Feature Flags

- `wav` (default) - WAV export via hound
- `flac` (default) - FLAC export via flacenc
- `butler` - Butler thread integration for async disk I/O

## License

MIT OR Apache-2.0
