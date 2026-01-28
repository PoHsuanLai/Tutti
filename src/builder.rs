//! TuttiEngineBuilder for configuring the engine

use crate::{TuttiEngine, Result};
use crate::core::TuttiSystemBuilder;

#[cfg(feature = "midi")]
use crate::midi::{MidiSystem, MidiSystemBuilder};

#[cfg(feature = "sampler")]
use crate::sampler::{SamplerSystem, SamplerSystemBuilder};

#[cfg(feature = "neural")]
use crate::neural::{NeuralSystem, NeuralSystemBuilder};

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
    pub fn with_midi(mut self) -> Self {
        self.enable_midi = true;
        self
    }

    /// Enable sampler subsystem
    #[cfg(feature = "sampler")]
    pub fn with_sampler(mut self) -> Self {
        self.enable_sampler = true;
        self
    }

    /// Enable neural subsystem
    #[cfg(feature = "neural")]
    pub fn with_neural(mut self) -> Self {
        self.enable_neural = true;
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

        let _sample_rate = self.sample_rate.unwrap_or_else(|| core.sample_rate());

        // Build optional subsystems
        #[cfg(feature = "midi")]
        let midi = if self.enable_midi {
            Some(MidiSystem::builder().build()?)
        } else {
            None
        };

        #[cfg(feature = "sampler")]
        let sampler = if self.enable_sampler {
            Some(SamplerSystem::builder(sample_rate).build()?)
        } else {
            None
        };

        #[cfg(feature = "neural")]
        let neural = if self.enable_neural {
            Some(NeuralSystem::builder().build()?)
        } else {
            None
        };

        Ok(TuttiEngine::from_parts(
            core,
            #[cfg(feature = "midi")]
            midi,
            #[cfg(feature = "sampler")]
            sampler,
            #[cfg(feature = "neural")]
            neural,
        ))
    }
}
