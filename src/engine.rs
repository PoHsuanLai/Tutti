//! TuttiEngine that coordinates all audio subsystems

use crate::core::{
    MeteringManager, NodeId, NodeRegistry, PdcManager, TransportHandle, TransportManager, TuttiNet,
    TuttiSystem,
};
use crate::Result;
use tutti_core::Arc;

// Synth types (feature-gated)
#[cfg(all(feature = "synth", feature = "midi"))]
use tutti_synth::SynthHandle;

#[cfg(any(feature = "sampler", feature = "plugin", feature = "soundfont"))]
use std::path::{Path, PathBuf};

use tutti_core::compat::Mutex;

#[cfg(any(feature = "wav", feature = "flac", feature = "mp3", feature = "ogg"))]
use tutti_core::compat::HashMap;

#[cfg(any(feature = "wav", feature = "flac", feature = "mp3", feature = "ogg"))]
use tutti_core::Wave;
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

    /// Live analysis state + thread handle (opt-in via enable_live_analysis)
    #[cfg(feature = "analysis")]
    live_analysis: Mutex<Option<(Arc<crate::analysis::LiveAnalysisState>, JoinHandle<()>)>>,

    /// Cache for loaded audio samples (Wave)
    #[cfg(any(feature = "wav", feature = "flac", feature = "mp3", feature = "ogg"))]
    sample_cache: Mutex<HashMap<PathBuf, Arc<Wave>>>,
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

    /// Get the metering handle for fluent meter control.
    ///
    /// # Example
    /// ```ignore
    /// // Enable meters with chaining
    /// engine.metering().amp().lufs().correlation();
    ///
    /// // Read values
    /// let m = engine.metering();
    /// let (l_peak, r_peak, l_rms, r_rms) = m.amplitude();
    /// let lufs = m.loudness_global().unwrap_or(-70.0);
    /// let cpu = m.cpu_average();
    /// ```
    pub fn metering(&self) -> crate::core::MeteringHandle {
        crate::core::MeteringHandle::new(self.core.metering().clone())
    }

    /// Get direct access to the metering manager.
    ///
    /// Use `metering()` for the fluent API. This provides low-level access
    /// for advanced use cases like analysis taps.
    pub fn metering_manager(&self) -> &Arc<MeteringManager> {
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
    /// // Create nodes with parameters
    /// let lfo = engine.create("bass_lfo", &params! { "depth" => 0.8 })?;
    /// let env = engine.create("env", &params! { "gain" => 2.0 })?;
    /// let comp = engine.create("comp", &params! {})?;
    ///
    /// // Use in audio graph
    /// engine.graph(|net| {
    ///     chain!(net, lfo, env, comp => output);
    /// });
    /// ```
    pub fn dsp(&self) -> crate::dsp_nodes::DspHandle<'_> {
        crate::dsp_nodes::DspHandle::new(&self.registry, self.core.sample_rate())
    }

    /// Create a MIDI-responsive polyphonic synthesizer.
    ///
    /// Returns a fluent builder for configuring oscillator, filter, envelope,
    /// and voice settings. Call `.build()` to get a `PolySynth`, then add it
    /// to the graph. The synth responds to `note_on()`/`note_off()`.
    ///
    /// # Example
    /// ```ignore
    /// use tutti::prelude::*;
    ///
    /// let synth = engine.synth()
    ///     .saw()                         // oscillator type
    ///     .poly(8)                       // 8 voices
    ///     .filter_moog(2000.0, 0.7)      // Moog lowpass
    ///     .adsr(0.01, 0.2, 0.6, 0.3)     // envelope
    ///     .build()?;
    ///
    /// let synth_id = engine.graph(|net| net.add(synth).to_master());
    ///
    /// engine.note_on(synth_id, 0, 60, 100);  // Play middle C
    /// ```
    #[cfg(all(feature = "synth", feature = "midi"))]
    pub fn synth(&self) -> SynthHandle {
        let midi_registry = self.core.graph(|net| net.midi_registry().clone());
        SynthHandle::new(self.sample_rate(), midi_registry)
    }

    // =========================================================================
    // Fluent Builders - Resource Loading
    // =========================================================================

    /// Create a SoundFont synthesizer.
    ///
    /// Returns a fluent builder for configuring preset and channel. Call `.build()`
    /// to get a `SoundFontUnit` that can be added to the graph. The SoundFont file
    /// is loaded and cached internally.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let piano = engine.sf2("piano.sf2").preset(0).build()?;
    /// engine.graph(|net| net.add(piano).to_master());
    /// ```
    #[cfg(feature = "soundfont")]
    pub fn sf2(&self, path: impl AsRef<Path>) -> crate::builders::Sf2Builder<'_> {
        crate::builders::Sf2Builder::new(self, path.as_ref().to_path_buf())
    }

    /// Load a WAV audio sample.
    ///
    /// Returns a fluent builder for configuring gain, speed, and looping.
    /// Call `.build()` to get a `SamplerUnit`. The audio file is loaded and cached.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let kick = engine.wav("kick.wav").gain(0.8).build()?;
    /// engine.graph(|net| net.add(kick).to_master());
    /// ```
    #[cfg(all(feature = "sampler", feature = "wav"))]
    pub fn wav(&self, path: impl AsRef<Path>) -> crate::builders::WavBuilder<'_> {
        crate::builders::WavBuilder::new(self, path.as_ref().to_path_buf())
    }

    /// Load a FLAC audio sample.
    ///
    /// Returns a fluent builder for configuring gain, speed, and looping.
    /// Call `.build()` to get a `SamplerUnit`. The audio file is loaded and cached.
    #[cfg(all(feature = "sampler", feature = "flac"))]
    pub fn flac(&self, path: impl AsRef<Path>) -> crate::builders::FlacBuilder<'_> {
        crate::builders::FlacBuilder::new(self, path.as_ref().to_path_buf())
    }

    /// Load an MP3 audio sample.
    ///
    /// Returns a fluent builder for configuring gain, speed, and looping.
    /// Call `.build()` to get a `SamplerUnit`. The audio file is loaded and cached.
    #[cfg(all(feature = "sampler", feature = "mp3"))]
    pub fn mp3(&self, path: impl AsRef<Path>) -> crate::builders::Mp3Builder<'_> {
        crate::builders::Mp3Builder::new(self, path.as_ref().to_path_buf())
    }

    /// Load an OGG Vorbis audio sample.
    ///
    /// Returns a fluent builder for configuring gain, speed, and looping.
    /// Call `.build()` to get a `SamplerUnit`. The audio file is loaded and cached.
    #[cfg(all(feature = "sampler", feature = "ogg"))]
    pub fn ogg(&self, path: impl AsRef<Path>) -> crate::builders::OggBuilder<'_> {
        crate::builders::OggBuilder::new(self, path.as_ref().to_path_buf())
    }

    /// Load a VST3 plugin.
    ///
    /// Returns a fluent builder for configuring plugin parameters.
    /// Call `.build()` to get a boxed `AudioUnit`. Requires plugin runtime.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let reverb = engine.vst3("Reverb.vst3")
    ///     .param("room_size", 0.8)
    ///     .build()?;
    /// engine.graph(|net| net.add_boxed(reverb).to_master());
    /// ```
    #[cfg(feature = "plugin")]
    pub fn vst3(&self, path: impl AsRef<Path>) -> crate::builders::Vst3Builder<'_> {
        crate::builders::Vst3Builder::new(self, path.as_ref().to_path_buf())
    }

    /// Load a VST2 plugin.
    #[cfg(all(feature = "plugin", feature = "vst2"))]
    pub fn vst2(&self, path: impl AsRef<Path>) -> crate::builders::Vst2Builder<'_> {
        crate::builders::Vst2Builder::new(self, path.as_ref().to_path_buf())
    }

    /// Load a CLAP plugin.
    #[cfg(all(feature = "plugin", feature = "clap"))]
    pub fn clap(&self, path: impl AsRef<Path>) -> crate::builders::ClapBuilder<'_> {
        crate::builders::ClapBuilder::new(self, path.as_ref().to_path_buf())
    }

    /// Load a neural synth model (.mpk).
    ///
    /// Returns a fluent builder. Call `.build()` to get the voice unit and model ID.
    /// The model is loaded and cached.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let (violin, model_id) = engine.neural_synth("violin.mpk").build()?;
    /// engine.graph(|net| net.add_neural(violin, model_id).to_master());
    /// ```
    #[cfg(all(feature = "neural", feature = "midi"))]
    pub fn neural_synth(&self, path: impl AsRef<Path>) -> crate::builders::NeuralSynthBuilder<'_> {
        crate::builders::NeuralSynthBuilder::new(self, path.as_ref().to_path_buf())
    }

    /// Load a neural effect model (.mpk).
    ///
    /// Returns a fluent builder. Call `.build()` to get the effect unit and model ID.
    #[cfg(feature = "neural")]
    pub fn neural_effect(&self, path: impl AsRef<Path>) -> crate::builders::NeuralEffectBuilder<'_> {
        crate::builders::NeuralEffectBuilder::new(self, path.as_ref().to_path_buf())
    }

    /// Create a neural synth from a closure.
    ///
    /// The closure receives MIDI feature vector and returns control params.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let (synth, model_id) = engine.neural_synth_fn(|features| {
    ///     my_model.infer(features)
    /// }).build()?;
    /// engine.graph(|net| net.add_neural(synth, model_id).to_master());
    /// ```
    #[cfg(all(feature = "neural", feature = "midi"))]
    pub fn neural_synth_fn<F>(&self, infer_fn: F) -> crate::builders::NeuralSynthFnBuilder<'_, F>
    where
        F: Fn(&[f32]) -> Vec<f32> + Send + 'static,
    {
        crate::builders::NeuralSynthFnBuilder::new(self, infer_fn)
    }

    /// Create a neural effect from a closure.
    ///
    /// The closure receives audio samples and returns processed samples.
    #[cfg(feature = "neural")]
    pub fn neural_effect_fn<F>(&self, infer_fn: F) -> crate::builders::NeuralEffectFnBuilder<'_, F>
    where
        F: Fn(&[f32]) -> Vec<f32> + Send + 'static,
    {
        crate::builders::NeuralEffectFnBuilder::new(self, infer_fn)
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
        let guard = self.live_analysis.lock();
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
        let mut guard = self.live_analysis.lock();
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
        let entry = self.live_analysis.lock().take();
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
    /// let synth = engine.create("piano", &params! {})?;
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

    // =========================================================================
    // Internal helpers for fluent builders
    // =========================================================================

    /// Get direct access to the SoundFont system (for builders).
    #[cfg(feature = "soundfont")]
    pub(crate) fn soundfont_system(&self) -> &Arc<crate::synth::SoundFontSystem> {
        &self.soundfont
    }

    /// Get the plugin runtime handle (for builders).
    #[cfg(feature = "plugin")]
    pub(crate) fn plugin_runtime(&self) -> Option<&tokio::runtime::Handle> {
        self.plugin_runtime.as_ref()
    }

    /// Load a Wave from file, using cache for repeated loads.
    #[cfg(any(feature = "wav", feature = "flac", feature = "mp3", feature = "ogg"))]
    pub(crate) fn load_wave_cached(&self, path: &Path) -> Result<Arc<Wave>> {
        let path_buf = path.to_path_buf();

        // Check cache first
        {
            let cache = self.sample_cache.lock();
            if let Some(wave) = cache.get(&path_buf) {
                return Ok(wave.clone());
            }
        }

        // Load from disk
        let wave = Arc::new(Wave::load(path).map_err(|e| {
            crate::Error::InvalidConfig(format!("Failed to load audio file: {:?}", e))
        })?);

        // Store in cache
        {
            let mut cache = self.sample_cache.lock();
            cache.insert(path_buf, wave.clone());
        }

        Ok(wave)
    }

    /// Export audio from the current graph.
    ///
    /// Creates a snapshot of the current DSP graph and renders it offline.
    /// The export uses an isolated timeline so MIDI and automation can be
    /// properly rendered without interfering with live playback.
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
        let context = self.core.create_export_context();

        crate::export::ExportBuilder::new(net, sample_rate).with_context(context)
    }

    // =========================================================================
    // MIDI Event Helpers
    // =========================================================================

    /// Queue MIDI events to a specific audio node.
    ///
    /// Events are placed in the MidiRegistry and pulled by the node during
    /// audio processing (pull-based MIDI delivery).
    ///
    /// # Example
    /// ```ignore
    /// use tutti_midi_io::MidiEvent;
    /// let note_on = MidiEvent::note_on_builder(60, 100).build();
    /// engine.queue_midi_to_node(synth_id, &[note_on]);
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

    /// Send a Note On event to a node.
    ///
    /// Accepts either a raw MIDI note number (0-127) or a `Note` enum variant.
    ///
    /// # Example
    /// ```ignore
    /// use tutti::midi::Note;
    ///
    /// // Using Note enum (recommended)
    /// engine.note_on(synth_id, Note::C4, 100);
    ///
    /// // Using raw MIDI number
    /// engine.note_on(synth_id, 60u8, 100);
    /// ```
    #[cfg(feature = "midi")]
    pub fn note_on(&self, node: crate::core::NodeId, note: impl Into<u8>, velocity: u8) -> &Self {
        let event = crate::MidiEvent::note_on_builder(note.into(), velocity)
            .channel(0)
            .build();
        self.queue_midi_to_node(node, &[event]);
        self
    }

    /// Send a Note On event to a specific MIDI channel.
    #[cfg(feature = "midi")]
    pub fn note_on_ch(
        &self,
        node: crate::core::NodeId,
        channel: u8,
        note: impl Into<u8>,
        velocity: u8,
    ) -> &Self {
        let event = crate::MidiEvent::note_on_builder(note.into(), velocity)
            .channel(channel)
            .build();
        self.queue_midi_to_node(node, &[event]);
        self
    }

    /// Send a Note Off event to a node.
    #[cfg(feature = "midi")]
    pub fn note_off(&self, node: crate::core::NodeId, note: impl Into<u8>) -> &Self {
        let event = crate::MidiEvent::note_off_builder(note.into())
            .channel(0)
            .build();
        self.queue_midi_to_node(node, &[event]);
        self
    }

    /// Send a Note Off event to a specific MIDI channel.
    #[cfg(feature = "midi")]
    pub fn note_off_ch(
        &self,
        node: crate::core::NodeId,
        channel: u8,
        note: impl Into<u8>,
    ) -> &Self {
        let event = crate::MidiEvent::note_off_builder(note.into())
            .channel(channel)
            .build();
        self.queue_midi_to_node(node, &[event]);
        self
    }

    /// Send a Control Change event to a node.
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

    // =========================================================================
    // Custom DSP Registration (for user-defined nodes)
    // =========================================================================

    /// Register a parameterized node with clean API.
    ///
    /// The closure receives a `Params` wrapper with ergonomic getters.
    /// Returns any `AudioUnit` directly (auto-boxed, infallible).
    ///
    /// # Example
    /// ```ignore
    /// // Simple parameterized node
    /// engine.register("filter", |p| {
    ///     lowpass_hz::<f32>(p.get_or("cutoff", 1000.0))
    /// });
    ///
    /// // Multi-parameter node
    /// engine.register("synth", |p| {
    ///     let freq: f32 = p.get_or("freq", 440.0);
    ///     let detune: f32 = p.get_or("detune", 0.0);
    ///     sine_hz(freq + detune)
    /// });
    ///
    /// let filter = engine.create("filter", &params! { "cutoff" => 2000.0 })?;
    /// ```
    pub fn register<F, U>(&self, name: impl Into<String>, constructor: F) -> &Self
    where
        F: Fn(crate::core::Params<'_>) -> U + Send + Sync + 'static,
        U: crate::core::AudioUnit + 'static,
    {
        self.registry.register_simple(name, constructor);
        self
    }

    /// Register a static node (no parameters needed).
    ///
    /// # Example
    /// ```ignore
    /// engine.register_static("noise", || white());
    /// engine.register_static("a440", || sine_hz::<f32>(440.0));
    ///
    /// let noise = engine.create("noise", &params!{})?;
    /// ```
    pub fn register_static<F, U>(&self, name: impl Into<String>, constructor: F) -> &Self
    where
        F: Fn() -> U + Send + Sync + 'static,
        U: crate::core::AudioUnit + 'static,
    {
        self.registry.register_static(name, constructor);
        self
    }

    /// Register a node constructor (legacy verbose API).
    ///
    /// Prefer `register()` for cleaner syntax.
    pub fn register_raw<F>(&self, name: impl Into<String>, constructor: F) -> &Self
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
            #[cfg(feature = "soundfont")]
            soundfont,
            #[cfg(feature = "plugin")]
            plugin_runtime,
            #[cfg(feature = "analysis")]
            live_analysis: Mutex::new(None),
            #[cfg(any(feature = "wav", feature = "flac", feature = "mp3", feature = "ogg"))]
            sample_cache: Mutex::new(HashMap::new()),
        }
    }
}
