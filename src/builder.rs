//! Builder for configuring and constructing a `TuttiEngine`.

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

/// Subsystems (sampler, neural, soundfont) are automatically enabled when their
/// corresponding Cargo features are compiled. MIDI requires explicit opt-in via
/// `.midi()` to connect hardware devices.
///
/// The sample rate is determined by the audio output device and cannot be
/// overridden. Use `engine.sample_rate()` after building to query the actual rate.
///
/// # Example
///
/// ```ignore
/// use tutti::prelude::*;
///
/// let engine = TuttiEngine::builder()
///     .outputs(2)
///     .build()?;
///
/// // Query the device sample rate
/// let sr = engine.sample_rate(); // e.g. 44100.0 or 48000.0
///
/// // Subsystems are ready to use
/// let sampler = engine.sampler();
/// let neural = engine.neural();
/// ```
pub struct TuttiEngineBuilder {
    output_device: Option<usize>,
    inputs: usize,
    outputs: usize,

    #[cfg(feature = "midi")]
    enable_midi: bool,

    #[cfg(feature = "mpe")]
    mpe_mode: Option<tutti_midi_io::MpeMode>,

    #[cfg(feature = "neural")]
    neural_backend_factory: Option<tutti_core::BackendFactory>,
}

impl Default for TuttiEngineBuilder {
    fn default() -> Self {
        Self {
            output_device: None,
            inputs: 0,
            outputs: 2,

            #[cfg(feature = "midi")]
            enable_midi: false,

            #[cfg(feature = "mpe")]
            mpe_mode: None,

            #[cfg(feature = "neural")]
            neural_backend_factory: None,
        }
    }
}

impl TuttiEngineBuilder {
    pub fn output_device(mut self, index: usize) -> Self {
        self.output_device = Some(index);
        self
    }

    /// Default: 0
    pub fn inputs(mut self, count: usize) -> Self {
        self.inputs = count;
        self
    }

    /// Default: 2
    pub fn outputs(mut self, count: usize) -> Self {
        self.outputs = count;
        self
    }

    #[cfg(feature = "midi")]
    pub fn midi(mut self) -> Self {
        self.enable_midi = true;
        self
    }

    /// Automatically enables the MIDI subsystem.
    #[cfg(feature = "mpe")]
    pub fn mpe(mut self, mode: tutti_midi_io::MpeMode) -> Self {
        self.mpe_mode = Some(mode);
        self.enable_midi = true;
        self
    }

    /// Provide a custom inference backend (e.g. ONNX Runtime, candle)
    /// instead of the default Burn backend. If not set and the `burn` feature is
    /// enabled, the Burn backend is used automatically.
    #[cfg(feature = "neural")]
    pub fn neural_backend(mut self, factory: tutti_core::BackendFactory) -> Self {
        self.neural_backend_factory = Some(factory);
        self
    }

    pub fn build(self) -> Result<TuttiEngine> {
        // Build MIDI first so we can pass port manager to core
        #[cfg(feature = "midi")]
        let midi = if self.enable_midi {
            #[allow(unused_mut)]
            let mut builder = MidiSystem::builder();
            #[cfg(feature = "midi-hardware")]
            {
                builder = builder.io();
            }
            #[cfg(feature = "mpe")]
            if let Some(mode) = self.mpe_mode {
                builder = builder.mpe(mode);
            }
            Some(Arc::new(builder.build()?))
        } else {
            None
        };

        #[allow(unused_mut)]
        let mut core_builder = TuttiSystemBuilder::default()
            .inputs(self.inputs)
            .outputs(self.outputs);

        #[cfg(feature = "std")]
        if let Some(device) = self.output_device {
            core_builder = core_builder.output_device(device);
        }

        // Wire MIDI port manager into the audio callback for hardware → graph routing.
        // Routing rules are configured via engine.midi_routing() after building.
        #[cfg(feature = "midi")]
        if let Some(ref midi_system) = midi {
            core_builder = core_builder.midi_input(midi_system.port_manager());
        }

        let core = core_builder.build()?;

        #[cfg(any(feature = "sampler", feature = "soundfont", feature = "neural"))]
        let sample_rate = core.sample_rate();

        #[cfg(feature = "sampler")]
        let sampler = Arc::new(
            SamplerSystem::builder(sample_rate)
                .pdc_manager(core.pdc().clone())
                .build()?,
        );

        #[cfg(feature = "neural")]
        let neural = {
            let mut builder = NeuralSystem::builder().sample_rate(sample_rate as f32);

            // Use custom backend if provided, otherwise use Burn if available
            if let Some(factory) = self.neural_backend_factory {
                builder = builder.backend(factory);
            } else {
                #[cfg(feature = "burn")]
                {
                    builder = builder.backend(tutti_burn::burn_backend_factory());
                }
            }

            Arc::new(builder.build()?)
        };

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
        ))
    }
}
