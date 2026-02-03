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
use crate::transport::{Metronome, TransportHandle, TransportManager};

#[cfg(feature = "std")]
use crate::output::AudioEngine;

#[cfg(feature = "neural")]
use crate::neural::NeuralNodeManager;

/// Complete audio system with DSP graph, transport, metering, PDC, and neural audio.
pub struct TuttiSystem {
    #[cfg(feature = "std")]
    engine: Mutex<AudioEngine>,
    net: Mutex<TuttiNet>,
    transport: Arc<TransportManager>,
    metering: Arc<MeteringManager>,
    metronome: Arc<Metronome>,
    pdc: Arc<PdcManager>,
    #[cfg(feature = "neural")]
    neural: Arc<NeuralNodeManager>,
    sample_rate: f64,
    #[cfg(not(feature = "std"))]
    channels: usize,
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
        TransportHandle::new(self.transport.clone(), self.metronome.clone())
    }

    /// Get the transport manager (advanced use - prefer `transport()` for fluent API).
    pub fn transport_manager(&self) -> &Arc<TransportManager> {
        &self.transport
    }

    /// Get the metering manager.
    pub fn metering(&self) -> &Arc<MeteringManager> {
        &self.metering
    }

    /// Get the metronome (advanced use - prefer `transport().metronome()` for fluent API).
    pub fn metronome(&self) -> &Arc<Metronome> {
        &self.metronome
    }

    /// Get the PDC manager.
    pub fn pdc(&self) -> &Arc<PdcManager> {
        &self.pdc
    }

    /// Get the neural node manager (neural feature only).
    ///
    /// The neural manager tracks which Net nodes are neural processors
    /// and enables GPU batching optimization via [`GraphAnalyzer`](crate::GraphAnalyzer).
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Register a neural synth
    /// let node_id = system.graph(|net| {
    ///     net.push(builder.build_voice()?)
    /// });
    ///
    /// system.neural().register(node_id, NeuralNodeInfo::synth(
    ///     builder.model_id(),
    ///     512,
    ///     44100.0
    /// ));
    /// ```
    #[cfg(feature = "neural")]
    pub fn neural(&self) -> &Arc<NeuralNodeManager> {
        &self.neural
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

        #[cfg(not(feature = "neural"))]
        let (net, backend) = TuttiNet::with_io(inputs, outputs);

        #[cfg(feature = "neural")]
        let (net, backend, neural) = TuttiNet::with_io(inputs, outputs);

        let transport = Arc::new(TransportManager::new(sample_rate));
        let metering = Arc::new(MeteringManager::new(sample_rate));
        let metronome = Arc::new(Metronome::new(sample_rate as f32));

        // Initialize PDC with outputs count (channels = outputs for now)
        let pdc = Arc::new(PdcManager::new(outputs, 0));

        #[cfg(feature = "std")]
        {
            let mut callback_state =
                AudioCallbackState::new(transport.clone(), metering.clone(), sample_rate);

            callback_state.set_net_backend(backend);

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
            metronome,
            pdc,
            #[cfg(feature = "neural")]
            neural,
            sample_rate,
            #[cfg(not(feature = "std"))]
            channels: outputs,
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
            use crate::compat::Box;
            use fundsp::prelude::*;
            let _node = net.add(Box::new(sine_hz::<f32>(440.0)));
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
