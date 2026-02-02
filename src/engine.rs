//! TuttiEngine that coordinates all audio subsystems

use crate::core::{
    MeteringManager, NodeId, NodeRegistry, PdcManager, TransportHandle, TransportManager, TuttiNet,
    TuttiSystem,
};
use crate::Result;
use std::path::Path;
use std::sync::Arc;

#[cfg(feature = "midi")]
use crate::midi::{MidiHandle, MidiSystem};
#[cfg(feature = "midi")]
use crate::MidiEvent;

#[cfg(feature = "sampler")]
use crate::sampler::{SamplerHandle, SamplerSystem};

#[cfg(feature = "neural")]
use crate::neural::{NeuralHandle, NeuralSystem};

/// Main audio engine that coordinates all subsystems.
///
/// TuttiEngine wraps tutti-core's TuttiSystem and integrates subsystems based on
/// enabled Cargo features:
/// - MIDI subsystem (feature "midi") - requires `.midi()` to connect hardware
/// - Sampler subsystem (feature "sampler") - automatically initialized
/// - Neural subsystem (feature "neural") - automatically initialized
/// - SoundFont support (feature "soundfont") - automatically initialized
/// - Plugin hosting (feature "plugin") - requires tokio runtime handle
///
/// # Example
///
/// ```ignore
/// use tutti::prelude::*;
///
/// // Enable features in Cargo.toml:
/// // tutti = { version = "...", features = ["sampler", "neural"] }
///
/// let engine = TuttiEngine::builder()
///     .sample_rate(44100.0)
///     .build()?;
///
/// // Subsystems are ready to use
/// let sampler = engine.sampler();
/// let neural = engine.neural();
///
/// engine.graph(|net| {
///     let osc = net.add(Box::new(sine_hz(440.0)));
///     net.pipe_output(osc);
/// });
///
/// engine.transport().play();
/// ```
pub struct TuttiEngine {
    /// Core audio system (always present)
    core: TuttiSystem,

    /// Node registry for dynamic node creation
    registry: NodeRegistry,

    /// MIDI subsystem (optional)
    #[cfg(feature = "midi")]
    midi: Option<Arc<MidiSystem>>,

    /// Sampler subsystem (feature-gated)
    #[cfg(feature = "sampler")]
    sampler: Arc<SamplerSystem>,

    /// Neural subsystem (feature-gated)
    #[cfg(feature = "neural")]
    neural: Arc<NeuralSystem>,

    /// SoundFont manager (feature-gated)
    #[cfg(feature = "soundfont")]
    soundfont: Arc<crate::synth::SoundFontSystem>,

    /// Plugin runtime handle for async plugin loading (optional)
    #[cfg(feature = "plugin")]
    plugin_runtime: Option<tokio::runtime::Handle>,
}

impl TuttiEngine {
    /// Create a new engine builder
    pub fn builder() -> crate::TuttiEngineBuilder {
        crate::TuttiEngineBuilder::default()
    }

    /// Get sample rate
    pub fn sample_rate(&self) -> f64 {
        self.core.sample_rate()
    }

    /// Check if audio is running
    #[cfg(feature = "std")]
    pub fn is_running(&self) -> bool {
        self.core.is_running()
    }

    /// List available output devices
    #[cfg(feature = "std")]
    pub fn list_output_devices() -> Result<Vec<String>> {
        TuttiSystem::list_output_devices()
    }

    /// Get current output device name
    #[cfg(feature = "std")]
    pub fn current_output_device_name(&self) -> Result<String> {
        self.core.current_output_device_name()
    }

    /// Set output device
    #[cfg(feature = "std")]
    pub fn set_output_device(&self, index: Option<usize>) {
        self.core.set_output_device(index);
    }

    /// Get number of output channels
    pub fn channels(&self) -> usize {
        self.core.channels()
    }

    /// Modify the DSP graph
    ///
    /// # Example
    /// ```ignore
    /// engine.graph(|net| {
    ///     let node = net.add(Box::new(sine_hz(440.0)));
    ///     net.pipe_output(node);
    /// });
    /// ```
    pub fn graph<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut TuttiNet) -> R,
    {
        self.core.graph(f)
    }

    /// Create an instance of a loaded node and add it to the graph
    ///
    /// This combines node creation from the registry with adding to the graph.
    ///
    /// # Example
    /// ```ignore
    /// // Load once
    /// engine.load_mpk("synth", "model.mpk")?;
    ///
    /// // Instantiate multiple times
    /// let synth1 = engine.instance("synth", &params! {})?;
    /// let synth2 = engine.instance("synth", &params! { "pitch" => 2.0 })?;
    ///
    /// // Use in graph
    /// engine.graph(|net| {
    ///     chain!(net, synth1, synth2 => output);
    /// });
    /// ```
    pub fn instance(&self, name: &str, params: &crate::core::NodeParams) -> Result<NodeId> {
        let node = self.registry.create(name, params).map_err(|e| {
            crate::Error::InvalidConfig(format!("Failed to create instance '{}': {:?}", name, e))
        })?;

        let node_id = self.core.graph(|net| net.add(node));

        Ok(node_id)
    }

    /// Get fluent transport API handle.
    ///
    /// # Example
    /// ```ignore
    /// engine.transport()
    ///     .tempo(128.0)
    ///     .loop_range(0.0, 16.0)
    ///     .enable_loop()
    ///     .metronome()
    ///         .volume(0.7)
    ///         .accent_every(4)
    ///         .always()
    ///     .play();
    /// ```
    pub fn transport(&self) -> TransportHandle {
        self.core.transport()
    }

    /// Get the transport manager (advanced use - prefer `transport()` for fluent API).
    pub fn transport_manager(&self) -> &Arc<TransportManager> {
        self.core.transport_manager()
    }

    /// Get the metering manager
    pub fn metering(&self) -> &Arc<MeteringManager> {
        self.core.metering()
    }

    /// Get the PDC manager
    pub fn pdc(&self) -> &Arc<PdcManager> {
        self.core.pdc()
    }

    /// Get the DSP node builder handle.
    ///
    /// Provides methods for registering DSP nodes (LFO, dynamics, spatial, etc.)
    /// via fluent API. Nodes are registered once and can be instantiated multiple
    /// times with different parameters.
    ///
    /// # Example
    /// ```ignore
    /// use tutti::prelude::*;
    /// use tutti::dsp_nodes::{LfoShape, ChannelLayout};
    ///
    /// // Register DSP node types
    /// engine.dsp()
    ///     .lfo("bass_lfo", LfoShape::Sine, 0.5)
    ///     .envelope("env", 0.001, 0.1)
    ///     .sidechain().compressor("comp", -20.0, 4.0, 0.001, 0.05);
    ///
    /// // Instantiate with parameters
    /// let lfo = engine.instance("bass_lfo", &params! { "depth" => 0.8 })?;
    /// let env = engine.instance("env", &params! { "gain" => 2.0 })?;
    /// let comp = engine.instance("comp", &params! {})?;
    ///
    /// // Use in audio graph
    /// engine.graph(|net| {
    ///     chain!(net, lfo, env, comp => output);
    /// });
    /// ```
    pub fn dsp(&self) -> crate::dsp_nodes::DspHandle<'_> {
        crate::dsp_nodes::DspHandle::new(&self.registry, self.core.sample_rate())
    }

    /// Get the audio analysis subsystem handle.
    ///
    /// Provides tools for audio analysis: transient detection, pitch detection,
    /// stereo correlation, and waveform thumbnails.
    ///
    /// # Example
    /// ```ignore
    /// use tutti::prelude::*;
    ///
    /// let engine = TuttiEngine::builder().build()?;
    /// let analysis = engine.analysis();
    ///
    /// // Detect transients (kick drum hits, etc.)
    /// let transients = analysis.detect_transients(&drum_samples);
    /// for t in transients {
    ///     println!("Transient at {}s: strength {}", t.time_seconds, t.strength);
    /// }
    ///
    /// // Detect pitch
    /// let pitch = analysis.detect_pitch(&vocal_samples);
    /// if pitch.confidence > 0.7 {
    ///     println!("Detected: {} Hz (MIDI {})", pitch.frequency, pitch.midi_note);
    /// }
    ///
    /// // Generate waveform summary for UI
    /// let waveform = analysis.waveform_summary(&samples, 512);
    /// ```
    #[cfg(feature = "analysis")]
    pub fn analysis(&self) -> crate::analysis::AnalysisHandle {
        crate::analysis::AnalysisHandle::new(self.core.sample_rate())
    }

    /// Get the MIDI subsystem handle.
    ///
    /// Returns a handle that works whether or not MIDI is enabled.
    /// Methods are no-ops when MIDI is disabled.
    ///
    /// # Example
    /// ```ignore
    /// // Always works, even if MIDI not enabled
    /// engine.midi().send().note_on(0, 60, 100);
    /// ```
    #[cfg(feature = "midi")]
    pub fn midi(&self) -> MidiHandle {
        MidiHandle::new(self.midi.clone())
    }

    /// Queue MIDI events to a specific node
    ///
    /// Events are queued and will be delivered to the node before the next audio callback.
    ///
    /// # Arguments
    /// * `node` - The node ID to send MIDI to
    /// * `events` - Slice of MIDI events to queue
    #[cfg(feature = "midi")]
    pub fn queue_midi(&self, node: NodeId, events: &[MidiEvent]) {
        self.graph(|net| {
            net.queue_midi(node, events);
        })
    }

    /// Get the sampler subsystem handle.
    ///
    /// Available when the "sampler" feature is enabled in Cargo.toml.
    /// The subsystem is automatically initialized when the engine is built.
    ///
    /// # Example
    /// ```ignore
    /// let sampler = engine.sampler();
    /// sampler.stream("file.wav").gain(0.8).start();
    /// ```
    #[cfg(feature = "sampler")]
    pub fn sampler(&self) -> SamplerHandle {
        SamplerHandle::new(Some(self.sampler.clone()))
    }

    /// Get the neural subsystem handle.
    ///
    /// Available when the "neural" feature is enabled in Cargo.toml.
    /// The subsystem is automatically initialized when the engine is built.
    ///
    /// # Example
    /// ```ignore
    /// let neural = engine.neural();
    /// neural.load_synth("model.mpk")?;
    /// ```
    #[cfg(feature = "neural")]
    pub fn neural(&self) -> NeuralHandle {
        NeuralHandle::new(Some(self.neural.clone()))
    }

    /// Export audio from the current graph.
    ///
    /// Creates a snapshot of the current DSP graph and renders it offline.
    ///
    /// # Example
    /// ```ignore
    /// // Export 10 seconds to WAV
    /// engine.export()
    ///     .duration_seconds(10.0)
    ///     .to_file("output.wav")?;
    ///
    /// // Export with options
    /// engine.export()
    ///     .duration_beats(16.0, 120.0)  // 16 beats at 120 BPM
    ///     .format(AudioFormat::Flac)
    ///     .normalize(NormalizationMode::Lufs(-14.0))
    ///     .to_file("output.flac")?;
    /// ```
    #[cfg(feature = "export")]
    pub fn export(&self) -> crate::export::ExportBuilder {
        let net = self.core.clone_net();
        let sample_rate = self.core.sample_rate();
        crate::export::ExportBuilder::new(net, sample_rate)
    }

    // ===== Neural model loading =====

    /// Load a neural model from Burn's native .mpk format
    ///
    /// # Example
    /// ```ignore
    /// engine.load_mpk("my_synth", "model.mpk")?;
    /// ```
    #[cfg(feature = "neural")]
    pub fn load_mpk(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<()> {
        crate::neural::register_neural_model(&self.registry, &self.neural, name, path)
            .map_err(|e| crate::Error::InvalidConfig(format!("Failed to load .mpk model: {:?}", e)))
    }

    /// Load a neural model from ONNX format (requires conversion to Burn format first)
    ///
    /// Note: ONNX models must be pre-converted using the burn-import tool.
    ///
    /// # Example
    /// ```ignore
    /// engine.load_onnx("my_synth", "model.onnx")?;
    /// ```
    #[cfg(feature = "neural")]
    pub fn load_onnx(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<()> {
        crate::neural::register_neural_model(&self.registry, &self.neural, name, path).map_err(
            |e| crate::Error::InvalidConfig(format!("Failed to load .onnx model: {:?}", e)),
        )
    }

    // ===== Plugin loading =====

    /// Load a VST3 plugin
    ///
    /// Requires a tokio runtime handle to be set via `.plugin_runtime()` in the builder.
    ///
    /// # Example
    /// ```ignore
    /// let runtime = tokio::runtime::Runtime::new()?;
    /// let engine = TuttiEngine::builder()
    ///     .plugin_runtime(runtime.handle().clone())
    ///     .build()?;
    ///
    /// engine.load_vst3("reverb", "/Library/Audio/Plug-Ins/VST3/Reverb.vst3")?;
    ///
    /// // Later, instantiate:
    /// let reverb = engine.instance("reverb", &params! {})?;
    /// ```
    #[cfg(feature = "plugin")]
    pub fn load_vst3(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<()> {
        let runtime = self.plugin_runtime.as_ref().ok_or_else(|| {
            crate::Error::InvalidConfig(
                "Plugin runtime not set. Use .plugin_runtime() in builder.".into(),
            )
        })?;

        crate::plugin::register_plugin(&self.registry, runtime, name, path)
            .map_err(|e| crate::Error::InvalidConfig(format!("Failed to load VST3: {:?}", e)))?;

        Ok(())
    }

    /// Load a VST2 plugin
    ///
    /// Requires a tokio runtime handle to be set via `.plugin_runtime()` in the builder.
    ///
    /// # Example
    /// ```ignore
    /// let runtime = tokio::runtime::Runtime::new()?;
    /// let engine = TuttiEngine::builder()
    ///     .plugin_runtime(runtime.handle().clone())
    ///     .build()?;
    ///
    /// engine.load_vst2("synth", "/Library/Audio/Plug-Ins/VST/Synth.vst")?;
    ///
    /// // Later, instantiate:
    /// let synth = engine.instance("synth", &params! {})?;
    /// ```
    #[cfg(all(feature = "plugin", feature = "vst2"))]
    pub fn load_vst2(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<()> {
        let runtime = self.plugin_runtime.as_ref().ok_or_else(|| {
            crate::Error::InvalidConfig(
                "Plugin runtime not set. Use .plugin_runtime() in builder.".into(),
            )
        })?;

        crate::plugin::register_plugin(&self.registry, runtime, name, path)
            .map_err(|e| crate::Error::InvalidConfig(format!("Failed to load VST2: {:?}", e)))?;

        Ok(())
    }

    /// Load a CLAP plugin
    ///
    /// Requires a tokio runtime handle to be set via `.plugin_runtime()` in the builder.
    ///
    /// # Example
    /// ```ignore
    /// let runtime = tokio::runtime::Runtime::new()?;
    /// let engine = TuttiEngine::builder()
    ///     .plugin_runtime(runtime.handle().clone())
    ///     .build()?;
    ///
    /// engine.load_clap("synth", "/Library/Audio/Plug-Ins/CLAP/Synth.clap")?;
    ///
    /// // Later, instantiate:
    /// let synth = engine.instance("synth", &params! {})?;
    /// ```
    #[cfg(all(feature = "plugin", feature = "clap"))]
    pub fn load_clap(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<()> {
        let runtime = self.plugin_runtime.as_ref().ok_or_else(|| {
            crate::Error::InvalidConfig(
                "Plugin runtime not set. Use .plugin_runtime() in builder.".into(),
            )
        })?;

        crate::plugin::register_plugin(&self.registry, runtime, name, path)
            .map_err(|e| crate::Error::InvalidConfig(format!("Failed to load CLAP: {:?}", e)))?;

        Ok(())
    }

    // ===== Sample loading =====

    /// Load a WAV audio sample
    ///
    /// Loads the entire file into memory using fundsp's Wave loader.
    /// For large files, consider using StreamingSamplerUnit with the Butler thread.
    ///
    /// # Example
    /// ```ignore
    /// engine.load_wav("kick", "kick.wav")?;
    /// let kick = engine.instance("kick", &params! {})?;
    /// ```
    #[cfg(feature = "sampler")]
    pub fn load_wav(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<()> {
        self.load_sample(name, path)
    }

    /// Load a FLAC audio sample
    ///
    /// Loads the entire file into memory using fundsp's Wave loader.
    /// For large files, consider using StreamingSamplerUnit with the Butler thread.
    ///
    /// # Example
    /// ```ignore
    /// engine.load_flac("snare", "snare.flac")?;
    /// let snare = engine.instance("snare", &params! {})?;
    /// ```
    #[cfg(feature = "sampler")]
    pub fn load_flac(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<()> {
        self.load_sample(name, path)
    }

    /// Load an MP3 audio sample
    ///
    /// Loads the entire file into memory using fundsp's Wave loader.
    /// For large files, consider using StreamingSamplerUnit with the Butler thread.
    ///
    /// # Example
    /// ```ignore
    /// engine.load_mp3("music", "music.mp3")?;
    /// let music = engine.instance("music", &params! {})?;
    /// ```
    #[cfg(feature = "sampler")]
    pub fn load_mp3(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<()> {
        self.load_sample(name, path)
    }

    /// Load an OGG Vorbis audio sample
    ///
    /// Loads the entire file into memory using fundsp's Wave loader.
    /// For large files, consider using StreamingSamplerUnit with the Butler thread.
    ///
    /// # Example
    /// ```ignore
    /// engine.load_ogg("vocal", "vocal.ogg")?;
    /// let vocal = engine.instance("vocal", &params! {})?;
    /// ```
    #[cfg(feature = "sampler")]
    pub fn load_ogg(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<()> {
        self.load_sample(name, path)
    }

    /// Load a SoundFont (.sf2) file
    ///
    /// Registers the SoundFont in the node registry. You can then instantiate
    /// multiple synth instances with different presets.
    ///
    /// # Instance Parameters
    /// - `preset` (i32) - Preset number (0-127, default: 0)
    /// - `channel` (i32) - MIDI channel (0-15, default: 0)
    ///
    /// # Example
    /// ```ignore
    /// engine.load_sf2("piano", "piano.sf2")?;
    ///
    /// // Instantiate with default preset (0)
    /// let piano = engine.instance("piano", &params! {})?;
    ///
    /// // Instantiate with specific preset
    /// let piano_bright = engine.instance("piano", &params! { "preset" => 1 })?;
    /// ```
    #[cfg(feature = "soundfont")]
    pub fn load_sf2(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<()> {
        let name = name.into();
        let path_buf = path.as_ref().to_path_buf();

        // Load the SoundFont file
        let handle = self.soundfont.load(&path_buf).map_err(|e| {
            crate::Error::InvalidConfig(format!("Failed to load SoundFont: {:?}", e))
        })?;

        // Clone Arc to move into closure
        let manager_clone = self.soundfont.clone();
        let sample_rate = self.sample_rate();

        // Register in node registry
        self.registry.register(&name, move |params| {
            let soundfont = manager_clone.get(&handle).ok_or_else(|| {
                crate::core::NodeRegistryError::ConstructionFailed(
                    "SoundFont not found in manager".to_string(),
                )
            })?;

            let settings =
                crate::synth::SoundFontSystem::new(sample_rate as u32).default_settings();
            let mut unit = crate::synth::SoundFontUnit::new(soundfont, &settings);

            // Apply instance parameters
            if let Some(preset) = params.get("preset").and_then(|v| v.as_i64()) {
                let channel = params.get("channel").and_then(|v| v.as_i64()).unwrap_or(0);
                unit.program_change(channel as i32, preset as i32);
            }

            Ok(Box::new(unit) as Box<dyn crate::core::AudioUnit>)
        });

        Ok(())
    }

    /// Internal helper for loading audio samples
    #[cfg(feature = "sampler")]
    fn load_sample(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<()> {
        use crate::dsp::Wave;

        let name = name.into();
        let path_buf = path.as_ref().to_path_buf();

        self.registry.register(name, move |_params| {
            // Load audio file using fundsp's Wave (uses symphonia, supports many formats)
            let wave = Wave::load(&path_buf).map_err(|e| {
                crate::core::NodeRegistryError::AudioFileLoadError(format!("{:?}", e))
            })?;

            // Wrap in SamplerUnit for in-memory playback
            Ok(Box::new(crate::sampler::SamplerUnit::new(Arc::new(wave))))
        });

        Ok(())
    }

    /// Register a custom node constructor
    ///
    /// # Example
    /// ```ignore
    /// engine.add_node("my_filter", |params| {
    ///     let cutoff = params.get("cutoff")?.as_f32().unwrap_or(1000.0);
    ///     Ok(Box::new(lowpass_hz(cutoff)))
    /// })?;
    /// ```
    pub fn add_node<F>(&self, name: impl Into<String>, constructor: F)
    where
        F: Fn(
                &crate::core::NodeParams,
            ) -> std::result::Result<
                Box<dyn crate::core::AudioUnit>,
                crate::core::NodeRegistryError,
            > + Send
            + Sync
            + 'static,
    {
        self.registry.register(name, constructor);
    }

    /// Internal: create engine from builder
    pub(crate) fn from_parts(
        core: TuttiSystem,
        #[cfg(feature = "midi")] midi: Option<Arc<MidiSystem>>,
        #[cfg(feature = "sampler")] sampler: Arc<SamplerSystem>,
        #[cfg(feature = "neural")] neural: Arc<NeuralSystem>,
        #[cfg(feature = "soundfont")] soundfont: Arc<crate::synth::SoundFontSystem>,
        #[cfg(feature = "plugin")] plugin_runtime: Option<tokio::runtime::Handle>,
    ) -> Self {
        Self {
            core,
            registry: NodeRegistry::default(),
            #[cfg(feature = "midi")]
            midi,
            #[cfg(feature = "sampler")]
            sampler,
            #[cfg(feature = "neural")]
            neural,
            #[cfg(feature = "soundfont")]
            soundfont,
            #[cfg(feature = "plugin")]
            plugin_runtime,
        }
    }
}
