//! Custom neural model implementation types.

pub use crate::synthesis::model_trait::{
    load_metadata, load_model_config, parameter_from_tensor_name, InputShape,
    MidiEvent as NeuralSynthMidiEvent, ModelConfig, ModelMetadata, NeuralSynthArchitecture,
    NeuralSynthInput, NeuralSynthModel, NeuralSynthOutput, NeuralSynthOutputData, OutputShape,
    ParameterDescriptor, ParameterType,
};
