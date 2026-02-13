//! Top-level engine that coordinates all audio subsystems.

use crate::core::{
    MeteringManager, NodeId, NodeRegistry, PdcManager, TransportHandle, TransportManager, TuttiNet,
    TuttiSystem,
};
use crate::Result;
use tutti_core::Arc;

#[cfg(all(feature = "synth", feature = "midi"))]
use tutti_synth::SynthHandle;

#[cfg(any(
    feature = "sampler",
    feature = "plugin",
    feature = "soundfont",
    feature = "neural"
))]
use std::path::{Path, PathBuf};

use tutti_core::compat::Mutex;

#[cfg(any(feature = "wav", feature = "flac", feature = "mp3", feature = "ogg"))]
use tutti_core::compat::HashMap;

#[cfg(feature = "analysis")]
use std::thread::JoinHandle;
#[cfg(any(feature = "wav", feature = "flac", feature = "mp3", feature = "ogg"))]
use tutti_core::Wave;

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

/// Coordinates all audio subsystems based on enabled Cargo features:
/// MIDI (opt-in via `.midi()`), sampler, neural, soundfont (auto-initialized),
/// and plugin hosting (in-process, GUI editor support).
///
/// # Example
///
/// ```ignore
/// use tutti::prelude::*;
///
/// let engine = TuttiEngine::builder()
///     .sample_rate(44100.0)
///     .build()?;
///
/// engine.graph_mut(|net| {
///     let osc = net.add(Box::new(sine_hz(440.0)));
///     net.pipe_output(osc);
/// });
///
/// engine.transport().play();
/// ```
pub struct TuttiEngine {
    core: TuttiSystem,
    registry: NodeRegistry,

    #[cfg(feature = "midi")]
    midi: Option<Arc<MidiSystem>>,

    #[cfg(feature = "sampler")]
    sampler: Arc<SamplerSystem>,

    #[cfg(feature = "neural")]
    neural: Arc<NeuralSystem>,

    #[cfg(feature = "soundfont")]
    soundfont: Arc<crate::synth::SoundFontSystem>,

    /// Control handles for loaded plugins (editor, state, params).
    #[cfg(feature = "plugin")]
    plugin_control_handles: Mutex<Vec<tutti_plugin::PluginHandle>>,

    /// Keeps in-process plugin threads alive for the lifetime of the engine.
    #[cfg(feature = "plugin")]
    inprocess_handles: Mutex<Vec<tutti_plugin::InProcessThreadHandle>>,

    /// Opt-in via enable_live_analysis; stopped via disable_live_analysis.
    #[cfg(feature = "analysis")]
    live_analysis: Mutex<Option<(Arc<crate::analysis::LiveAnalysisState>, JoinHandle<()>)>>,

    #[cfg(any(feature = "wav", feature = "flac", feature = "mp3", feature = "ogg"))]
    sample_cache: Mutex<HashMap<PathBuf, Arc<Wave>>>,
}

impl TuttiEngine {
    pub fn builder() -> crate::TuttiEngineBuilder {
        crate::TuttiEngineBuilder::default()
    }

    pub fn sample_rate(&self) -> f64 {
        self.core.sample_rate()
    }

    #[cfg(feature = "std")]
    pub fn is_running(&self) -> bool {
        self.core.is_running()
    }

    #[cfg(feature = "std")]
    pub fn list_output_devices() -> Result<Vec<String>> {
        Ok(TuttiSystem::list_output_devices()?)
    }

    #[cfg(feature = "std")]
    pub fn current_output_device_name(&self) -> Result<String> {
        Ok(self.core.current_output_device_name()?)
    }

    #[cfg(feature = "std")]
    pub fn set_output_device(&self, index: Option<usize>) -> &Self {
        self.core.set_output_device(index);
        self
    }

    pub fn channels(&self) -> usize {
        self.core.channels()
    }

    /// Access the DSP graph for reading, querying, or side-effects.
    ///
    /// Does **not** commit changes to the audio thread. Use for
    /// operations that don't change graph structure (querying nodes,
    /// queueing MIDI events, cloning data for export).
    ///
    /// For structural changes, use [`graph_mut`].
    pub fn graph<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut TuttiNet) -> R,
    {
        self.core.graph(f)
    }

    /// Modify the DSP graph and auto-commit to the audio thread.
    ///
    /// Use for structural changes: adding/removing nodes,
    /// connecting/disconnecting, resetting the graph.
    ///
    /// # Example
    /// ```ignore
    /// engine.graph_mut(|net| {
    ///     let node = net.add(Box::new(sine_hz(440.0)));
    ///     net.pipe_output(node);
    /// });
    /// ```
    pub fn graph_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut TuttiNet) -> R,
    {
        self.core.graph_mut(f)
    }

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

    /// Low-level access for advanced use cases like analysis taps.
    /// Prefer `metering()` for the fluent API.
    pub fn metering_manager(&self) -> &Arc<MeteringManager> {
        self.core.metering()
    }

    pub fn pdc(&self) -> &Arc<PdcManager> {
        self.core.pdc()
    }

    /// Nodes are registered once and can be instantiated multiple
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
    /// engine.graph_mut(|net| {
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
    /// let synth_id = engine.graph_mut(|net| net.add(synth).master());
    ///
    /// engine.note_on(synth_id, 0, 60, 100);  // Play middle C
    /// ```
    #[cfg(all(feature = "synth", feature = "midi"))]
    pub fn synth(&self) -> SynthHandle {
        let midi_registry = self.core.graph(|net| net.midi_registry().clone());
        SynthHandle::new(self.sample_rate(), midi_registry)
    }

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
    /// engine.graph_mut(|net| net.add(piano).master());
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
    /// engine.graph_mut(|net| net.add(kick).master());
    /// ```
    #[cfg(all(feature = "sampler", feature = "wav"))]
    pub fn wav(&self, path: impl AsRef<Path>) -> crate::builders::SampleBuilder<'_> {
        crate::builders::SampleBuilder::new(self, path.as_ref().to_path_buf())
    }

    /// Load a FLAC audio sample. See [`wav()`](Self::wav) for builder API.
    #[cfg(all(feature = "sampler", feature = "flac"))]
    pub fn flac(&self, path: impl AsRef<Path>) -> crate::builders::SampleBuilder<'_> {
        crate::builders::SampleBuilder::new(self, path.as_ref().to_path_buf())
    }

    /// Load an MP3 audio sample. See [`wav()`](Self::wav) for builder API.
    #[cfg(all(feature = "sampler", feature = "mp3"))]
    pub fn mp3(&self, path: impl AsRef<Path>) -> crate::builders::SampleBuilder<'_> {
        crate::builders::SampleBuilder::new(self, path.as_ref().to_path_buf())
    }

    /// Load an OGG Vorbis audio sample. See [`wav()`](Self::wav) for builder API.
    #[cfg(all(feature = "sampler", feature = "ogg"))]
    pub fn ogg(&self, path: impl AsRef<Path>) -> crate::builders::SampleBuilder<'_> {
        crate::builders::SampleBuilder::new(self, path.as_ref().to_path_buf())
    }

    /// Load a VST3 plugin.
    ///
    /// Returns a fluent builder for configuring plugin parameters.
    /// Call `.build()` to get a boxed `AudioUnit`. Requires plugin runtime.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let (reverb, _handle) = engine.vst3("Reverb.vst3")
    ///     .param("room_size", 0.8)
    ///     .build()?;
    /// engine.graph_mut(|net| net.add_boxed(reverb).master());
    /// ```
    #[cfg(all(feature = "plugin", feature = "vst3"))]
    pub fn vst3(&self, path: impl AsRef<Path>) -> crate::builders::PluginBuilder<'_> {
        crate::builders::PluginBuilder::new(self, path.as_ref().to_path_buf())
    }

    /// Load a VST2 plugin.
    #[cfg(all(feature = "plugin", feature = "vst2"))]
    pub fn vst2(&self, path: impl AsRef<Path>) -> crate::builders::PluginBuilder<'_> {
        crate::builders::PluginBuilder::new(self, path.as_ref().to_path_buf())
    }

    /// Load a CLAP plugin.
    #[cfg(all(feature = "plugin", feature = "clap"))]
    pub fn clap(&self, path: impl AsRef<Path>) -> crate::builders::PluginBuilder<'_> {
        crate::builders::PluginBuilder::new(self, path.as_ref().to_path_buf())
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
    /// engine.graph_mut(|net| net.add_neural(violin, model_id).master());
    /// ```
    #[cfg(all(feature = "neural", feature = "midi"))]
    pub fn neural_synth(&self, path: impl AsRef<Path>) -> crate::builders::NeuralSynthBuilder<'_> {
        crate::builders::NeuralSynthBuilder::new(self, path.as_ref().to_path_buf())
    }

    /// Load a neural effect model (.mpk).
    ///
    /// Returns a fluent builder. Call `.build()` to get the effect unit and model ID.
    #[cfg(feature = "neural")]
    pub fn neural_effect(
        &self,
        path: impl AsRef<Path>,
    ) -> crate::builders::NeuralEffectBuilder<'_> {
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
    /// engine.graph_mut(|net| net.add_neural(synth, model_id).master());
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

    /// Transient detection, pitch detection, stereo correlation, waveform thumbnails.
    /// If live analysis is enabled, the handle includes live state.
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

    /// Methods are no-ops when MIDI hardware is not connected.
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
    /// engine.graph_mut(|net| {
    ///     net.pipe_output(synth);
    /// });
    ///
    /// // Route all hardware MIDI to the synth
    /// engine.set_midi_target(synth);
    /// ```
    #[cfg(feature = "midi")]
    pub fn set_midi_target(&self, node: NodeId) -> &Self {
        let unit_id = self.graph(|net| net.node(node).get_id());
        self.core.set_midi_target(unit_id);
        self
    }

    /// Events are delivered to the node before the next audio callback.
    #[cfg(feature = "midi")]
    pub fn queue_midi(&self, node: NodeId, events: &[MidiEvent]) -> &Self {
        self.graph(|net| {
            net.queue_midi(node, events);
        });
        self
    }

    /// Automatically initialized when the engine is built.
    #[cfg(feature = "sampler")]
    pub fn sampler(&self) -> SamplerHandle {
        SamplerHandle::new(Some(self.sampler.clone()))
    }

    /// Automatically initialized when the engine is built.
    #[cfg(feature = "neural")]
    pub fn neural(&self) -> NeuralHandle {
        NeuralHandle::new(Some(self.neural.clone()))
    }

    /// Create an automation lane wired to this engine's transport.
    #[cfg(feature = "automation")]
    pub fn automation_lane<T: Clone + Send + 'static>(
        &self,
        envelope: crate::AutomationEnvelope<T>,
    ) -> crate::LiveAutomationLane<T> {
        crate::AutomationLane::new(envelope, self.transport())
    }

    /// Get direct access to the SoundFont system (for builders).
    #[cfg(feature = "soundfont")]
    pub(crate) fn soundfont_system(&self) -> &Arc<crate::synth::SoundFontSystem> {
        &self.soundfont
    }

    /// Store an in-process plugin thread handle to keep it alive.
    #[cfg(feature = "plugin")]
    pub(crate) fn store_inprocess_handle(
        &self,
        handle: tutti_plugin::InProcessThreadHandle,
        control_handle: tutti_plugin::PluginHandle,
    ) {
        self.inprocess_handles.lock().push(handle);
        self.plugin_control_handles.lock().push(control_handle);
    }

    /// Index corresponds to the order in which plugins were loaded.
    #[cfg(feature = "plugin")]
    pub fn plugin(&self, index: usize) -> Option<tutti_plugin::PluginHandle> {
        self.plugin_control_handles.lock().get(index).cloned()
    }

    #[cfg(feature = "plugin")]
    pub fn plugin_count(&self) -> usize {
        self.plugin_control_handles.lock().len()
    }

    /// Decode and cache audio data. Repeated loads return the cached `Arc<Wave>`
    /// without re-decoding. Same cache used by `wav()`, `mp3()`, etc.
    #[cfg(any(feature = "wav", feature = "flac", feature = "mp3", feature = "ogg"))]
    pub fn load_wave(&self, path: impl AsRef<Path>) -> Result<Arc<Wave>> {
        self.get_wave_cached(path.as_ref())
    }

    /// Start a non-blocking background wave import, returning a handle to poll progress.
    ///
    /// The import runs on a dedicated thread. Poll [`ImportHandle::progress()`]
    /// each frame to get status updates, or call [`ImportHandle::wait()`] to block.
    ///
    /// The loaded wave is automatically cached on completion.
    ///
    /// ```ignore
    /// let mut import = engine.start_load_wave("song.mp3");
    ///
    /// loop {
    ///     match import.progress() {
    ///         ImportStatus::Running(p) => println!("Loading: {:.0}%", p * 100.0),
    ///         ImportStatus::Complete(wave) => break,
    ///         ImportStatus::Failed(e) => { eprintln!("{}", e); break; }
    ///         ImportStatus::Pending => {}
    ///     }
    /// }
    /// ```
    #[cfg(all(
        feature = "sampler",
        any(feature = "wav", feature = "flac", feature = "mp3", feature = "ogg")
    ))]
    pub fn start_load_wave(&self, path: impl AsRef<Path>) -> crate::ImportHandle {
        let path_buf = path.as_ref().to_path_buf();

        // Return immediately if cached.
        {
            let cache = self.sample_cache.lock();
            if let Some(wave) = cache.get(&path_buf) {
                return crate::ImportHandle::from_cached(wave.clone());
            }
        }

        crate::ImportHandle::start(path_buf)
    }

    /// Return a cached Wave or error if not yet loaded. Never blocks on disk I/O.
    #[cfg(any(feature = "wav", feature = "flac", feature = "mp3", feature = "ogg"))]
    pub(crate) fn get_wave_cached(&self, path: &Path) -> Result<Arc<Wave>> {
        let cache = self.sample_cache.lock();
        cache.get(path).cloned().ok_or_else(|| {
            crate::Error::Core(tutti_core::Error::InvalidConfig(format!(
                "Wave not loaded yet: {}",
                path.display()
            )))
        })
    }

    /// Store a wave in the cache so subsequent `load_wave()` calls
    /// return the cached copy.
    #[cfg(any(feature = "wav", feature = "flac", feature = "mp3", feature = "ogg"))]
    pub fn cache_wave(&self, path: impl AsRef<Path>, wave: Arc<Wave>) {
        let mut cache = self.sample_cache.lock();
        cache.insert(path.as_ref().to_path_buf(), wave);
    }

    /// Compute the end beat of all scheduled content in the graph.
    ///
    /// Iterates all nodes and finds the latest `start_beat + duration_beats`
    /// across all sampler clips. Returns 0.0 if no timed content exists.
    ///
    /// # Example
    /// ```ignore
    /// let end = engine.content_end_beat();
    /// println!("Song ends at beat {}", end);
    /// ```
    #[cfg(feature = "sampler")]
    pub fn content_end_beat(&self) -> f64 {
        self.graph(|net| {
            let all_ids: Vec<_> = net.inner_ref().ids().copied().collect();
            all_ids
                .into_iter()
                .filter_map(|node_id| {
                    net.node_ref_typed::<crate::sampler::SamplerUnit>(node_id)
                        .map(|sampler| {
                            if sampler.duration_beats() > 0.0 {
                                sampler.start_beat() + sampler.duration_beats()
                            } else {
                                let tempo = self.transport().get_tempo() as f64;
                                let beats = sampler.duration_seconds() * tempo / 60.0;
                                sampler.start_beat() + beats
                            }
                        })
                })
                .fold(0.0f64, f64::max)
        })
    }

    /// Compute the total duration of all content in seconds.
    ///
    /// Finds the latest end beat across all clips and converts to seconds
    /// using the current tempo. Returns 0.0 if no content exists.
    ///
    /// # Example
    /// ```ignore
    /// let secs = engine.content_duration();
    /// engine.export()
    ///     .duration_seconds(secs + 2.0)  // +2s for reverb tails
    ///     .to_file("output.wav")?;
    /// ```
    #[cfg(feature = "sampler")]
    pub fn content_duration(&self) -> f64 {
        let end_beat = self.content_end_beat();
        let tempo = self.transport().get_tempo() as f64;
        if tempo > 0.0 {
            end_beat * 60.0 / tempo
        } else {
            0.0
        }
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
        let mut net = self.core.clone_net();
        let sample_rate = self.core.sample_rate();
        let mut context = self.core.create_export_context();

        // Transfer pending MIDI events from the live registry into the export snapshot.
        // Events are placed at beat 0.0 (start of export) since they were queued
        // as immediate events without beat positions.
        #[cfg(feature = "midi")]
        {
            self.core.graph(|tutti_net| {
                tutti_net
                    .midi_registry()
                    .drain_into_snapshot(context.midi_snapshot_mut(), 0.0);
            });

            // Create a MidiSnapshotReader and inject it into all MIDI-consuming
            // nodes in the cloned net. Each node gets its own clone so cursors
            // are independent.
            let reader = tutti_core::midi::MidiSnapshotReader::new(
                context.midi_snapshot.clone(),
                context.timeline.clone(),
            );

            Self::inject_midi_sources(&mut net, &reader);
        }

        // Replace transport on timeline-aware sampler nodes with export timeline.
        // Only affects samplers that already have a transport (start_beat was set).
        // Samplers without transport use self-advancing playback and don't need injection.
        #[cfg(feature = "sampler")]
        {
            use tutti_core::AudioUnit;

            let timeline: std::sync::Arc<dyn tutti_core::TransportReader> =
                context.timeline.clone();
            let node_ids: Vec<_> = net.ids().copied().collect();
            for node_id in node_ids {
                if let Some(sampler) = <dyn AudioUnit>::as_any_mut(net.node_mut(node_id))
                    .downcast_mut::<crate::sampler::SamplerUnit>()
                {
                    if sampler.has_transport() {
                        sampler.replace_transport(timeline.clone());
                    }
                }
            }
        }

        crate::export::ExportBuilder::new(net, sample_rate).with_context(context)
    }

    /// Inject a `MidiSnapshotReader` into all MIDI-consuming nodes in a cloned net.
    ///
    /// Iterates all nodes, attempts to downcast to known MIDI-consuming types
    /// (PolySynth, SoundFontUnit, NeuralSynthNode), and sets the MIDI source.
    #[cfg(all(feature = "export", feature = "midi"))]
    fn inject_midi_sources(
        net: &mut tutti_core::dsp::Net,
        reader: &tutti_core::midi::MidiSnapshotReader,
    ) {
        use tutti_core::AudioUnit;

        let node_ids: Vec<_> = net.ids().copied().collect();
        for node_id in node_ids {
            let unit = net.node_mut(node_id);

            // Try PolySynth
            #[cfg(all(feature = "synth", feature = "midi"))]
            if let Some(synth) =
                <dyn AudioUnit>::as_any_mut(unit).downcast_mut::<tutti_synth::PolySynth>()
            {
                synth.set_midi_source(Box::new(reader.clone()));
                continue;
            }

            // Try SoundFontUnit
            #[cfg(feature = "soundfont")]
            if let Some(sf_unit) =
                <dyn AudioUnit>::as_any_mut(unit).downcast_mut::<tutti_synth::SoundFontUnit>()
            {
                sf_unit.set_midi_source(Box::new(reader.clone()));
                continue;
            }

            // Try NeuralSynthNode
            #[cfg(all(feature = "neural", feature = "midi"))]
            if let Some(neural_synth) =
                <dyn AudioUnit>::as_any_mut(unit).downcast_mut::<tutti_neural::NeuralSynthNode>()
            {
                neural_synth.set_midi_source(Box::new(reader.clone()));
                continue;
            }
        }
    }

    /// Accepts either a raw MIDI note number (0-127) or a `Note` enum variant.
    ///
    /// # Example
    /// ```ignore
    /// use tutti::midi::Note;
    /// engine.note_on(synth_id, Note::C4, 100);
    /// engine.note_on(synth_id, 60u8, 100);
    /// ```
    #[cfg(feature = "midi")]
    pub fn note_on(&self, node: crate::core::NodeId, note: impl Into<u8>, velocity: u8) -> &Self {
        let event = crate::MidiEvent::note_on_builder(note.into(), velocity)
            .channel(0)
            .build();
        self.queue_midi(node, &[event])
    }

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
        self.queue_midi(node, &[event])
    }

    #[cfg(feature = "midi")]
    pub fn note_off(&self, node: crate::core::NodeId, note: impl Into<u8>) -> &Self {
        let event = crate::MidiEvent::note_off_builder(note.into())
            .channel(0)
            .build();
        self.queue_midi(node, &[event])
    }

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
        self.queue_midi(node, &[event])
    }

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
        self.queue_midi(node, &[event])
    }

    /// `value` is 14-bit unsigned: 0-16383, where 8192 = center (no bend).
    #[cfg(feature = "midi")]
    pub fn pitch_bend(&self, node: crate::core::NodeId, channel: u8, value: u16) -> &Self {
        let event = crate::MidiEvent::bend_builder(value.min(16383))
            .channel(channel)
            .build();
        self.queue_midi(node, &[event])
    }

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

    pub(crate) fn from_parts(
        core: TuttiSystem,
        #[cfg(feature = "midi")] midi: Option<Arc<MidiSystem>>,
        #[cfg(feature = "sampler")] sampler: Arc<SamplerSystem>,
        #[cfg(feature = "neural")] neural: Arc<NeuralSystem>,
        #[cfg(feature = "soundfont")] soundfont: Arc<crate::synth::SoundFontSystem>,
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
            plugin_control_handles: Mutex::new(Vec::new()),
            #[cfg(feature = "plugin")]
            inprocess_handles: Mutex::new(Vec::new()),
            #[cfg(feature = "analysis")]
            live_analysis: Mutex::new(None),
            #[cfg(any(feature = "wav", feature = "flac", feature = "mp3", feature = "ogg"))]
            sample_cache: Mutex::new(HashMap::new()),
        }
    }
}
