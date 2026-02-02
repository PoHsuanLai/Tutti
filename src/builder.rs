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
/// # Example
///
/// ```ignore
/// use tutti::prelude::*;
///
/// let engine = TuttiEngine::builder()
///     .sample_rate(48000.0)
///     .outputs(2)
///     .build()?;
/// ```
pub struct TuttiEngineBuilder {
    sample_rate: Option<f64>,
    output_device: Option<usize>,
    inputs: usize,
    outputs: usize,

    #[cfg(feature = "midi")]
    enable_midi: bool,

    #[cfg(feature = "sampler")]
    enable_sampler: bool,

    #[cfg(feature = "neural")]
    enable_neural: bool,

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

            #[cfg(feature = "sampler")]
            enable_sampler: false,

            #[cfg(feature = "neural")]
            enable_neural: false,

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

    /// Enable sampler subsystem
    #[cfg(feature = "sampler")]
    pub fn sampler(mut self) -> Self {
        self.enable_sampler = true;
        self
    }

    /// Enable neural subsystem
    #[cfg(feature = "neural")]
    pub fn neural(mut self) -> Self {
        self.enable_neural = true;
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
        // Build core system
        let mut core_builder = TuttiSystemBuilder::default()
            .inputs(self.inputs)
            .outputs(self.outputs);

        if let Some(device) = self.output_device {
            core_builder = core_builder.output_device(device);
        }

        let core = core_builder.build()?;

        let sample_rate = self.sample_rate.unwrap_or_else(|| core.sample_rate());

        // Build optional subsystems
        #[cfg(feature = "midi")]
        let midi = if self.enable_midi {
            Some(Arc::new(
                MidiSystem::builder()
                    .build()
                    .map_err(|e| crate::Error::InvalidConfig(e.to_string()))?,
            ))
        } else {
            None
        };

        #[cfg(feature = "sampler")]
        let sampler = if self.enable_sampler {
            Some(
                SamplerSystem::builder(sample_rate)
                    .build()
                    .map_err(|e| crate::Error::InvalidConfig(e.to_string()))?,
            )
        } else {
            None
        };

        #[cfg(feature = "neural")]
        let neural = if self.enable_neural {
            Some(
                NeuralSystem::builder()
                    .build()
                    .map_err(|e| crate::Error::InvalidConfig(e.to_string()))?,
            )
        } else {
            None
        };

        // SoundFont manager is always created if feature is enabled (lazy loading)
        #[cfg(feature = "soundfont")]
        let soundfont_manager = Some(Arc::new(crate::synth::SoundFontSystem::new(sample_rate as u32)));

        Ok(TuttiEngine::from_parts(
            core,
            #[cfg(feature = "midi")]
            midi,
            #[cfg(feature = "sampler")]
            sampler,
            #[cfg(feature = "neural")]
            neural,
            #[cfg(feature = "soundfont")]
            soundfont_manager,
            #[cfg(feature = "plugin")]
            self.plugin_runtime,
        ))
    }
}
