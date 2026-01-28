//! Tutti system - unified audio engine with transport, metering, and DSP graph.

use crate::callback::AudioCallbackState;
use crate::error::Result;
use crate::metering::MeteringManager;
use crate::net_frontend::TuttiNet;
use crate::output::{AudioEngine, AudioEngineConfig};
use crate::pdc::PdcManager;
use crate::transport::{Metronome, TransportManager};
use std::sync::{Arc, Mutex};

#[cfg(feature = "neural")]
use crate::neural::NeuralNodeManager;

/// Complete audio system with DSP graph, transport, metering, PDC, and neural audio.
pub struct TuttiSystem {
    engine: Mutex<AudioEngine>,
    net: Mutex<TuttiNet>,
    transport: Arc<TransportManager>,
    metering: Arc<MeteringManager>,
    metronome: Arc<Metronome>,
    pdc: Arc<PdcManager>,
    #[cfg(feature = "neural")]
    neural: Arc<NeuralNodeManager>,
    sample_rate: f64,
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
    pub fn is_running(&self) -> bool {
        self.engine.lock().unwrap().is_running()
    }

    /// List available output devices.
    pub fn list_output_devices() -> Result<Vec<String>> {
        AudioEngine::list_output_devices()
    }

    /// Get the name of the current output device.
    pub fn current_output_device_name(&self) -> Result<String> {
        self.engine.lock().unwrap().current_output_device_name()
    }

    /// Set output device (requires restart to take effect).
    pub fn set_output_device(&self, index: Option<usize>) {
        self.engine.lock().unwrap().set_output_device(index);
    }

    /// Get number of output channels.
    pub fn channels(&self) -> usize {
        self.engine.lock().unwrap().channels()
    }

    /// Modify the DSP graph (non-realtime).
    ///
    /// # Example
    /// ```ignore
    /// system.graph(|net| {
    ///     let node = net.add(Box::new(sine_hz(440.0)));
    ///     net.pipe_output(node);
    /// });
    /// ```
    pub fn graph<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut TuttiNet) -> R,
    {
        let mut net = self.net.lock().unwrap();
        f(&mut net)
    }

    /// Get the transport manager.
    pub fn transport(&self) -> &Arc<TransportManager> {
        &self.transport
    }

    /// Get the metering manager.
    pub fn metering(&self) -> &Arc<MeteringManager> {
        &self.metering
    }

    /// Get the metronome.
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
}

/// Builder for TuttiSystem.
#[derive(Default)]
pub struct TuttiSystemBuilder {
    engine_config: AudioEngineConfig,
    inputs: usize,
    outputs: usize,
}

impl TuttiSystemBuilder {
    /// Set output device index.
    pub fn output_device(mut self, index: usize) -> Self {
        self.engine_config.output_device_index = Some(index);
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
        let mut engine = AudioEngine::new(self.engine_config)?;
        let sample_rate = engine.sample_rate();

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

        let mut callback_state = AudioCallbackState::new(
            transport.clone(),
            metering.clone(),
            sample_rate,
        );

        callback_state.set_net_backend(backend);

        engine.start(callback_state)?;

        Ok(TuttiSystem {
            engine: Mutex::new(engine),
            net: Mutex::new(net),
            transport,
            metering,
            metronome,
            pdc,
            #[cfg(feature = "neural")]
            neural,
            sample_rate,
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
