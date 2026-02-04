//! TuttiEngine that coordinates all audio subsystems

use crate::core::{
    MeteringManager, NodeId, NodeRegistry, PdcManager, TransportHandle, TransportManager, TuttiNet,
    TuttiSystem,
};
use crate::Result;
use std::path::Path;
use std::sync::Arc;

#[cfg(feature = "analysis")]
use std::sync::Mutex;
#[cfg(feature = "analysis")]
use std::thread::JoinHandle;

#[cfg(feature = "midi")]
use crate::core::MidiRoutingTable;
#[cfg(feature = "midi")]
use crate::midi::{MidiHandle, MidiSystem};
#[cfg(feature = "midi")]
use crate::MidiEvent;

#[cfg(feature = "sampler")]
use crate::sampler::{SamplerHandle, SamplerSystem};

#[cfg(feature = "neural")]
use crate::neural::{NeuralHandle, NeuralSystem};

#[cfg(feature = "neural")]
use crate::core::NeuralModelId;

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

    /// Side-map: registry name → NeuralModelId for neural-aware instantiation.
    /// When `instance()` finds a name here, it uses `net.add_neural()` instead of `net.add()`.
    #[cfg(feature = "neural")]
    neural_models: std::sync::Mutex<std::collections::HashMap<String, NeuralModelId>>,

    /// SoundFont manager (feature-gated)
    #[cfg(feature = "soundfont")]
    soundfont: Arc<crate::synth::SoundFontSystem>,

    /// Plugin runtime handle for async plugin loading (optional)
    #[cfg(feature = "plugin")]
    plugin_runtime: Option<tokio::runtime::Handle>,

    /// Live analysis state + thread handle (opt-in via enable_live_analysis)
    #[cfg(feature = "analysis")]
    live_analysis: Mutex<Option<(Arc<crate::analysis::LiveAnalysisState>, JoinHandle<()>)>>,
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
    pub fn set_output_device(&self, index: Option<usize>) -> &Self {
        self.core.set_output_device(index);
        self
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

    /// Create an instance of a loaded node and add it to the graph.
    ///
    /// For neural models (loaded via `load_synth_mpk`/`load_effect_mpk`),
    /// the node is automatically registered with `NeuralNodeManager` for
    /// GPU batching and the batching strategy is forwarded.
    ///
    /// # Example
    /// ```ignore
    /// // Load once
    /// engine.load_synth_mpk("violin", "violin.mpk")?;
    ///
    /// // Instantiate multiple times
    /// let v1 = engine.instance("violin", &params! {})?;
    /// let v2 = engine.instance("violin", &params! {})?;
    ///
    /// // Use in graph
    /// engine.graph(|net| {
    ///     net.pipe_output(v1);
    ///     net.pipe_output(v2);
    /// });
    /// ```
    pub fn instance(&self, name: &str, params: &crate::core::NodeParams) -> Result<NodeId> {
        let node = self.registry.create(name, params).map_err(|e| {
            crate::Error::InvalidConfig(format!("Failed to create instance '{}': {:?}", name, e))
        })?;

        #[cfg(feature = "neural")]
        {
            let model_id = self.neural_models.lock().unwrap().get(name).copied();
            if let Some(mid) = model_id {
                let node_id = self.core.graph(|net| net.add_neural(node, mid));
                self.forward_neural_strategy();
                return Ok(node_id);
            }
        }

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
    ///     println!("Transient at {}s: strength {}", t.time, t.strength);
    /// }
    ///
    /// // Detect pitch
    /// let pitch = analysis.detect_pitch(&vocal_samples);
    /// if pitch.confidence > 0.7 {
    ///     println!("Detected: {} Hz (MIDI {:?})", pitch.frequency, pitch.midi_note);
    /// }
    ///
    /// // Generate waveform summary for UI
    /// let waveform = analysis.waveform_summary(&samples, 512);
    /// ```
    #[cfg(feature = "analysis")]
    pub fn analysis(&self) -> crate::analysis::AnalysisHandle {
        let guard = self.live_analysis.lock().unwrap();
        match &*guard {
            Some((state, _)) => {
                crate::analysis::AnalysisHandle::with_live(self.core.sample_rate(), state.clone())
            }
            None => crate::analysis::AnalysisHandle::new(self.core.sample_rate()),
        }
    }

    /// Enable live analysis of the running audio graph.
    ///
    /// Spawns a background thread that reads from a ring buffer tap in the
    /// audio callback and runs pitch detection, transient detection, and
    /// waveform analysis. Results are accessible via `analysis().live_*()`.
    ///
    /// This is opt-in — call this after building the engine to start live analysis.
    /// Call `disable_live_analysis()` to stop.
    #[cfg(feature = "analysis")]
    pub fn enable_live_analysis(&self) -> &Self {
        let mut guard = self.live_analysis.lock().unwrap();
        if guard.is_some() {
            return self; // Already enabled
        }

        let consumer = self.core.metering().enable_analysis_tap();
        let state = Arc::new(crate::analysis::LiveAnalysisState::new(512));
        let state2 = state.clone();
        let sample_rate = self.core.sample_rate();

        let handle = std::thread::Builder::new()
            .name("tutti-live-analysis".into())
            .spawn(move || {
                crate::analysis::live::run_analysis_thread(consumer, state2, sample_rate);
            })
            .expect("failed to spawn analysis thread");

        *guard = Some((state, handle));
        self
    }

    /// Disable live analysis and stop the background thread.
    #[cfg(feature = "analysis")]
    pub fn disable_live_analysis(&self) -> &Self {
        let entry = self.live_analysis.lock().unwrap().take();
        if let Some((state, handle)) = entry {
            state.stop();
            self.core.metering().disable_analysis_tap();
            let _ = handle.join();
        }
        self
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

    /// Configure MIDI routing from hardware inputs to audio nodes.
    ///
    /// Supports channel-based, port-based, and layered routing. All methods
    /// are chainable. Changes are automatically committed to the audio thread
    /// when the closure returns.
    ///
    /// # Example
    /// ```ignore
    /// // Channel-based routing (GM-style)
    /// engine.midi_routing(|r| {
    ///     r.channel(0, lead_synth_id)
    ///      .channel(1, bass_synth_id)
    ///      .channel(9, drum_kit_id);
    /// });
    ///
    /// // Layering (same input to multiple synths)
    /// engine.midi_routing(|r| {
    ///     r.channel_layer(0, &[strings_id, brass_id, choir_id]);
    /// });
    ///
    /// // Port-based routing (multiple MIDI keyboards)
    /// engine.midi_routing(|r| {
    ///     r.port(0, main_synth_id)
    ///      .port(1, controller_synth_id);
    /// });
    ///
    /// // Combined port + channel routing
    /// engine.midi_routing(|r| {
    ///     r.port_channel(0, 0, piano_id)   // Port 0, Ch 0 → piano
    ///      .port_channel(0, 1, bass_id)    // Port 0, Ch 1 → bass
    ///      .port_channel(1, 9, drums_id);  // Port 1, Ch 10 → drums
    /// });
    /// ```
    #[cfg(feature = "midi")]
    pub fn midi_routing<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut MidiRoutingTable) -> R,
    {
        self.core.midi_routing(f)
    }

    /// Set a simple MIDI target (all hardware MIDI → one synth).
    ///
    /// This is a convenience method for the common case of routing all hardware
    /// MIDI to a single synth. For more sophisticated routing, use `midi_routing()`.
    ///
    /// # Example
    /// ```ignore
    /// let synth = engine.instance("piano", &params! {})?;
    /// engine.graph(|net| {
    ///     net.pipe_output(synth);
    /// });
    ///
    /// // Route all hardware MIDI to the synth
    /// engine.set_midi_target(synth);
    /// ```
    #[cfg(feature = "midi")]
    pub fn set_midi_target(&self, node: NodeId) -> &Self {
        // Get the unit ID from the node
        let unit_id = self.graph(|net| net.node(node).get_id());
        self.core.set_midi_target(unit_id);
        self
    }

    /// Queue MIDI events to a specific node
    ///
    /// Events are queued and will be delivered to the node before the next audio callback.
    ///
    /// # Arguments
    /// * `node` - The node ID to send MIDI to
    /// * `events` - Slice of MIDI events to queue
    #[cfg(feature = "midi")]
    pub fn queue_midi(&self, node: NodeId, events: &[MidiEvent]) -> &Self {
        self.graph(|net| {
            net.queue_midi(node, events);
        });
        self
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
    ///     .normalize(NormalizationMode::lufs(-14.0))
    ///     .to_file("output.flac")?;
    /// ```
    #[cfg(feature = "export")]
    pub fn export(&self) -> crate::export::ExportBuilder {
        let net = self.core.clone_net();
        let sample_rate = self.core.sample_rate();
        crate::export::ExportBuilder::new(net, sample_rate)
    }

    /// Load a neural synth model (.mpk) and register it for instantiation.
    ///
    /// The model is loaded once and registered in the node registry. Use
    /// `instance()` to create multiple voice instances — each shares the
    /// same GPU model for batched inference.
    ///
    /// # Example
    /// ```ignore
    /// engine.load_synth_mpk("violin", "violin.mpk")?;
    /// let v1 = engine.instance("violin", &params!{})?;
    /// let v2 = engine.instance("violin", &params!{})?; // same model = batched
    /// ```
    #[cfg(feature = "neural")]
    pub fn load_synth_mpk(
        &self,
        name: impl Into<String>,
        path: impl AsRef<std::path::Path>,
    ) -> Result<&Self> {
        let name = name.into();
        let builder = self
            .neural()
            .load_synth(
                path.as_ref().to_str().ok_or_else(|| {
                    crate::Error::InvalidConfig("Invalid UTF-8 in path".to_string())
                })?,
            )
            .map_err(|e| {
                crate::Error::InvalidConfig(format!("Failed to load synth model: {}", e))
            })?;

        // Store name → model_id for neural-aware instantiation
        let model_id = builder.model_id();
        self.neural_models
            .lock()
            .unwrap()
            .insert(name.clone(), model_id);

        // Register factory in the node registry
        self.registry.register(name, move |_params| {
            builder.build_voice().map_err(|e| {
                crate::core::NodeRegistryError::Neural(format!(
                    "Failed to build neural voice: {}",
                    e
                ))
            })
        });

        Ok(self)
    }

    /// Load a neural effect model (.mpk) and register it for instantiation.
    ///
    /// The model is loaded once and registered in the node registry. Use
    /// `instance()` to create effect instances — each shares the same GPU
    /// model for batched inference.
    ///
    /// # Example
    /// ```ignore
    /// engine.load_effect_mpk("amp", "amp_sim.mpk")?;
    /// let fx = engine.instance("amp", &params!{})?;
    /// ```
    #[cfg(feature = "neural")]
    pub fn load_effect_mpk(
        &self,
        name: impl Into<String>,
        path: impl AsRef<std::path::Path>,
    ) -> Result<&Self> {
        let name = name.into();
        let builder = self
            .neural()
            .load_effect(
                path.as_ref().to_str().ok_or_else(|| {
                    crate::Error::InvalidConfig("Invalid UTF-8 in path".to_string())
                })?,
            )
            .map_err(|e| {
                crate::Error::InvalidConfig(format!("Failed to load effect model: {}", e))
            })?;

        // Store name → model_id for neural-aware instantiation
        let model_id = builder.model_id();
        self.neural_models
            .lock()
            .unwrap()
            .insert(name.clone(), model_id);

        // Register factory in the node registry
        self.registry.register(name, move |_params| {
            builder.build_effect().map_err(|e| {
                crate::core::NodeRegistryError::Neural(format!(
                    "Failed to build neural effect: {}",
                    e
                ))
            })
        });

        Ok(self)
    }

    /// Forward the batching strategy from the graph to the neural inference engine.
    ///
    /// Called after adding neural nodes. The strategy is recomputed on `commit()`
    /// (which happens inside `graph()`), so we read and forward it here.
    #[cfg(feature = "neural")]
    fn forward_neural_strategy(&self) {
        let strategy = self.core.graph(|net| net.batching_strategy().cloned());
        if let Some(s) = strategy {
            self.neural.update_strategy(s);
        }
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
    pub fn load_vst3(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<&Self> {
        let runtime = self.plugin_runtime.as_ref().ok_or_else(|| {
            crate::Error::InvalidConfig(
                "Plugin runtime not set. Use .plugin_runtime() in builder.".into(),
            )
        })?;

        crate::plugin::register_plugin(&self.registry, runtime, name, path)
            .map_err(|e| crate::Error::InvalidConfig(format!("Failed to load VST3: {:?}", e)))?;

        Ok(self)
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
    pub fn load_vst2(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<&Self> {
        let runtime = self.plugin_runtime.as_ref().ok_or_else(|| {
            crate::Error::InvalidConfig(
                "Plugin runtime not set. Use .plugin_runtime() in builder.".into(),
            )
        })?;

        crate::plugin::register_plugin(&self.registry, runtime, name, path)
            .map_err(|e| crate::Error::InvalidConfig(format!("Failed to load VST2: {:?}", e)))?;

        Ok(self)
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
    pub fn load_clap(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<&Self> {
        let runtime = self.plugin_runtime.as_ref().ok_or_else(|| {
            crate::Error::InvalidConfig(
                "Plugin runtime not set. Use .plugin_runtime() in builder.".into(),
            )
        })?;

        crate::plugin::register_plugin(&self.registry, runtime, name, path)
            .map_err(|e| crate::Error::InvalidConfig(format!("Failed to load CLAP: {:?}", e)))?;

        Ok(self)
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
    pub fn load_wav(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<&Self> {
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
    pub fn load_flac(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<&Self> {
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
    pub fn load_mp3(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<&Self> {
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
    pub fn load_ogg(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<&Self> {
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
    pub fn load_sf2(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<&Self> {
        let name = name.into();
        let path_buf = path.as_ref().to_path_buf();

        // Load the SoundFont file
        let handle = self.soundfont.load(&path_buf).map_err(|e| {
            crate::Error::InvalidConfig(format!("Failed to load SoundFont: {:?}", e))
        })?;

        // Clone Arc to move into closure
        let manager_clone = self.soundfont.clone();
        let sample_rate = self.sample_rate();

        // Get MIDI registry from core
        #[cfg(feature = "midi")]
        let midi_registry = {
            let mut reg = None;
            self.core.graph(|net| {
                reg = Some(net.midi_registry().clone());
            });
            reg.unwrap()
        };

        // Register in node registry
        self.registry.register(&name, move |params| {
            let soundfont = manager_clone.get(&handle).ok_or_else(|| {
                crate::core::NodeRegistryError::ConstructionFailed(
                    "SoundFont not found in manager".to_string(),
                )
            })?;

            let settings =
                crate::synth::SoundFontSystem::new(sample_rate as u32).default_settings();

            #[cfg(feature = "midi")]
            let mut unit =
                crate::synth::SoundFontUnit::with_midi(soundfont, &settings, midi_registry.clone());

            #[cfg(not(feature = "midi"))]
            let mut unit = crate::synth::SoundFontUnit::new(soundfont, &settings);

            // Apply instance parameters
            if let Some(preset) = params.get("preset").and_then(|v| v.as_i64()) {
                let channel = params.get("channel").and_then(|v| v.as_i64()).unwrap_or(0);
                unit.program_change(channel as i32, preset as i32);
            }

            Ok(Box::new(unit) as Box<dyn crate::core::AudioUnit>)
        });

        Ok(self)
    }

    /// Queue MIDI events to a specific audio node
    ///
    /// This allows programmatic MIDI triggering for internal audio nodes
    /// that implement MidiAudioUnit (e.g., SoundFontUnit, PolySynth).
    ///
    /// # Example
    /// ```ignore
    /// let synth = engine.instance("piano", &params! {})?;
    ///
    /// // Trigger notes programmatically
    /// use tutti_midi_io::MidiEvent;
    /// let note_on = MidiEvent::note_on_builder(60, 100).build();
    /// engine.queue_midi_to_node(synth, &[note_on]);
    /// ```
    #[cfg(feature = "midi")]
    pub fn queue_midi_to_node(
        &self,
        node: crate::core::NodeId,
        events: &[crate::MidiEvent],
    ) -> &Self {
        self.core.graph(|net| {
            net.queue_midi(node, events);
        });
        self
    }

    /// Send a Note On event to a node
    ///
    /// Convenience method for triggering notes on MIDI-aware audio nodes.
    ///
    /// # Example
    /// ```ignore
    /// let synth = engine.instance("piano", &params! {})?;
    /// engine.note_on(synth, 0, 60, 100);  // Channel 0, Middle C, velocity 100
    /// ```
    #[cfg(feature = "midi")]
    pub fn note_on(&self, node: crate::core::NodeId, channel: u8, note: u8, velocity: u8) -> &Self {
        let event = crate::MidiEvent::note_on_builder(note, velocity)
            .channel(channel)
            .build();
        self.queue_midi_to_node(node, &[event]);
        self
    }

    /// Send a Note Off event to a node
    ///
    /// # Example
    /// ```ignore
    /// engine.note_off(synth, 0, 60);  // Channel 0, Middle C
    /// ```
    #[cfg(feature = "midi")]
    pub fn note_off(&self, node: crate::core::NodeId, channel: u8, note: u8) -> &Self {
        let event = crate::MidiEvent::note_off_builder(note)
            .channel(channel)
            .build();
        self.queue_midi_to_node(node, &[event]);
        self
    }

    /// Send a Control Change event to a node
    ///
    /// # Example
    /// ```ignore
    /// engine.control_change(synth, 0, 7, 100);  // Channel 0, Volume CC, max value
    /// ```
    #[cfg(feature = "midi")]
    pub fn control_change(
        &self,
        node: crate::core::NodeId,
        channel: u8,
        cc: u8,
        value: u8,
    ) -> &Self {
        let event = crate::MidiEvent::cc_builder(cc, value)
            .channel(channel)
            .build();
        self.queue_midi_to_node(node, &[event]);
        self
    }

    /// Internal helper for loading audio samples
    #[cfg(feature = "sampler")]
    fn load_sample(&self, name: impl Into<String>, path: impl AsRef<Path>) -> Result<&Self> {
        use crate::dsp::Wave;

        let name = name.into();
        let path_buf = path.as_ref().to_path_buf();

        self.registry.register(name, move |_params| {
            // Load audio file using fundsp's Wave (uses symphonia, supports many formats)
            let wave = Wave::load(&path_buf)
                .map_err(|e| crate::core::NodeRegistryError::AudioFile(format!("{:?}", e)))?;

            // Wrap in SamplerUnit for in-memory playback
            Ok(Box::new(crate::sampler::SamplerUnit::new(Arc::new(wave))))
        });

        Ok(self)
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
    pub fn add_node<F>(&self, name: impl Into<String>, constructor: F) -> &Self
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
        self
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
            #[cfg(feature = "neural")]
            neural_models: std::sync::Mutex::new(std::collections::HashMap::new()),
            #[cfg(feature = "soundfont")]
            soundfont,
            #[cfg(feature = "plugin")]
            plugin_runtime,
            #[cfg(feature = "analysis")]
            live_analysis: Mutex::new(None),
        }
    }
}
