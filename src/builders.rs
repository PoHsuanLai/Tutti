//! Fluent builders for audio resource loading.
//!
//! All builders return `AudioUnit` implementations that users add to the graph.
//! Resources are cached internally for efficiency.

use crate::Result;
use std::path::PathBuf;
use std::sync::Arc;

/// Fluent builder for SoundFont synthesis.
///
/// Created via `engine.sf2(path)`. Loads the SoundFont file (cached) and creates
/// a synthesizer instance with the specified preset.
#[cfg(feature = "soundfont")]
pub struct Sf2Builder<'a> {
    engine: &'a crate::TuttiEngine,
    path: PathBuf,
    preset: i32,
    channel: i32,
}

#[cfg(feature = "soundfont")]
impl<'a> Sf2Builder<'a> {
    pub(crate) fn new(engine: &'a crate::TuttiEngine, path: PathBuf) -> Self {
        Self {
            engine,
            path,
            preset: 0,
            channel: 0,
        }
    }

    /// Preset number (0-127). Default: 0 (piano on most SoundFonts).
    pub fn preset(mut self, preset: i32) -> Self {
        self.preset = preset;
        self
    }

    /// MIDI channel (0-15). Default: 0.
    pub fn channel(mut self, channel: i32) -> Self {
        self.channel = channel;
        self
    }

    /// The SoundFont file is loaded and cached if not already loaded.
    pub fn build(self) -> Result<crate::synth::SoundFontUnit> {
        let soundfont_system = self.engine.soundfont_system();
        let handle = soundfont_system.load(&self.path)?;

        let soundfont = soundfont_system.get(&handle).ok_or_else(|| {
            tutti_core::Error::InvalidConfig("SoundFont not found in cache".to_string())
        })?;

        let settings = soundfont_system.default_settings();

        #[cfg(feature = "midi")]
        let mut unit = {
            let midi_registry = self.engine.graph_mut(|net| net.midi_registry().clone());
            crate::synth::SoundFontUnit::with_midi(soundfont, &settings, midi_registry)
        };

        #[cfg(not(feature = "midi"))]
        let mut unit = crate::synth::SoundFontUnit::new(soundfont, &settings);

        unit.program_change(self.channel, self.preset);

        Ok(unit)
    }
}

/// Fluent builder for audio samples (WAV, FLAC, MP3, OGG).
///
/// Created via `engine.wav(path)`, `engine.flac(path)`, etc. Loads the audio
/// file (cached) and creates a sampler unit for playback.
#[cfg(feature = "sampler")]
pub struct SampleBuilder<'a> {
    engine: &'a crate::TuttiEngine,
    path: PathBuf,
    gain: f32,
    speed: f32,
    looping: bool,
    start_beat: Option<f64>,
    duration_beats: f64,
}

#[cfg(feature = "sampler")]
impl<'a> SampleBuilder<'a> {
    pub(crate) fn new(engine: &'a crate::TuttiEngine, path: PathBuf) -> Self {
        Self {
            engine,
            path,
            gain: 1.0,
            speed: 1.0,
            looping: false,
            start_beat: None,
            duration_beats: 0.0,
        }
    }

    /// Playback gain (0.0 - 1.0+). Default: 1.0.
    pub fn gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }

    /// Speed multiplier. Default: 1.0 (original speed).
    pub fn speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// Default: false.
    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }

    /// Place on the timeline at a beat position. Enables transport-aware
    /// playback: the sampler only produces audio when the playhead is within range.
    pub fn start_beat(mut self, beat: f64) -> Self {
        self.start_beat = Some(beat);
        self
    }

    /// Duration in beats for transport-aware playback. 0.0 = entire sample.
    pub fn duration_beats(mut self, beats: f64) -> Self {
        self.duration_beats = beats;
        self
    }

    /// Tries the cache first; if not cached, loads synchronously and caches.
    pub fn build(self) -> Result<crate::sampler::SamplerUnit> {
        let wave = match self.engine.get_wave_cached(&self.path) {
            Ok(w) => w,
            Err(_) => {
                let w = tutti_core::Wave::load_with_progress(&self.path, |_| {}).map_err(|e| {
                    crate::Error::Core(tutti_core::Error::InvalidConfig(format!(
                        "Failed to load {}: {}",
                        self.path.display(),
                        e
                    )))
                })?;
                let w = Arc::new(w);
                self.engine.cache_wave(&self.path, w.clone());
                w
            }
        };
        let mut unit =
            crate::sampler::SamplerUnit::with_settings(wave, self.gain, self.speed, self.looping);
        if let Some(start) = self.start_beat {
            let transport = self.engine.transport();
            unit.set_transport(Arc::new(transport), start, self.duration_beats);
        }
        Ok(unit)
    }
}

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
            )?;
            // Activate on the main thread (CLAP requirement).
            // start_processing() will be called lazily on the bridge thread
            // by the first process_f32() call.
            inst.activate()?;
            Box::new(inst)
        }
        #[cfg(feature = "vst3")]
        "vst3" => {
            let inst = tutti_plugin_server::vst3_loader::Vst3Instance::load(
                &path,
                sample_rate,
                block_size,
            )?;
            Box::new(inst)
        }
        #[cfg(feature = "vst2")]
        "dll" | "so" | "vst" => {
            let inst = tutti_plugin_server::vst2_loader::Vst2Instance::load(
                &path,
                sample_rate,
                block_size,
            )?;
            Box::new(inst)
        }
        _ => {
            return Err(tutti_core::Error::InvalidConfig(format!(
                "Unsupported plugin format: .{ext}"
            ))
            .into());
        }
    };

    let metadata = instance.metadata().clone();
    let num_channels = metadata
        .audio_io
        .inputs
        .max(metadata.audio_io.outputs)
        .max(2);

    let (bridge, thread_handle) =
        crate::plugin::InProcessBridge::new(instance, num_channels, block_size);

    let bridge_arc: std::sync::Arc<dyn crate::plugin::PluginBridge> = std::sync::Arc::new(bridge);

    for (name, value) in params {
        if let Ok(param_id) = name.parse::<u32>() {
            bridge_arc.set_parameter_rt(param_id, *value);
        }
    }

    let mut client =
        crate::plugin::PluginClient::from_bridge(bridge_arc.clone(), metadata.clone(), block_size);

    // Inject MIDI registry so engine.note_on() reaches the plugin
    #[cfg(feature = "midi")]
    {
        let midi_registry = engine.graph_mut(|net| net.midi_registry().clone());
        client.set_midi_registry(midi_registry);
    }

    let plugin_handle = crate::plugin::PluginHandle::from_bridge_and_metadata(bridge_arc, metadata);

    engine.store_inprocess_handle(thread_handle, plugin_handle.clone());

    Ok((Box::new(client), plugin_handle))
}

/// Fluent builder for audio plugins (VST3, VST2, CLAP).
///
/// Created via `engine.vst3(path)`, `engine.vst2(path)`, or `engine.clap(path)`.
/// Loads the plugin in-process (GUI editor works).
#[cfg(feature = "plugin")]
pub struct PluginBuilder<'a> {
    engine: &'a crate::TuttiEngine,
    path: PathBuf,
    params: std::collections::HashMap<String, f32>,
}

#[cfg(feature = "plugin")]
impl<'a> PluginBuilder<'a> {
    pub(crate) fn new(engine: &'a crate::TuttiEngine, path: PathBuf) -> Self {
        Self {
            engine,
            path,
            params: std::collections::HashMap::new(),
        }
    }

    pub fn param(mut self, name: impl Into<String>, value: f32) -> Self {
        self.params.insert(name.into(), value);
        self
    }

    pub fn build(self) -> Result<(Box<dyn crate::AudioUnit>, crate::plugin::PluginHandle)> {
        load_plugin(self.engine, self.path, &self.params)
    }
}

/// Fluent builder for neural synth models.
///
/// Created via `engine.neural_synth(path)`. Loads the model (cached) and creates
/// a synth voice instance.
#[cfg(all(feature = "neural", feature = "midi"))]
pub struct NeuralSynthBuilder<'a> {
    engine: &'a crate::TuttiEngine,
    path: PathBuf,
}

#[cfg(all(feature = "neural", feature = "midi"))]
impl<'a> NeuralSynthBuilder<'a> {
    pub(crate) fn new(engine: &'a crate::TuttiEngine, path: PathBuf) -> Self {
        Self { engine, path }
    }

    /// Returns the voice unit and its model ID for batched inference.
    pub fn build(self) -> Result<(Box<dyn crate::AudioUnit>, crate::NeuralModelId)> {
        let neural = self.engine.neural();
        let path_str = self
            .path
            .to_str()
            .ok_or_else(|| tutti_core::Error::InvalidConfig("Invalid UTF-8 in path".to_string()))?;

        let builder = neural.load_synth(path_str)?;

        let model_id = builder.model_id();
        let voice = builder.build_voice()?;

        Ok((voice, model_id))
    }
}

/// Fluent builder for neural effect models.
///
/// Created via `engine.neural_effect(path)`. Loads the model (cached).
#[cfg(feature = "neural")]
pub struct NeuralEffectBuilder<'a> {
    engine: &'a crate::TuttiEngine,
    path: PathBuf,
}

#[cfg(feature = "neural")]
impl<'a> NeuralEffectBuilder<'a> {
    pub(crate) fn new(engine: &'a crate::TuttiEngine, path: PathBuf) -> Self {
        Self { engine, path }
    }

    /// Returns the effect unit and its model ID for batched inference.
    pub fn build(self) -> Result<(Box<dyn crate::AudioUnit>, crate::NeuralModelId)> {
        let neural = self.engine.neural();
        let path_str = self
            .path
            .to_str()
            .ok_or_else(|| tutti_core::Error::InvalidConfig("Invalid UTF-8 in path".to_string()))?;

        let builder = neural.load_effect(path_str)?;

        let model_id = builder.model_id();
        let effect = builder.build_effect()?;

        Ok((effect, model_id))
    }
}

/// Wraps a user-provided inference closure as a neural synth.
///
/// Created via `engine.neural_synth_fn(closure)`.
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
    pub(crate) fn new(engine: &'a crate::TuttiEngine, infer_fn: F) -> Self {
        Self { engine, infer_fn }
    }

    /// Returns the voice unit and its model ID for batched inference.
    pub fn build(self) -> Result<(Box<dyn crate::AudioUnit>, crate::NeuralModelId)> {
        let midi_registry = self.engine.graph_mut(|net| net.midi_registry().clone());

        let neural_handle = self.engine.neural();
        let neural_system = neural_handle.inner().ok_or_else(|| {
            tutti_core::Error::InvalidConfig("Neural subsystem not enabled".into())
        })?;

        let builder =
            neural_system.register_synth("_closure_synth", self.infer_fn, Some(midi_registry))?;

        let model_id = builder.model_id();
        let voice = builder.build_voice()?;

        Ok((voice, model_id))
    }
}

/// Wraps a user-provided inference closure as a neural effect.
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
    pub(crate) fn new(engine: &'a crate::TuttiEngine, infer_fn: F) -> Self {
        Self { engine, infer_fn }
    }

    /// Returns the effect unit and its model ID for batched inference.
    pub fn build(self) -> Result<(Box<dyn crate::AudioUnit>, crate::NeuralModelId)> {
        let neural_handle = self.engine.neural();
        let neural_system = neural_handle.inner().ok_or_else(|| {
            tutti_core::Error::InvalidConfig("Neural subsystem not enabled".into())
        })?;

        let builder = neural_system.register_effect("_closure_effect", self.infer_fn)?;

        let model_id = builder.model_id();
        let effect = builder.build_effect()?;

        Ok((effect, model_id))
    }
}
