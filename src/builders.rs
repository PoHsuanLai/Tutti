//! Fluent builders for audio resource loading.
//!
//! All builders return `AudioUnit` implementations that users add to the graph.
//! Resources are cached internally for efficiency.
//!
//! # Example
//!
//! ```ignore
//! // SoundFont
//! let piano = engine.sf2("piano.sf2").preset(0).build()?;
//! engine.graph(|net| net.add(piano).master());
//!
//! // Audio samples
//! let kick = engine.wav("kick.wav").build()?;
//! let snare = engine.flac("snare.flac").speed(0.8).build()?;
//!
//! // Compose before adding
//! engine.graph(|net| {
//!     net.add(kick >> reverb);
//! });
//! ```

use crate::Result;
use std::path::PathBuf;

// ============================================================================
// SoundFont Builder
// ============================================================================

/// Fluent builder for SoundFont synthesis.
///
/// Created via `engine.sf2(path)`. Loads the SoundFont file (cached) and creates
/// a synthesizer instance with the specified preset.
///
/// # Example
///
/// ```ignore
/// let piano = engine.sf2("piano.sf2")
///     .preset(0)
///     .channel(0)
///     .build()?;
/// engine.graph(|net| net.add(piano).master());
/// ```
#[cfg(feature = "soundfont")]
pub struct Sf2Builder<'a> {
    engine: &'a crate::TuttiEngine,
    path: PathBuf,
    preset: i32,
    channel: i32,
}

#[cfg(feature = "soundfont")]
impl<'a> Sf2Builder<'a> {
    /// Create a new SoundFont builder.
    pub(crate) fn new(engine: &'a crate::TuttiEngine, path: PathBuf) -> Self {
        Self {
            engine,
            path,
            preset: 0,
            channel: 0,
        }
    }

    /// Set the preset number (0-127).
    ///
    /// Default: 0 (piano on most SoundFonts)
    pub fn preset(mut self, preset: i32) -> Self {
        self.preset = preset;
        self
    }

    /// Set the MIDI channel (0-15).
    ///
    /// Default: 0
    pub fn channel(mut self, channel: i32) -> Self {
        self.channel = channel;
        self
    }

    /// Build the SoundFont synthesizer unit.
    ///
    /// Returns a `SoundFontUnit` that can be added to the audio graph.
    /// The SoundFont file is loaded and cached if not already loaded.
    pub fn build(self) -> Result<crate::synth::SoundFontUnit> {
        // Access the soundfont system through the engine
        let soundfont_system = self.engine.soundfont_system();

        // Load (or get cached) SoundFont
        let handle = soundfont_system.load(&self.path).map_err(|e| {
            crate::Error::InvalidConfig(format!("Failed to load SoundFont: {:?}", e))
        })?;

        let soundfont = soundfont_system.get(&handle).ok_or_else(|| {
            crate::Error::InvalidConfig("SoundFont not found in cache".to_string())
        })?;

        let settings = soundfont_system.default_settings();

        // Create unit with or without MIDI registry
        #[cfg(feature = "midi")]
        let mut unit = {
            let midi_registry = self.engine.graph(|net| net.midi_registry().clone());
            crate::synth::SoundFontUnit::with_midi(soundfont, &settings, midi_registry)
        };

        #[cfg(not(feature = "midi"))]
        let mut unit = crate::synth::SoundFontUnit::new(soundfont, &settings);

        // Apply preset
        unit.program_change(self.channel, self.preset);

        Ok(unit)
    }
}

// ============================================================================
// Audio Sample Builders
// ============================================================================

/// Fluent builder for WAV audio samples.
///
/// Created via `engine.wav(path)`. Loads the audio file (cached) and creates
/// a sampler unit for playback.
///
/// # Example
///
/// ```ignore
/// let kick = engine.wav("kick.wav")
///     .gain(0.8)
///     .speed(1.2)
///     .looping(true)
///     .build()?;
/// engine.graph(|net| net.add(kick).master());
/// ```
#[cfg(all(feature = "sampler", feature = "wav"))]
pub struct WavBuilder<'a> {
    engine: &'a crate::TuttiEngine,
    path: PathBuf,
    gain: f32,
    speed: f32,
    looping: bool,
}

#[cfg(all(feature = "sampler", feature = "wav"))]
impl<'a> WavBuilder<'a> {
    /// Create a new WAV builder.
    pub(crate) fn new(engine: &'a crate::TuttiEngine, path: PathBuf) -> Self {
        Self {
            engine,
            path,
            gain: 1.0,
            speed: 1.0,
            looping: false,
        }
    }

    /// Set playback gain (0.0 - 1.0+).
    ///
    /// Default: 1.0
    pub fn gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }

    /// Set playback speed multiplier.
    ///
    /// Default: 1.0 (original speed)
    pub fn speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// Enable or disable looping.
    ///
    /// Default: false
    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }

    /// Build the sampler unit.
    ///
    /// Returns a `SamplerUnit` that can be added to the audio graph.
    /// The audio file is loaded and cached if not already loaded.
    pub fn build(self) -> Result<crate::sampler::SamplerUnit> {
        let wave = self.engine.load_wave_cached(&self.path)?;
        Ok(crate::sampler::SamplerUnit::with_settings(
            wave,
            self.gain,
            self.speed,
            self.looping,
        ))
    }
}

/// Fluent builder for FLAC audio samples.
#[cfg(all(feature = "sampler", feature = "flac"))]
pub struct FlacBuilder<'a> {
    engine: &'a crate::TuttiEngine,
    path: PathBuf,
    gain: f32,
    speed: f32,
    looping: bool,
}

#[cfg(all(feature = "sampler", feature = "flac"))]
impl<'a> FlacBuilder<'a> {
    /// Create a new FLAC builder.
    pub(crate) fn new(engine: &'a crate::TuttiEngine, path: PathBuf) -> Self {
        Self {
            engine,
            path,
            gain: 1.0,
            speed: 1.0,
            looping: false,
        }
    }

    /// Set playback gain.
    pub fn gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }

    /// Set playback speed multiplier.
    pub fn speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// Enable or disable looping.
    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }

    /// Build the sampler unit.
    pub fn build(self) -> Result<crate::sampler::SamplerUnit> {
        let wave = self.engine.load_wave_cached(&self.path)?;
        Ok(crate::sampler::SamplerUnit::with_settings(
            wave,
            self.gain,
            self.speed,
            self.looping,
        ))
    }
}

/// Fluent builder for MP3 audio samples.
#[cfg(all(feature = "sampler", feature = "mp3"))]
pub struct Mp3Builder<'a> {
    engine: &'a crate::TuttiEngine,
    path: PathBuf,
    gain: f32,
    speed: f32,
    looping: bool,
}

#[cfg(all(feature = "sampler", feature = "mp3"))]
impl<'a> Mp3Builder<'a> {
    /// Create a new MP3 builder.
    pub(crate) fn new(engine: &'a crate::TuttiEngine, path: PathBuf) -> Self {
        Self {
            engine,
            path,
            gain: 1.0,
            speed: 1.0,
            looping: false,
        }
    }

    /// Set playback gain.
    pub fn gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }

    /// Set playback speed multiplier.
    pub fn speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// Enable or disable looping.
    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }

    /// Build the sampler unit.
    pub fn build(self) -> Result<crate::sampler::SamplerUnit> {
        let wave = self.engine.load_wave_cached(&self.path)?;
        Ok(crate::sampler::SamplerUnit::with_settings(
            wave,
            self.gain,
            self.speed,
            self.looping,
        ))
    }
}

/// Fluent builder for OGG Vorbis audio samples.
#[cfg(all(feature = "sampler", feature = "ogg"))]
pub struct OggBuilder<'a> {
    engine: &'a crate::TuttiEngine,
    path: PathBuf,
    gain: f32,
    speed: f32,
    looping: bool,
}

#[cfg(all(feature = "sampler", feature = "ogg"))]
impl<'a> OggBuilder<'a> {
    /// Create a new OGG builder.
    pub(crate) fn new(engine: &'a crate::TuttiEngine, path: PathBuf) -> Self {
        Self {
            engine,
            path,
            gain: 1.0,
            speed: 1.0,
            looping: false,
        }
    }

    /// Set playback gain.
    pub fn gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }

    /// Set playback speed multiplier.
    pub fn speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// Enable or disable looping.
    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }

    /// Build the sampler unit.
    pub fn build(self) -> Result<crate::sampler::SamplerUnit> {
        let wave = self.engine.load_wave_cached(&self.path)?;
        Ok(crate::sampler::SamplerUnit::with_settings(
            wave,
            self.gain,
            self.speed,
            self.looping,
        ))
    }
}

// ============================================================================
// Plugin Builders
// ============================================================================

/// Load a plugin in-process. The plugin runs on a dedicated thread
/// in the same process, enabling GUI editor support.
#[cfg(feature = "plugin")]
fn load_plugin(
    engine: &crate::TuttiEngine,
    path: PathBuf,
    params: &std::collections::HashMap<String, f32>,
) -> Result<(Box<dyn crate::AudioUnit>, crate::plugin::PluginHandle)> {
    let sample_rate = engine.sample_rate();
    let block_size = 512;

    // Determine plugin format from file extension and load
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let instance: Box<dyn crate::plugin::PluginInstance> = match ext.as_str() {
        #[cfg(feature = "clap")]
        "clap" => {
            let mut inst = tutti_plugin_server::clap_loader::ClapInstance::load(
                &path,
                sample_rate,
                block_size,
            )
            .map_err(|e| crate::Error::InvalidConfig(format!("CLAP load failed: {e}")))?;
            // Activate on the main thread (CLAP requirement).
            // start_processing() will be called lazily on the bridge thread
            // by the first process_f32() call.
            inst.activate()
                .map_err(|e| crate::Error::InvalidConfig(format!("CLAP activate failed: {e}")))?;
            Box::new(inst)
        }
        #[cfg(feature = "vst3")]
        "vst3" => {
            let inst = tutti_plugin_server::vst3_loader::Vst3Instance::load(
                &path,
                sample_rate,
                block_size,
            )
            .map_err(|e| crate::Error::InvalidConfig(format!("VST3 load failed: {e}")))?;
            Box::new(inst)
        }
        #[cfg(feature = "vst2")]
        "dll" | "so" | "vst" => {
            let inst = tutti_plugin_server::vst2_loader::Vst2Instance::load(
                &path,
                sample_rate,
                block_size,
            )
            .map_err(|e| crate::Error::InvalidConfig(format!("VST2 load failed: {e}")))?;
            Box::new(inst)
        }
        _ => {
            return Err(crate::Error::InvalidConfig(format!(
                "Unsupported plugin format: .{ext}"
            )));
        }
    };

    let metadata = instance.metadata().clone();
    let num_channels = metadata.audio_io.inputs.max(metadata.audio_io.outputs).max(2);

    let (bridge, thread_handle) =
        crate::plugin::InProcessBridge::new(instance, num_channels, block_size);

    let bridge_arc: std::sync::Arc<dyn crate::plugin::PluginBridge> = std::sync::Arc::new(bridge);

    // Apply initial parameters
    for (name, value) in params {
        if let Ok(param_id) = name.parse::<u32>() {
            bridge_arc.set_parameter_rt(param_id, *value);
        }
    }

    // Create PluginClient for AudioUnit impl
    let mut client = crate::plugin::PluginClient::from_bridge(
        bridge_arc.clone(),
        metadata.clone(),
        block_size,
    );

    // Inject MIDI registry so engine.note_on() reaches the plugin
    #[cfg(feature = "midi")]
    {
        let midi_registry = engine.graph(|net| net.midi_registry().clone());
        client.set_midi_registry(midi_registry);
    }

    let plugin_handle =
        crate::plugin::PluginHandle::from_bridge_and_metadata(bridge_arc, metadata);

    engine.store_inprocess_handle(thread_handle, plugin_handle.clone());

    Ok((Box::new(client), plugin_handle))
}

/// Fluent builder for VST3 plugins.
///
/// Created via `engine.vst3(path)`. Loads the plugin in-process (GUI editor works).
///
/// # Example
///
/// ```ignore
/// let (reverb, handle) = engine.vst3("Reverb.vst3")
///     .param("room_size", 0.8)
///     .build()?;
/// handle.open_editor(window_handle);
/// ```
#[cfg(all(feature = "plugin", feature = "vst3"))]
pub struct Vst3Builder<'a> {
    engine: &'a crate::TuttiEngine,
    path: PathBuf,
    params: std::collections::HashMap<String, f32>,
}

#[cfg(all(feature = "plugin", feature = "vst3"))]
impl<'a> Vst3Builder<'a> {
    pub(crate) fn new(engine: &'a crate::TuttiEngine, path: PathBuf) -> Self {
        Self {
            engine,
            path,
            params: std::collections::HashMap::new(),
        }
    }

    /// Set a plugin parameter by numeric ID.
    pub fn param(mut self, name: impl Into<String>, value: f32) -> Self {
        self.params.insert(name.into(), value);
        self
    }

    /// Build the plugin instance.
    pub fn build(self) -> Result<(Box<dyn crate::AudioUnit>, crate::plugin::PluginHandle)> {
        load_plugin(self.engine, self.path, &self.params)
    }
}

/// Fluent builder for VST2 plugins.
#[cfg(all(feature = "plugin", feature = "vst2"))]
pub struct Vst2Builder<'a> {
    engine: &'a crate::TuttiEngine,
    path: PathBuf,
    params: std::collections::HashMap<String, f32>,
}

#[cfg(all(feature = "plugin", feature = "vst2"))]
impl<'a> Vst2Builder<'a> {
    pub(crate) fn new(engine: &'a crate::TuttiEngine, path: PathBuf) -> Self {
        Self {
            engine,
            path,
            params: std::collections::HashMap::new(),
        }
    }

    /// Set a plugin parameter by numeric ID.
    pub fn param(mut self, name: impl Into<String>, value: f32) -> Self {
        self.params.insert(name.into(), value);
        self
    }

    /// Build the plugin instance.
    pub fn build(self) -> Result<(Box<dyn crate::AudioUnit>, crate::plugin::PluginHandle)> {
        load_plugin(self.engine, self.path, &self.params)
    }
}

/// Fluent builder for CLAP plugins.
#[cfg(all(feature = "plugin", feature = "clap"))]
pub struct ClapBuilder<'a> {
    engine: &'a crate::TuttiEngine,
    path: PathBuf,
    params: std::collections::HashMap<String, f32>,
}

#[cfg(all(feature = "plugin", feature = "clap"))]
impl<'a> ClapBuilder<'a> {
    pub(crate) fn new(engine: &'a crate::TuttiEngine, path: PathBuf) -> Self {
        Self {
            engine,
            path,
            params: std::collections::HashMap::new(),
        }
    }

    /// Set a plugin parameter by numeric ID.
    pub fn param(mut self, name: impl Into<String>, value: f32) -> Self {
        self.params.insert(name.into(), value);
        self
    }

    /// Build the plugin instance.
    pub fn build(self) -> Result<(Box<dyn crate::AudioUnit>, crate::plugin::PluginHandle)> {
        load_plugin(self.engine, self.path, &self.params)
    }
}

// ============================================================================
// Neural Builders
// ============================================================================

/// Fluent builder for neural synth models.
///
/// Created via `engine.neural_synth(path)`. Loads the model (cached) and creates
/// a synth voice instance.
///
/// # Example
///
/// ```ignore
/// let violin = engine.neural_synth("violin.mpk").build()?;
/// engine.graph(|net| net.add_neural(violin, model_id).master());
/// ```
#[cfg(all(feature = "neural", feature = "midi"))]
pub struct NeuralSynthBuilder<'a> {
    engine: &'a crate::TuttiEngine,
    path: PathBuf,
}

#[cfg(all(feature = "neural", feature = "midi"))]
impl<'a> NeuralSynthBuilder<'a> {
    /// Create a new neural synth builder.
    pub(crate) fn new(engine: &'a crate::TuttiEngine, path: PathBuf) -> Self {
        Self { engine, path }
    }

    /// Build the neural synth voice.
    ///
    /// Returns the voice unit and its model ID for batched inference.
    pub fn build(self) -> Result<(Box<dyn crate::AudioUnit>, crate::NeuralModelId)> {
        let neural = self.engine.neural();
        let path_str = self.path.to_str().ok_or_else(|| {
            crate::Error::InvalidConfig("Invalid UTF-8 in path".to_string())
        })?;

        let builder = neural.load_synth(path_str).map_err(|e| {
            crate::Error::InvalidConfig(format!("Failed to load neural synth model: {}", e))
        })?;

        let model_id = builder.model_id();
        let voice = builder.build_voice().map_err(|e| {
            crate::Error::InvalidConfig(format!("Failed to build neural voice: {}", e))
        })?;

        Ok((voice, model_id))
    }
}

/// Fluent builder for neural effect models.
///
/// Created via `engine.neural_effect(path)`. Loads the model (cached) and creates
/// an effect instance.
#[cfg(feature = "neural")]
pub struct NeuralEffectBuilder<'a> {
    engine: &'a crate::TuttiEngine,
    path: PathBuf,
}

#[cfg(feature = "neural")]
impl<'a> NeuralEffectBuilder<'a> {
    /// Create a new neural effect builder.
    pub(crate) fn new(engine: &'a crate::TuttiEngine, path: PathBuf) -> Self {
        Self { engine, path }
    }

    /// Build the neural effect.
    ///
    /// Returns the effect unit and its model ID for batched inference.
    pub fn build(self) -> Result<(Box<dyn crate::AudioUnit>, crate::NeuralModelId)> {
        let neural = self.engine.neural();
        let path_str = self.path.to_str().ok_or_else(|| {
            crate::Error::InvalidConfig("Invalid UTF-8 in path".to_string())
        })?;

        let builder = neural.load_effect(path_str).map_err(|e| {
            crate::Error::InvalidConfig(format!("Failed to load neural effect model: {}", e))
        })?;

        let model_id = builder.model_id();
        let effect = builder.build_effect().map_err(|e| {
            crate::Error::InvalidConfig(format!("Failed to build neural effect: {}", e))
        })?;

        Ok((effect, model_id))
    }
}

/// Fluent builder for neural synth from closure.
///
/// Created via `engine.neural_synth_fn(closure)`. Wraps a user-provided
/// inference function as a neural synth.
#[cfg(all(feature = "neural", feature = "midi"))]
pub struct NeuralSynthFnBuilder<'a, F>
where
    F: Fn(&[f32]) -> Vec<f32> + Send + 'static,
{
    engine: &'a crate::TuttiEngine,
    infer_fn: F,
}

#[cfg(all(feature = "neural", feature = "midi"))]
impl<'a, F> NeuralSynthFnBuilder<'a, F>
where
    F: Fn(&[f32]) -> Vec<f32> + Send + 'static,
{
    /// Create a new neural synth function builder.
    pub(crate) fn new(engine: &'a crate::TuttiEngine, infer_fn: F) -> Self {
        Self { engine, infer_fn }
    }

    /// Build the neural synth voice from the closure.
    ///
    /// Returns the voice unit and its model ID for batched inference.
    pub fn build(self) -> Result<(Box<dyn crate::AudioUnit>, crate::NeuralModelId)> {
        // Get MIDI registry for pull-based MIDI delivery
        let midi_registry = self.engine.graph(|net| net.midi_registry().clone());

        let neural_handle = self.engine.neural();
        let neural_system = neural_handle.inner().ok_or_else(|| {
            crate::Error::InvalidConfig("Neural subsystem not enabled".into())
        })?;

        let builder = neural_system
            .register_synth("_closure_synth", self.infer_fn, Some(midi_registry))
            .map_err(|e| {
                crate::Error::InvalidConfig(format!("Failed to register neural synth: {}", e))
            })?;

        let model_id = builder.model_id();
        let voice = builder.build_voice().map_err(|e| {
            crate::Error::InvalidConfig(format!("Failed to build neural synth: {}", e))
        })?;

        Ok((voice, model_id))
    }
}

/// Fluent builder for neural effect from closure.
#[cfg(feature = "neural")]
pub struct NeuralEffectFnBuilder<'a, F>
where
    F: Fn(&[f32]) -> Vec<f32> + Send + 'static,
{
    engine: &'a crate::TuttiEngine,
    infer_fn: F,
}

#[cfg(feature = "neural")]
impl<'a, F> NeuralEffectFnBuilder<'a, F>
where
    F: Fn(&[f32]) -> Vec<f32> + Send + 'static,
{
    /// Create a new neural effect function builder.
    pub(crate) fn new(engine: &'a crate::TuttiEngine, infer_fn: F) -> Self {
        Self { engine, infer_fn }
    }

    /// Build the neural effect from the closure.
    ///
    /// Returns the effect unit and its model ID for batched inference.
    pub fn build(self) -> Result<(Box<dyn crate::AudioUnit>, crate::NeuralModelId)> {
        let neural_handle = self.engine.neural();
        let neural_system = neural_handle.inner().ok_or_else(|| {
            crate::Error::InvalidConfig("Neural subsystem not enabled".into())
        })?;

        let builder = neural_system
            .register_effect("_closure_effect", self.infer_fn)
            .map_err(|e| {
                crate::Error::InvalidConfig(format!("Failed to register neural effect: {}", e))
            })?;

        let model_id = builder.model_id();
        let effect = builder.build_effect().map_err(|e| {
            crate::Error::InvalidConfig(format!("Failed to build neural effect: {}", e))
        })?;

        Ok((effect, model_id))
    }
}
