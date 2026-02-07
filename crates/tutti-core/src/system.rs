//! Tutti system - unified audio engine with transport, metering, and DSP graph.

#[cfg(feature = "std")]
use crate::callback::AudioCallbackState;
use crate::compat::{Arc, Mutex};
#[cfg(feature = "std")]
use crate::compat::{String, Vec};
use crate::error::Result;
use crate::metering::MeteringManager;
use crate::net_frontend::TuttiNet;
use crate::pdc::PdcManager;
use crate::transport::{ClickState, TransportHandle, TransportManager};

#[cfg(feature = "std")]
use crate::output::AudioEngine;

#[cfg(feature = "midi")]
use crate::midi::MidiRoutingTable;

/// Complete audio system with DSP graph, transport, metering, PDC, and neural audio.
pub struct TuttiSystem {
    #[cfg(feature = "std")]
    engine: Mutex<AudioEngine>,
    net: Mutex<TuttiNet>,
    transport: Arc<TransportManager>,
    metering: Arc<MeteringManager>,
    click_state: Arc<ClickState>,
    pdc: Arc<PdcManager>,
    sample_rate: f64,
    #[cfg(not(feature = "std"))]
    channels: usize,

    /// MIDI routing table for channel/port/layer routing
    #[cfg(feature = "midi")]
    midi_routing: Mutex<MidiRoutingTable>,
}

impl TuttiSystem {
    /// Create a new Tutti system builder.
    pub fn builder() -> TuttiSystemBuilder {
        TuttiSystemBuilder::default()
    }

    /// Get sample rate.
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// Check if audio is running.
    #[cfg(feature = "std")]
    pub fn is_running(&self) -> bool {
        self.engine.lock().is_running()
    }

    /// List available output devices.
    #[cfg(feature = "std")]
    pub fn list_output_devices() -> Result<Vec<String>> {
        AudioEngine::list_devices()
    }

    /// Get the name of the current output device.
    #[cfg(feature = "std")]
    pub fn current_output_device_name(&self) -> Result<String> {
        self.engine.lock().device_name()
    }

    /// Set output device (requires restart to take effect).
    #[cfg(feature = "std")]
    pub fn set_output_device(&self, index: Option<usize>) {
        self.engine.lock().set_device(index);
    }

    /// Get number of output channels.
    pub fn channels(&self) -> usize {
        #[cfg(feature = "std")]
        {
            self.engine.lock().channels()
        }
        #[cfg(not(feature = "std"))]
        {
            self.channels
        }
    }

    /// Modify the DSP graph (non-realtime).
    ///
    /// The graph changes are automatically committed to the audio thread
    /// when the closure returns.
    ///
    /// # Example
    /// ```ignore
    /// system.graph(|net| {
    ///     let node = net.add(Box::new(sine_hz(440.0)));
    ///     net.pipe_output(node);
    ///     // Auto-committed here
    /// });
    /// ```
    pub fn graph<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut TuttiNet) -> R,
    {
        let mut net = self.net.lock();
        let result = f(&mut net);
        net.commit(); // Auto-commit to audio thread

        // Feed graph-level total latency into PdcManager for sampler butler
        let total_latency = net.total_latency();
        if total_latency > 0 {
            self.pdc.set_channel_latency(0, total_latency);
        }

        result
    }

    /// Get fluent transport API handle.
    ///
    /// # Example
    /// ```ignore
    /// system.transport()
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
        TransportHandle::new(self.transport.clone(), self.click_state.clone())
    }

    /// Get the transport manager (advanced use - prefer `transport()` for fluent API).
    pub fn transport_manager(&self) -> &Arc<TransportManager> {
        &self.transport
    }

    /// Get the metering manager.
    pub fn metering(&self) -> &Arc<MeteringManager> {
        &self.metering
    }

    /// Get the click state for creating a ClickNode.
    ///
    /// Prefer `transport().metronome()` for fluent configuration,
    /// or `transport().click_state()` for node creation.
    pub fn click_state(&self) -> &Arc<ClickState> {
        &self.click_state
    }

    /// Get the PDC manager.
    pub fn pdc(&self) -> &Arc<PdcManager> {
        &self.pdc
    }

    /// Clone the DSP graph for offline export.
    ///
    /// This creates a snapshot of the current audio graph that can be rendered
    /// offline without affecting the live audio engine.
    ///
    /// Used internally by the export system.
    pub fn clone_net(&self) -> fundsp::net::Net {
        self.net.lock().clone_net()
    }

    /// Create an export context for offline rendering.
    ///
    /// The context contains:
    /// - An isolated timeline that advances by sample count
    /// - A MIDI snapshot (if midi feature enabled)
    /// - Transport settings (tempo, loop range)
    ///
    /// Nodes configured for export will read from this context
    /// instead of the live transport.
    pub fn create_export_context(&self) -> crate::ExportContext {
        use crate::transport::ExportConfig;

        let transport_handle = TransportHandle::new(
            self.transport.clone(),
            self.click_state.clone(),
        );

        crate::ExportContext::new(ExportConfig {
            start_beat: 0.0,
            tempo: transport_handle.get_tempo(),
            sample_rate: self.sample_rate,
            loop_range: transport_handle.get_loop_range(),
        })
    }

    /// Configure MIDI routing.
    ///
    /// Use this to set up channel-based, port-based, or layered MIDI routing.
    /// All methods are chainable. Changes are automatically committed to the
    /// audio thread when the closure returns.
    ///
    /// # Example
    /// ```ignore
    /// // Channel-based routing (GM-style)
    /// system.midi_routing(|r| {
    ///     r.channel(0, lead_synth_id)
    ///      .channel(1, bass_synth_id)
    ///      .channel(9, drum_kit_id);
    /// });
    ///
    /// // Layering (same input to multiple synths)
    /// system.midi_routing(|r| {
    ///     r.channel_layer(0, &[strings_id, brass_id, choir_id]);
    /// });
    ///
    /// // Port-based routing (multiple keyboards)
    /// system.midi_routing(|r| {
    ///     r.port(0, main_synth_id)
    ///      .port(1, controller_synth_id);
    /// });
    /// ```
    #[cfg(feature = "midi")]
    pub fn midi_routing<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut MidiRoutingTable) -> R,
    {
        let mut routing = self.midi_routing.lock();
        let result = f(&mut routing);
        routing.commit(); // Auto-commit to audio thread
        result
    }

    /// Set a simple MIDI target (all MIDI → one synth).
    ///
    /// This is a convenience method for the common case of routing all hardware
    /// MIDI to a single synth. For more sophisticated routing, use `midi_routing()`.
    ///
    /// # Example
    /// ```ignore
    /// let synth_id = system.graph(|net| {
    ///     let id = net.add(Box::new(soundfont_unit));
    ///     net.pipe_output(id);
    ///     net.node(id).get_id()
    /// });
    ///
    /// system.set_midi_target(synth_id);
    /// ```
    #[cfg(feature = "midi")]
    pub fn set_midi_target(&self, unit_id: u64) {
        self.midi_routing(|r| {
            r.clear().fallback(unit_id);
        });
    }
}

/// Builder for TuttiSystem.
#[derive(Default)]
pub struct TuttiSystemBuilder {
    #[cfg(feature = "std")]
    device_index: Option<usize>,
    #[cfg_attr(feature = "std", allow(dead_code))]
    sample_rate: Option<f64>,
    inputs: usize,
    outputs: usize,

    /// MIDI input source for hardware MIDI routing (feature: midi)
    #[cfg(feature = "midi")]
    midi_input: Option<Arc<dyn crate::midi::MidiInputSource>>,
}

impl TuttiSystemBuilder {
    /// Set sample rate (only for no_std mode).
    #[cfg(not(feature = "std"))]
    pub fn sample_rate(mut self, rate: f64) -> Self {
        self.sample_rate = Some(rate);
        self
    }

    /// Set output device index (only for std mode with CPAL).
    #[cfg(feature = "std")]
    pub fn output_device(mut self, index: usize) -> Self {
        self.device_index = Some(index);
        self
    }

    /// Set number of inputs (default: 0).
    pub fn inputs(mut self, count: usize) -> Self {
        self.inputs = count;
        self
    }

    /// Set number of outputs (default: 2 for stereo).
    pub fn outputs(mut self, count: usize) -> Self {
        self.outputs = count;
        self
    }

    /// Set the MIDI input source for hardware MIDI routing.
    ///
    /// The input source provides MIDI events from hardware ports. These events
    /// are read in the audio callback and routed to registered audio nodes.
    ///
    /// # Example
    /// ```ignore
    /// let midi_system = MidiSystem::builder().io().build()?;
    /// let system = TuttiSystem::builder()
    ///     .midi_input(midi_system.port_manager())
    ///     .build()?;
    ///
    /// // Then configure routing:
    /// system.midi_routing(|r| {
    ///     r.add_channel_route(0, synth_id);
    /// });
    /// ```
    #[cfg(feature = "midi")]
    pub fn midi_input(mut self, input: Arc<dyn crate::midi::MidiInputSource>) -> Self {
        self.midi_input = Some(input);
        self
    }

    /// Build and start the audio system.
    pub fn build(self) -> Result<TuttiSystem> {
        #[cfg(feature = "std")]
        let (sample_rate, mut engine) = {
            let engine = AudioEngine::new(self.device_index)?;
            let sample_rate = engine.sample_rate();
            (sample_rate, engine)
        };

        #[cfg(not(feature = "std"))]
        let sample_rate = self.sample_rate.unwrap_or(44100.0);

        let inputs = self.inputs;
        let outputs = if self.outputs == 0 { 2 } else { self.outputs };

        let mut net = TuttiNet::new(inputs, outputs);
        let backend = net.backend();

        let transport = Arc::new(TransportManager::new(sample_rate));
        let metering = Arc::new(MeteringManager::new(sample_rate));
        let click_state = Arc::new(ClickState::new(
            transport.current_beat().clone(),
            transport.paused().clone(),
            transport.recording().clone(),
            transport.in_preroll().clone(),
        ));

        // Initialize PDC with outputs count (channels = outputs for now)
        let pdc = Arc::new(PdcManager::new(outputs, 0));

        // Create MIDI routing table
        #[cfg(feature = "midi")]
        let midi_routing = MidiRoutingTable::new();

        #[cfg(feature = "std")]
        {
            let mut callback_state =
                AudioCallbackState::new(transport.clone(), metering.clone(), sample_rate);

            callback_state.set_net_backend(backend);

            // Set up click node for metronome (mixed into output automatically)
            callback_state.set_click_node(click_state.clone(), sample_rate);

            // Wire up MIDI input routing if configured
            #[cfg(feature = "midi")]
            {
                if let Some(midi_input) = self.midi_input {
                    callback_state.set_midi_input(midi_input);
                }

                // Clone the MIDI registry from the net for the callback
                let midi_registry = net.midi_registry().clone();
                callback_state.set_midi_registry(midi_registry);

                // Share the routing snapshot with the callback
                callback_state.set_midi_routing(midi_routing.snapshot_arc());
            }

            engine.start(callback_state)?;
        }

        #[cfg(not(feature = "std"))]
        let _backend = backend; // Prevent unused variable warning

        Ok(TuttiSystem {
            #[cfg(feature = "std")]
            engine: Mutex::new(engine),
            net: Mutex::new(net),
            transport,
            metering,
            click_state,
            pdc,
            sample_rate,
            #[cfg(not(feature = "std"))]
            channels: outputs,
            #[cfg(feature = "midi")]
            midi_routing: Mutex::new(midi_routing),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_creation() {
        let system = TuttiSystem::builder().build();
        assert!(system.is_ok());

        let system = system.unwrap();
        assert!(system.sample_rate() > 0.0);
        assert!(system.is_running());
    }

    #[test]
    fn test_graph_closure() {
        let system = TuttiSystem::builder().build().unwrap();

        system.graph(|net| {
            use fundsp::prelude::*;
            let _node = net.add(sine_hz::<f32>(440.0));
        });
    }

    #[test]
    fn test_pdc_integration() {
        let system = TuttiSystem::builder().build().unwrap();

        // Test PDC is accessible
        assert_eq!(system.pdc().max_latency(), 0);
        assert!(system.pdc().is_enabled());

        // Set latency for channel 0
        system.pdc().set_channel_latency(0, 1024);
        assert_eq!(system.pdc().max_latency(), 1024);
        assert_eq!(system.pdc().get_channel_compensation(0), 0);

        // Set latency for channel 1 (higher)
        system.pdc().set_channel_latency(1, 2048);
        assert_eq!(system.pdc().max_latency(), 2048);

        // Channel 0 should now need 1024 samples of compensation
        assert_eq!(system.pdc().get_channel_compensation(0), 1024);
        assert_eq!(system.pdc().get_channel_compensation(1), 0);
    }
}
