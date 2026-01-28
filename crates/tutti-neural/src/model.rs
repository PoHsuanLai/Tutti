//! Custom neural model implementation types.

pub use crate::synthesis::model_trait::{
    InputShape, load_metadata, load_model_config, MidiEvent as NeuralSynthMidiEvent, ModelConfig,
    ModelMetadata, NeuralSynthArchitecture, NeuralSynthInput, NeuralSynthModel,
    NeuralSynthOutput, NeuralSynthOutputData, OutputShape, parameter_from_tensor_name,
    ParameterDescriptor, ParameterType,
};
