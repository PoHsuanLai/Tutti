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
    #[cfg(feature = "midi-io")]
    pub fn io(mut self) -> Self {
        self.enable_io = true;
        self
    }

    #[cfg(feature = "mpe")]
    pub fn mpe(mut self, mode: MpeMode) -> Self {
        self.mpe_mode = Some(mode);
        self
    }

    pub fn cc_mapping(mut self) -> Self {
        self.enable_cc_mapping = true;
        self
    }

    /// Collects MIDI output from audio nodes.
    pub fn output_collector(mut self) -> Self {
        self.enable_output_collector = true;
        self
    }

    pub fn build(self) -> Result<MidiSystem> {
        let port_manager = Arc::new(MidiPortManager::new(256));

        // MPE processor must exist before input manager (needs a clone)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_build() {
        let midi = MidiSystemBuilder::default().build().unwrap();

        // Port manager should always be present
        let ports = midi.list_ports();
        assert!(ports.is_empty(), "No ports should exist by default");

        // CC manager should be None by default
        assert!(midi.cc_manager().is_none());

        // Output collector should be None by default
        assert!(midi.output_collector().is_none());
    }

    #[test]
    fn test_build_with_cc_mapping() {
        let midi = MidiSystemBuilder::default().cc_mapping().build().unwrap();

        // CC manager should be present
        let cc = midi.cc_manager();
        assert!(cc.is_some(), "CC manager should be enabled");

        // Should be able to add mappings and retrieve them
        let manager = cc.unwrap();
        manager.add_mapping(None, 74, crate::cc::CCTarget::MasterVolume, 0.0, 1.0);
        assert_eq!(manager.get_all_mappings().len(), 1);
    }

    #[test]
    fn test_build_with_output_collector() {
        let midi = MidiSystemBuilder::default()
            .output_collector()
            .build()
            .unwrap();

        // Output collector should be present
        assert!(midi.output_collector().is_some());
    }
}
