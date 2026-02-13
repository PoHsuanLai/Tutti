# Testing Tutti

Quick guide for testing plugins and neural models.

## Quick Start

### Test Plugins

1. Download a free plugin:
   - [Dragonfly Room Reverb](https://github.com/michaelwillis/dragonfly-reverb/releases) (recommended, ~5MB)
   - [Surge XT](https://github.com/surge-synthesizer/releases-xt/releases) (~50MB)

2. Place the .vst3 file in `assets/plugins/`

3. Run example:
   ```bash
   cargo run --example plugin_loading --features plugin
   ```

### Test Neural Models

1. Create a test model:
   ```bash
   cd assets/models
   python create_test_model.py
   ```

   Requirements: `pip install torch onnxscript`

2. Run examples:
   ```bash
   cargo run --example neural_models --features neural
   cargo run --example neural_model_save_load --features neural
   ```

## What Gets Tested

### Plugin System
- VST3 and CLAP plugin loading
- Plugin scanning (assets dir + system directories)
- Creating nodes from plugins via NodeRegistry
- Audio routing through plugins

### Neural System
- Neural system initialization
- ONNX → Burn `.mpk` conversion workflow
- Model registration with NodeRegistry
- Model save/load functionality

## Examples

### `plugin_loading.rs`
Tests plugin hosting by:
- Scanning for plugins in `assets/plugins/` and system directories
- Creating a sine wave → reverb plugin → output chain
- Falls back to built-in reverb if no plugins found

### `neural_models.rs`
Shows neural model registration:
- Creating NeuralSystem
- Registering models from files
- Using NodeRegistry for model management

### `neural_model_save_load.rs`
Documents the complete workflow:
- Training in PyTorch
- Exporting to ONNX
- Converting with `burn-import`
- Loading in Tutti

## Requirements

### Plugins
- None (downloads are manual)
- Recommended: Dragonfly Room Reverb

### Neural Models
- Python 3: `pip install torch onnxscript`
- burn-import: `cargo install burn-import --features onnx` (for conversion)

## File Structure

```
tutti/
├── assets/
│   ├── plugins/                  # VST3/CLAP plugins (manual download)
│   │   └── README.md
│   └── models/                   # Neural models
│       ├── create_test_model.py  # Python script to create test model
│       ├── README.md
│       ├── simple_synth.onnx     # Created by Python script
│       └── simple_synth.mpk      # Converted by onnx2burn
└── examples/
    ├── plugin_loading.rs
    ├── neural_models.rs
    └── neural_model_save_load.rs
```

## Troubleshooting

**"No plugins found"**
- Download plugins from the links in the example docs
- Place `.vst3` files in `assets/plugins/`
- The example will auto-detect plugins in that directory

**"Model not found"**
- Run: `cd assets/models && python create_test_model.py`
- Convert: `onnx2burn simple_synth.onnx simple_synth.mpk`
- Requires: `pip install torch onnxscript`

**Using your own models:**
```rust
// 1. Register model with NodeRegistry
let neural = NeuralSystem::builder().sample_rate(44100.0).build()?;
let registry = NodeRegistry::default();
register_neural_model(&registry, &neural, "my_model", "path/to/model.mpk")?;

// 2. Create node from registry
let synth = registry.create("my_model", &params! {})?;

// 3. Use in audio graph
engine.graph_mut(|net| {
    let id = net.add(synth);
    net.pipe_output(id);
});
```
