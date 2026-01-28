# Tutti Plugin

Multi-process plugin hosting for Tutti.

## Overview

Multi-process architecture for loading VST2, VST3, and CLAP plugins in isolated server processes.

**Benefits:**
- **Crash isolation** - Plugin crashes don't crash the DAW
- **Security** - Malicious plugins sandboxed from main process
- **Memory safety** - Separate address spaces

## Quick Start

```rust
use tutti_plugin::{PluginClient, BridgeConfig};

// Start plugin server process
let mut client = PluginClient::new(BridgeConfig::default())?;
client.init().await?;

// Load plugin
client.load_plugin("/path/to/plugin.vst3", 44100.0).await?;

// Process audio
let buffer = AudioBuffer { /* ... */ };
client.process(&mut buffer);
```

## Features

- **32-bit and 64-bit audio** - Full support for both sample formats
- **Zero-copy shared memory** - Audio buffers passed via mmap
- **Sample-accurate MIDI** - Frame-precise event timing
- **Full transport context** - Tempo, time signature, play state

## License

MIT OR Apache-2.0
