//! MidiSystem builder for configuring MIDI subsystems.

use std::sync::Arc;

use crate::error::Result;
use crate::port::MidiPortManager;

#[cfg(feature = "midi-io")]
use crate::io::{MidiInputManager, MidiOutputManager};

#[cfg(feature = "mpe")]
use crate::mpe::{MpeMode, MpeProcessor};
#[cfg(feature = "mpe")]
use parking_lot::RwLock;

use super::{MidiSystem, MidiSystemInner};

/// Builder for configuring MidiSystem
pub struct MidiSystemBuilder {
    #[cfg(feature = "midi-io")]
    pub(super) enable_io: bool,
    #[cfg(feature = "mpe")]
    pub(super) mpe_mode: Option<MpeMode>,
    pub(super) enable_cc_mapping: bool,
    pub(super) enable_output_collector: bool,
}

#[allow(clippy::derivable_impls)]
impl Default for MidiSystemBuilder {
    fn default() -> Self {
        Self {
            #[cfg(feature = "midi-io")]
            enable_io: false,
            #[cfg(feature = "mpe")]
            mpe_mode: None,
            enable_cc_mapping: false,
            enable_output_collector: false,
        }
    }
}

impl MidiSystemBuilder {
    /// Enable hardware MIDI I/O
    #[cfg(feature = "midi-io")]
    pub fn io(mut self) -> Self {
        self.enable_io = true;
        self
    }

    /// Enable MPE with the given mode
    #[cfg(feature = "mpe")]
    pub fn mpe(mut self, mode: MpeMode) -> Self {
        self.mpe_mode = Some(mode);
        self
    }

    /// Enable CC mapping manager
    pub fn cc_mapping(mut self) -> Self {
        self.enable_cc_mapping = true;
        self
    }

    /// Enable MIDI output collector (for collecting MIDI from audio nodes)
    pub fn output_collector(mut self) -> Self {
        self.enable_output_collector = true;
        self
    }

    /// Build the MIDI system
    pub fn build(self) -> Result<MidiSystem> {
        let port_manager = Arc::new(MidiPortManager::new(256));

        // Create MPE processor before input manager so it can be passed in
        #[cfg(feature = "mpe")]
        let mpe_processor = self
            .mpe_mode
            .map(|mode| Arc::new(RwLock::new(MpeProcessor::new(mode))));

        #[cfg(feature = "midi-io")]
        let (input_manager, output_manager) = if self.enable_io {
            let input = MidiInputManager::new(
                port_manager.clone(),
                #[cfg(feature = "mpe")]
                mpe_processor.clone(),
            );
            let output = MidiOutputManager::new();
            (Some(Arc::new(input)), Some(Arc::new(output)))
        } else {
            (None, None)
        };

        let cc_manager = if self.enable_cc_mapping {
            Some(Arc::new(crate::cc::CCMappingManager::new()))
        } else {
            None
        };

        let output_collector = if self.enable_output_collector {
            Some(Arc::new(
                crate::output_collector::MidiOutputAggregator::new(),
            ))
        } else {
            None
        };

        Ok(MidiSystem {
            inner: Arc::new(MidiSystemInner {
                port_manager,
                #[cfg(feature = "midi-io")]
                input_manager,
                #[cfg(feature = "midi-io")]
                output_manager,
                #[cfg(feature = "mpe")]
                mpe_processor,
                cc_manager,
                output_collector,
            }),
        })
    }
}
