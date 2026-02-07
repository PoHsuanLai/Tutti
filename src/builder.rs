//! TuttiEngineBuilder for configuring the engine

use crate::core::TuttiSystemBuilder;
use crate::{Result, TuttiEngine};

#[cfg(any(
    feature = "midi",
    feature = "sampler",
    feature = "neural",
    feature = "soundfont"
))]
use tutti_core::Arc;

#[cfg(feature = "midi")]
use crate::midi::MidiSystem;

#[cfg(feature = "sampler")]
use crate::sampler::SamplerSystem;

#[cfg(feature = "neural")]
use crate::neural::NeuralSystem;

/// Builder for TuttiEngine
///
/// Subsystems (sampler, neural, soundfont) are automatically enabled when their
/// corresponding Cargo features are compiled. MIDI requires explicit opt-in via
/// `.midi()` to connect hardware devices.
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
///     .sample_rate(48000.0)
///     .outputs(2)
///     .build()?;
///
/// // Subsystems are ready to use
/// let sampler = engine.sampler();
/// let neural = engine.neural();
/// ```
pub struct TuttiEngineBuilder {
    sample_rate: Option<f64>,
    output_device: Option<usize>,
    inputs: usize,
    outputs: usize,

    #[cfg(feature = "midi")]
    enable_midi: bool,

    #[cfg(feature = "plugin")]
    plugin_runtime: Option<tokio::runtime::Handle>,

    #[cfg(feature = "neural")]
    neural_backend_factory: Option<tutti_core::BackendFactory>,
}

impl Default for TuttiEngineBuilder {
    fn default() -> Self {
        Self {
            sample_rate: None,
            output_device: None,
            inputs: 0,
            outputs: 2,

            #[cfg(feature = "midi")]
            enable_midi: false,

            #[cfg(feature = "plugin")]
            plugin_runtime: None,

            #[cfg(feature = "neural")]
            neural_backend_factory: None,
        }
    }
}

impl TuttiEngineBuilder {
    /// Set sample rate (if not set, uses device default)
    pub fn sample_rate(mut self, rate: f64) -> Self {
        self.sample_rate = Some(rate);
        self
    }

    /// Set output device index
    pub fn output_device(mut self, index: usize) -> Self {
        self.output_device = Some(index);
        self
    }

    /// Set number of inputs (default: 0)
    pub fn inputs(mut self, count: usize) -> Self {
        self.inputs = count;
        self
    }

    /// Set number of outputs (default: 2)
    pub fn outputs(mut self, count: usize) -> Self {
        self.outputs = count;
        self
    }

    /// Enable MIDI subsystem
    #[cfg(feature = "midi")]
    pub fn midi(mut self) -> Self {
        self.enable_midi = true;
        self
    }

    /// Set a custom neural inference backend factory.
    ///
    /// Use this to provide your own inference backend (e.g. ONNX Runtime, candle)
    /// instead of the default Burn backend. If not set and the `burn` feature is
    /// enabled, the Burn backend is used automatically.
    ///
    /// # Example
    /// ```ignore
    /// let engine = TuttiEngine::builder()
    ///     .neural_backend(Box::new(|config| {
    ///         Ok(Box::new(MyOnnxBackend::new(config)?))
    ///     }))
    ///     .build()?;
    /// ```
    #[cfg(feature = "neural")]
    pub fn neural_backend(mut self, factory: tutti_core::BackendFactory) -> Self {
        self.neural_backend_factory = Some(factory);
        self
    }

    /// Set plugin runtime handle for async plugin loading
    ///
    /// Required for loading VST2, VST3, and CLAP plugins.
    ///
    /// # Example
    /// ```ignore
    /// let runtime = tokio::runtime::Runtime::new()?;
    /// let engine = TuttiEngine::builder()
    ///     .plugin_runtime(runtime.handle().clone())
    ///     .build()?;
    /// ```
    #[cfg(feature = "plugin")]
    pub fn plugin_runtime(mut self, handle: tokio::runtime::Handle) -> Self {
        self.plugin_runtime = Some(handle);
        self
    }

    /// Build and start the engine
    pub fn build(self) -> Result<TuttiEngine> {
        // Build MIDI subsystem first (if enabled) so we can pass port manager to core
        #[cfg(feature = "midi")]
        let midi = if self.enable_midi {
            #[allow(unused_mut)]
            let mut builder = MidiSystem::builder();
            #[cfg(feature = "midi-io")]
            {
                builder = builder.io();
            }
            Some(Arc::new(
                builder
                    .build()
                    .map_err(|e| crate::Error::InvalidConfig(e.to_string()))?,
            ))
        } else {
            None
        };

        // Build core system with MIDI routing if enabled
        #[allow(unused_mut)]
        let mut core_builder = TuttiSystemBuilder::default()
            .inputs(self.inputs)
            .outputs(self.outputs);

        #[cfg(feature = "std")]
        if let Some(device) = self.output_device {
            core_builder = core_builder.output_device(device);
        }

        // Wire up MIDI port manager to the audio callback
        // This enables hardware MIDI → audio graph routing
        // Routing is configured via engine.core().midi_routing() after building
        #[cfg(feature = "midi")]
        if let Some(ref midi_system) = midi {
            core_builder = core_builder.midi_input(midi_system.port_manager());
        }

        let core = core_builder.build()?;

        #[cfg(any(feature = "sampler", feature = "soundfont"))]
        let sample_rate = self.sample_rate.unwrap_or_else(|| core.sample_rate());

        // Build sampler subsystem (always enabled when feature is compiled)
        #[cfg(feature = "sampler")]
        let sampler = Arc::new(
            SamplerSystem::builder(sample_rate)
                .build()
                .map_err(|e| crate::Error::InvalidConfig(e.to_string()))?,
        );

        // Build neural subsystem (always enabled when feature is compiled)
        #[cfg(feature = "neural")]
        let neural = {
            let mut builder = NeuralSystem::builder();

            // Use custom backend if provided, otherwise use Burn if available
            if let Some(factory) = self.neural_backend_factory {
                builder = builder.backend(factory);
            } else {
                #[cfg(feature = "burn")]
                {
                    builder = builder.backend(tutti_burn::burn_backend_factory());
                }
            }

            Arc::new(
                builder
                    .build()
                    .map_err(|e| crate::Error::InvalidConfig(e.to_string()))?,
            )
        };

        // Build SoundFont manager (always enabled when feature is compiled)
        #[cfg(feature = "soundfont")]
        let soundfont = Arc::new(crate::synth::SoundFontSystem::new(sample_rate as u32));

        Ok(TuttiEngine::from_parts(
            core,
            #[cfg(feature = "midi")]
            midi,
            #[cfg(feature = "sampler")]
            sampler,
            #[cfg(feature = "neural")]
            neural,
            #[cfg(feature = "soundfont")]
            soundfont,
            #[cfg(feature = "plugin")]
            self.plugin_runtime,
        ))
    }
}
