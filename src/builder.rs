//! TuttiEngineBuilder for configuring the engine

use crate::core::TuttiSystemBuilder;
use crate::{Result, TuttiEngine};
use std::sync::Arc;

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
            Some(Arc::new(
                MidiSystem::builder()
                    .io() // Enable hardware I/O
                    .build()
                    .map_err(|e| crate::Error::InvalidConfig(e.to_string()))?,
            ))
        } else {
            None
        };

        // Build core system with MIDI routing if enabled
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
        let neural = Arc::new(
            NeuralSystem::builder()
                .build()
                .map_err(|e| crate::Error::InvalidConfig(e.to_string()))?,
        );

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
