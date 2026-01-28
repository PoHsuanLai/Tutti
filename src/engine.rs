//! TuttiEngine that coordinates all audio subsystems

use crate::core::{MeteringManager, PdcManager, TransportManager, TuttiNet, TuttiSystem};
use crate::Result;
use std::sync::Arc;

#[cfg(feature = "midi")]
use crate::midi::MidiSystem;

#[cfg(feature = "sampler")]
use crate::sampler::SamplerSystem;

#[cfg(feature = "neural")]
use crate::neural::NeuralSystem;

/// Main audio engine that coordinates all subsystems.
///
/// TuttiEngine wraps tutti-core's TuttiSystem and optionally integrates:
/// - MIDI subsystem (if "midi" feature enabled)
/// - Sampler subsystem (if "sampler" feature enabled)
/// - Neural subsystem (if "neural" feature enabled)
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

    /// MIDI subsystem (optional)
    #[cfg(feature = "midi")]
    midi: Option<MidiSystem>,

    /// Sampler subsystem (optional)
    #[cfg(feature = "sampler")]
    sampler: Option<SamplerSystem>,

    /// Neural subsystem (optional)
    #[cfg(feature = "neural")]
    neural: Option<NeuralSystem>,
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
    pub fn is_running(&self) -> bool {
        self.core.is_running()
    }

    /// List available output devices
    pub fn list_output_devices() -> Result<Vec<String>> {
        TuttiSystem::list_output_devices()
    }

    /// Get current output device name
    pub fn current_output_device_name(&self) -> Result<String> {
        self.core.current_output_device_name()
    }

    /// Set output device
    pub fn set_output_device(&self, index: Option<usize>) {
        self.core.set_output_device(index);
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

    /// Get the transport manager
    pub fn transport(&self) -> &Arc<TransportManager> {
        self.core.transport()
    }

    /// Get the metering manager
    pub fn metering(&self) -> &Arc<MeteringManager> {
        self.core.metering()
    }

    /// Get the PDC manager
    pub fn pdc(&self) -> &Arc<PdcManager> {
        self.core.pdc()
    }

    /// Get the MIDI subsystem (if enabled)
    #[cfg(feature = "midi")]
    pub fn midi(&self) -> Option<&MidiSystem> {
        self.midi.as_ref()
    }

    /// Get the sampler subsystem (if enabled)
    #[cfg(feature = "sampler")]
    pub fn sampler(&self) -> Option<&SamplerSystem> {
        self.sampler.as_ref()
    }

    /// Get the neural subsystem (if enabled)
    #[cfg(feature = "neural")]
    pub fn neural(&self) -> Option<&NeuralSystem> {
        self.neural.as_ref()
    }

    /// Internal: create engine from builder
    pub(crate) fn from_parts(
        core: TuttiSystem,
        #[cfg(feature = "midi")] midi: Option<MidiSystem>,
        #[cfg(feature = "sampler")] sampler: Option<SamplerSystem>,
        #[cfg(feature = "neural")] neural: Option<NeuralSystem>,
    ) -> Self {
        Self {
            core,
            #[cfg(feature = "midi")]
            midi,
            #[cfg(feature = "sampler")]
            sampler,
            #[cfg(feature = "neural")]
            neural,
        }
    }
}
