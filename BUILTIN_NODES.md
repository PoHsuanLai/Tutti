# Built-in Audio Nodes Reference

This document lists all AudioUnit implementations provided by Tutti. These are building blocks you can use to construct your audio graph.

## Table of Contents

- [Synthesizers](#synthesizers)
- [Samplers](#samplers)
- [DSP Nodes](#dsp-nodes)
- [Spatial Audio](#spatial-audio)
- [FunDSP Nodes](#fundsp-nodes)

---

## Synthesizers

### PolySynth

Simple polyphonic synthesizer with multiple waveforms and ADSR envelopes.

**Features:** Requires `synth` feature

**Usage:**
```rust
use tutti::prelude::*;

// Register PolySynth as a node type
engine.add_node("polysynth", |params| {
    let sample_rate = params.get("sample_rate")?.as_f64()? as f32;
    let voices = params.get("voices").and_then(|v| v.as_i64()).unwrap_or(8) as usize;

    let waveform = match params.get("waveform").and_then(|v| v.as_str()) {
        Some("sine") => Waveform::Sine,
        Some("saw") => Waveform::Saw,
        Some("square") => Waveform::Square,
        Some("triangle") => Waveform::Triangle,
        _ => Waveform::Saw,
    };

    let envelope = Envelope {
        attack: params.get("attack").and_then(|v| v.as_f64()).unwrap_or(0.01) as f32,
        decay: params.get("decay").and_then(|v| v.as_f64()).unwrap_or(0.1) as f32,
        sustain: params.get("sustain").and_then(|v| v.as_f64()).unwrap_or(0.7) as f32,
        release: params.get("release").and_then(|v| v.as_f64()).unwrap_or(0.2) as f32,
    };

    let mut synth = PolySynth::new(sample_rate, voices);
    synth.set_waveform(waveform);
    synth.set_envelope(envelope);
    Ok(Box::new(synth))
})?;

// Instantiate with parameters
let synth = engine.create("polysynth", &params! {
    "sample_rate" => 44100.0,
    "voices" => 8,
    "waveform" => "saw",
    "attack" => 0.01,
    "decay" => 0.1,
    "sustain" => 0.7,
    "release" => 0.2,
})?;
```

**Parameters:**
- `waveform` - Oscillator type: `"sine"`, `"saw"`, `"square"`, `"triangle"`
- `voices` - Max polyphony (default: 8)
- `attack` - Attack time in seconds (default: 0.01)
- `decay` - Decay time in seconds (default: 0.1)
- `sustain` - Sustain level 0.0-1.0 (default: 0.7)
- `release` - Release time in seconds (default: 0.2)

**MIDI Support:**
```rust
// Create with MIDI registry for automatic MIDI routing
let synth = PolySynth::with_midi(sample_rate, voices, midi_registry);
```

### SoundFontUnit

Sample-based synthesis using SoundFont 2.0 files.

**Features:** Requires `soundfont` feature

**Usage:**
```rust
use tutti::prelude::*;

// Load SoundFont (registers automatically)
engine.load_sf2("piano", "piano.sf2")?;

// Instantiate with optional preset/channel
let piano = engine.create("piano", &params! {
    "preset" => 0,   // Optional: SoundFont preset number
    "channel" => 0,  // Optional: MIDI channel
})?;
```

**Parameters:**
- `preset` - SoundFont preset number (0-127)
- `channel` - MIDI channel (0-15)

---

## Samplers

### SamplerUnit

One-shot sample playback with trigger control.

**Features:** Requires `sampler` feature

**Usage:**
```rust
use tutti::prelude::*;

// Load audio file
let wave = tutti_sampler::load_wav("kick.wav")?;

// Create sampler
let sampler = SamplerUnit::new(wave);

// Add to graph
let kick = engine.graph(|net| net.add(Box::new(sampler)));

// Trigger playback
sampler.trigger();
```

**Methods:**
- `trigger()` - Start playback from beginning
- `set_gain(f32)` - Set volume
- `set_speed(f32)` - Set playback speed (pitch)
- `stop()` - Stop playback

### StreamingSamplerUnit

Streaming playback for large audio files using Butler thread.

**Features:** Requires `sampler` feature with `butler` sub-feature

**Usage:**
```rust
use tutti::prelude::*;

// Use high-level streaming API (recommended)
let sampler = engine.sampler();
sampler.stream("long_audio.wav")
    .channel(0)
    .gain(0.8)
    .speed(1.0)
    .start();
```

### TimeStretchUnit

Real-time time-stretching and pitch-shifting.

**Features:** Requires `sampler` feature

**Usage:**
```rust
use tutti::prelude::*;

let wave = tutti_sampler::load_wav("vocal.wav")?;
let sampler = SamplerUnit::new(wave);

// Wrap with time-stretcher
let params = TimeStretchParams {
    time_stretch: 0.8,    // 80% speed (slower)
    pitch_shift: 1.2,     // 20% higher pitch
    formant_preserve: 0.5, // Preserve formants
};

let stretched = TimeStretchUnit::new(Box::new(sampler), 44100.0);
stretched.set_parameters(params);
```

**Parameters:**
- `time_stretch` - Speed multiplier (0.5 = half speed, 2.0 = double speed)
- `pitch_shift` - Pitch multiplier (0.5 = octave down, 2.0 = octave up)
- `formant_preserve` - Formant preservation 0.0-1.0

### AudioInput / AudioInputBackend

Hardware audio input capture.

**Features:** Requires `sampler` feature with `audio-input` sub-feature

**Usage:**
```rust
use tutti::prelude::*;

// Create audio input
let (input, backend) = AudioInput::new(2, 512); // 2 channels, 512 buffer

// Add to graph
let input_node = engine.graph(|net| net.add(Box::new(input)));

// In audio callback, write captured samples:
// backend.write(&captured_samples);
```

---

## DSP Nodes

### LfoNode

Low-frequency oscillator for modulation.

**Features:** Always available

**Usage:**
```rust
use tutti::prelude::*;

let lfo = LfoNode::new(44100.0)
    .frequency(2.0)           // 2 Hz
    .shape(LfoShape::Sine)    // Sine wave
    .mode(LfoMode::Unipolar); // 0.0 to 1.0 range

let lfo_node = engine.graph(|net| net.add(Box::new(lfo)));
```

**Shapes:**
- `LfoShape::Sine` - Smooth sine wave
- `LfoShape::Triangle` - Linear ramp up/down
- `LfoShape::Saw` - Sawtooth (ramp up, drop)
- `LfoShape::Square` - On/off square wave
- `LfoShape::Random` - Sample & hold random

**Modes:**
- `LfoMode::FreeRunning` - Self-oscillating at specified frequency
- `LfoMode::BeatSynced` - Synced to beat position input

### Envelope Following

For envelope detection, use the `afollow` function from the DSP module:

```rust
use tutti::dsp::afollow;

// Envelope follower with 10ms attack, 100ms release
let env = afollow(0.01, 0.1);
```

Or for symmetric smoothing, use `follow`:

```rust
use tutti::dsp::follow;

// Simple smoothing filter with 50ms response time
let smooth = follow(0.05);
```

### SidechainCompressor / StereoSidechainCompressor

Dynamic range compression triggered by external sidechain signal.

**Features:** Always available

**Usage:**
```rust
use tutti::prelude::*;

// Mono sidechain compressor
let comp = SidechainCompressor::new(44100.0)
    .threshold(-20.0)    // dB
    .ratio(4.0)          // 4:1 compression
    .attack(0.005)       // 5ms
    .release(0.1)        // 100ms
    .makeup_gain(3.0);   // dB

// Stereo version
let stereo_comp = StereoSidechainCompressor::new(44100.0)
    .threshold(-20.0)
    .ratio(4.0)
    .attack(0.005)
    .release(0.1);
```

**Parameters:**
- `threshold` - Compression threshold in dB
- `ratio` - Compression ratio (1.0 = no compression, 10.0 = limiting)
- `attack` - Attack time in seconds
- `release` - Release time in seconds
- `makeup_gain` - Post-compression gain in dB
- `knee` - Soft knee width in dB (optional)

### SidechainGate / StereoSidechainGate

Noise gate triggered by external sidechain signal.

**Features:** Always available

**Usage:**
```rust
use tutti::prelude::*;

let gate = SidechainGate::new(44100.0)
    .threshold(-40.0)    // dB - gate opens above this
    .attack(0.001)       // 1ms
    .hold(0.05)          // 50ms hold time
    .release(0.1);       // 100ms

let stereo_gate = StereoSidechainGate::new(44100.0)
    .threshold(-40.0)
    .attack(0.001)
    .release(0.1);
```

**Parameters:**
- `threshold` - Gate threshold in dB
- `attack` - Attack time in seconds
- `hold` - Hold time before closing in seconds
- `release` - Release time in seconds
- `range` - Gate range in dB (how much attenuation when closed)

---

## Spatial Audio

### BinauralPannerNode

HRTF-based binaural panning for headphone listening.

**Features:** Always available

**Usage:**
```rust
use tutti::prelude::*;

let panner = BinauralPannerNode::new(44100.0)
    .azimuth(45.0)      // Degrees (-180 to 180)
    .elevation(0.0)     // Degrees (-90 to 90)
    .distance(1.0);     // Distance in meters

let panned = engine.graph(|net| net.add(Box::new(panner)));
```

**Parameters:**
- `azimuth` - Horizontal angle in degrees (-180° to 180°, 0° = front)
- `elevation` - Vertical angle in degrees (-90° to 90°, 0° = horizontal)
- `distance` - Distance in meters (affects level and filtering)

### SpatialPannerNode

VBAP (Vector Base Amplitude Panning) for multi-speaker setups.

**Features:** Always available

**Usage:**
```rust
use tutti::prelude::*;

// Create with speaker layout
let panner = SpatialPannerNode::stereo()?;  // Stereo (L/R)
// Or:
let panner = SpatialPannerNode::quad()?;         // Quad
let panner = SpatialPannerNode::surround_5_1()?; // 5.1 surround
let panner = SpatialPannerNode::surround_7_1()?; // 7.1 surround

// Custom speaker positions
let positions = vec![
    (0.0, 0.0),      // Front
    (-30.0, 0.0),    // Left
    (30.0, 0.0),     // Right
];
let panner = SpatialPannerNode::custom(&positions)?;

// Set position
panner.set_position(45.0, 0.0);  // Azimuth, elevation
```

**Presets:**
- `stereo()` - Standard stereo (L/R)
- `quad()` - Quadraphonic (FL/FR/BL/BR)
- `surround_5_1()` - 5.1 surround sound
- `surround_7_1()` - 7.1 surround sound

---

## FunDSP Nodes

All FunDSP nodes are available via `tutti::dsp::*` or `tutti::prelude::*`.

### Oscillators

```rust
use tutti::prelude::*;

sine_hz(440.0)         // Sine wave at 440 Hz
saw_hz(110.0)          // Sawtooth at 110 Hz
square_hz(220.0)       // Square wave at 220 Hz
triangle_hz(330.0)     // Triangle wave at 330 Hz
pulse()                // Pulse oscillator
organ(220.0)           // Organ-like sound
hammond(220.0)         // Hammond organ emulation
```

### Filters

```rust
use tutti::prelude::*;

lowpass_hz(1000.0, 1.0)      // Lowpass filter (cutoff, Q)
highpass_hz(1000.0, 1.0)     // Highpass filter
bandpass_hz(1000.0, 1.0)     // Bandpass filter
notch_hz(1000.0, 1.0)        // Notch filter
peak_hz(1000.0, 1.0)         // Peaking EQ
bell_hz(1000.0, 1.0, 6.0)    // Bell EQ (freq, Q, gain_db)
lowshelf_hz(100.0, 1.0, 6.0) // Low shelf EQ
highshelf_hz(8000.0, 1.0, 6.0) // High shelf EQ

moog_hz(1000.0, 0.5)         // Moog ladder filter
resonator_hz(440.0, 100.0)   // Resonator (freq, bandwidth)
butterpass_hz(1000.0)        // Butterworth lowpass

// Linkwitz-Riley crossovers (4th order)
lr_lowpass_hz(1000.0)        // LR lowpass
lr_highpass_hz(1000.0)       // LR highpass
```

### Effects

```rust
use tutti::prelude::*;

reverb_stereo(0.5, 5.0, 1.0)  // Stereo reverb (room size, time, damping)
chorus(0, 0.5, 0.5, 0.5)      // Chorus (seed, rate, depth, feedback)
flanger(0.5, 0.5, 0.5)        // Flanger (rate, depth, feedback)
phaser(0.5, 0.5, 0.5)         // Phaser (rate, depth, feedback)

delay(1.0)                     // Delay (time in seconds)
feedback(delay(0.5) * 0.5)     // Feedback loop
limiter_stereo((1.0, 2.0))     // Stereo limiter (attack, release)
```

### Dynamics

```rust
use tutti::prelude::*;

limiter(0.001, 0.01)          // Limiter (attack, release)
clip()                         // Hard clipping
clip_to(0.5)                   // Clip to specific level
shape(Shape::Tanh)             // Waveshaping (Tanh, Hardclip, etc.)
```

### Envelopes

```rust
use tutti::prelude::*;

adsr_live(0.01, 0.1, 0.7, 0.2) // ADSR envelope (A, D, S, R)
envelope(|t| ...)               // Custom envelope function
lfo(|t| ...)                    // LFO with function
follow(0.01, 0.1)               // Envelope follower (attack, release)
afollow(0.01, 0.1)              // Asymmetric envelope follower
```

### Spatial

```rust
use tutti::prelude::*;

pan(0.0)                       // Stereo pan (-1.0 to 1.0)
panner(0.5)                    // Constant power panning (0.0 to 1.0)
rotate(1.0, 0.5)               // Rotate stereo field (speed, amount)
```

### Noise

```rust
use tutti::prelude::*;

white()                        // White noise
pink()                         // Pink noise
brown()                        // Brown noise
noise()                        // Uniform noise [-1, 1]
```

### Utilities

```rust
use tutti::prelude::*;

pass()                         // Pass-through (identity)
sink()                         // Discard output (silent)
zero()                         // Output zeros
dc(0.5)                        // DC offset
constant(440.0)                // Constant value

split()                        // Split into stereo
join()                         // Join stereo to mono

mul(0.5)                       // Multiply by constant
add(0.1)                       // Add constant
```

### Graph Operators

Combine FunDSP nodes using operators:

```rust
use tutti::prelude::*;

// Series (pipe)
sine_hz(440.0) >> lowpass_hz(1000.0, 1.0) >> mul(0.5)

// Parallel (sum/mix)
sine_hz(440.0) + saw_hz(220.0)

// Branch (duplicate signal)
let osc = sine_hz(440.0);
osc ^ (lowpass_hz(1000.0, 1.0) + highpass_hz(100.0, 1.0))

// Stack (multichannel)
sine_hz(440.0) | sine_hz(550.0)  // Stereo output
```

---

## Using Built-in Nodes

### Method 1: Direct instantiation (FunDSP)

```rust
use tutti::prelude::*;

engine.graph(|net| {
    let osc = net.add(Box::new(sine_hz(440.0)));
    let filter = net.add(Box::new(lowpass_hz(1000.0, 1.0)));
    net.connect(osc, 0, filter, 0);
    net.pipe_output(filter);
});
```

### Method 2: Via add_node() (reusable)

```rust
use tutti::prelude::*;

// Register once
engine.add_node("bass", |params| {
    let freq = get_param_or(params, "frequency", 110.0);
    Ok(Box::new(sine_hz(freq) >> lowpass_hz(200.0, 1.0)))
})?;

// Instantiate many times
let bass1 = engine.create("bass", &params! { "frequency" => 110.0 })?;
let bass2 = engine.create("bass", &params! { "frequency" => 55.0 })?;
```

### Method 3: Via load_*() helpers (files)

```rust
use tutti::prelude::*;

// Load external resources
engine.load_sf2("piano", "piano.sf2")?;
engine.load_synth_mpk("synth", "model.mpk")?;
engine.load_vst3("reverb", "plugin.vst3")?;

// Then instance
let piano = engine.create("piano", &params! {})?;
```

---

## See Also

- [README.md](README.md) - Main documentation
- [examples/](examples/) - Example code
- [FunDSP Documentation](https://docs.rs/fundsp) - Complete FunDSP reference
