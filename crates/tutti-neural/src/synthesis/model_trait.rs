//! Neural synth model trait.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Neural synth model trait.
pub trait NeuralSynthModel: Send {
    fn metadata(&self) -> ModelMetadata;
    fn parameters(&self) -> Vec<ParameterDescriptor>;
    fn architecture(&self) -> NeuralSynthArchitecture;
    fn infer(&self, input: NeuralSynthInput) -> Result<NeuralSynthOutput, String>;
    fn input_shape(&self) -> InputShape;
    fn output_shape(&self) -> OutputShape;
}

/// Model metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMetadata {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Parameter descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterDescriptor {
    pub id: String,
    pub display_name: String,
    pub param_type: ParameterType,
    pub default: f32,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ParameterType {
    Continuous { min: f32, max: f32 },
    Discrete { options: Vec<String> },
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NeuralSynthArchitecture {
    Harmonic,
    Wavetable,
    Filtered,
    RawAudio,
}

#[derive(Debug, Clone)]
pub struct NeuralSynthInput {
    pub audio: Option<Vec<f32>>,
    pub midi: Vec<MidiEvent>,
    pub parameters: HashMap<String, f32>,
    pub sample_rate: f32,

    /// Buffer size (number of samples to generate)
    pub buffer_size: usize,
}

/// Output from neural synth model
#[derive(Debug, Clone)]
pub struct NeuralSynthOutput {
    /// Architecture-specific output
    pub data: NeuralSynthOutputData,

    /// Inference latency (ms)
    pub latency_ms: f32,
}

/// Architecture-specific output data
#[derive(Debug, Clone)]
pub enum NeuralSynthOutputData {
    /// Harmonic synthesis output
    Harmonic {
        /// Fundamental frequency per frame
        f0: Vec<f32>,
        /// Harmonic amplitudes per frame
        amplitudes: Vec<f32>,
        /// Noise band gains (optional)
        noise_bands: Option<Vec<f32>>,
    },

    /// Wavetable synthesis output
    Wavetable {
        /// Wavetable index per frame
        wavetable_index: Vec<f32>,
        /// Position in wavetable per frame
        position: Vec<f32>,
        /// Amplitudes per frame
        amplitudes: Vec<f32>,
    },

    /// Filter-based output
    Filtered {
        /// Source audio/noise
        source: Vec<f32>,
        /// Filter parameters (cutoff, resonance, etc.)
        filter_params: Vec<f32>,
    },

    /// Raw audio output
    RawAudio {
        /// Direct audio samples
        samples: Vec<f32>,
    },
}

/// Expected input shape
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputShape {
    /// Audio input channels (None if no audio input)
    pub audio_channels: Option<usize>,

    /// Expected audio frames per inference
    pub audio_frames: Option<usize>,

    /// Number of parameters
    pub num_parameters: usize,
}

/// Expected output shape
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputShape {
    /// Number of output frames (for f0, amplitudes, etc.)
    pub num_frames: usize,

    /// Number of harmonics (for Harmonic architecture)
    pub num_harmonics: Option<usize>,

    /// Number of noise bands (for Harmonic architecture)
    pub num_noise_bands: Option<usize>,
}

/// MIDI event for neural synth input
#[derive(Debug, Clone)]
pub struct MidiEvent {
    /// Sample offset within the current buffer
    pub offset: usize,
    /// MIDI data (up to 3 bytes for channel messages)
    pub data: [u8; 3],
    /// Number of valid bytes in data
    pub len: usize,
}

/// Full model configuration loaded from TOML sidecar file
///
/// Includes both metadata and parameter descriptors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Model name
    pub name: String,

    /// Model version
    pub version: String,

    /// Author/creator
    #[serde(default)]
    pub author: Option<String>,

    /// Description
    #[serde(default)]
    pub description: Option<String>,

    /// Tags for categorization
    #[serde(default)]
    pub tags: Vec<String>,

    /// Architecture type (defaults to Harmonic)
    #[serde(default = "default_architecture")]
    pub architecture: NeuralSynthArchitecture,

    /// Parameter descriptors
    #[serde(default)]
    pub parameters: Vec<ParameterDescriptor>,
}

fn default_architecture() -> NeuralSynthArchitecture {
    NeuralSynthArchitecture::Harmonic
}

impl ModelConfig {
    /// Extract metadata from config
    pub fn metadata(&self) -> ModelMetadata {
        ModelMetadata {
            name: self.name.clone(),
            version: self.version.clone(),
            author: self.author.clone(),
            description: self.description.clone(),
            tags: self.tags.clone(),
        }
    }
}

/// Helper: Load full model config from TOML sidecar file
///
/// Looks for `model_name.toml` next to `model_name.onnx` or `model_name.mpk`
///
/// # Example TOML:
/// ```toml
/// name = "Neural Singing Voice"
/// version = "1.0.0"
/// author = "Your Name"
/// description = "Neural singing voice synthesizer"
/// tags = ["voice", "singing", "vocal"]
/// architecture = "Harmonic"
///
/// [[parameters]]
/// id = "pitch_shift"
/// display_name = "Pitch Shift"
/// default = 0.0
/// unit = "semitones"
/// description = "Transpose the voice"
/// [parameters.param_type]
/// type = "Continuous"
/// min = -12.0
/// max = 12.0
/// ```
pub fn load_model_config(model_path: &std::path::Path) -> Option<ModelConfig> {
    let toml_path = model_path.with_extension("toml");

    if !toml_path.exists() {
        return None;
    }

    match std::fs::read_to_string(&toml_path) {
        Ok(contents) => toml::from_str(&contents).ok(),
        Err(_e) => None,
    }
}

/// Helper: Load just model metadata from TOML sidecar file (backwards compatible)
pub fn load_metadata(model_path: &std::path::Path) -> Option<ModelMetadata> {
    load_model_config(model_path).map(|c| c.metadata())
}

/// Helper: Create default parameter from tensor name
///
/// Fallback when no metadata is available - generates basic parameters
/// from ONNX tensor names (e.g., "input_0" → parameter "input_0")
pub fn parameter_from_tensor_name(name: &str, index: usize) -> ParameterDescriptor {
    ParameterDescriptor {
        id: name.to_string(),
        display_name: format!("Parameter {}", index),
        param_type: ParameterType::Continuous { min: 0.0, max: 1.0 },
        default: 0.5,
        description: Some(format!("Neural parameter from tensor: {}", name)),
        unit: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_serialization() {
        let metadata = ModelMetadata {
            name: "Test Model".to_string(),
            version: "1.0.0".to_string(),
            author: Some("Test Author".to_string()),
            description: Some("Test description".to_string()),
            tags: vec!["voice".to_string(), "singing".to_string()],
        };

        let toml = toml::to_string(&metadata).unwrap();
        let deserialized: ModelMetadata = toml::from_str(&toml).unwrap();

        assert_eq!(metadata.name, deserialized.name);
        assert_eq!(metadata.version, deserialized.version);
    }

    #[test]
    fn test_parameter_descriptor() {
        let param = ParameterDescriptor {
            id: "pitch_shift".to_string(),
            display_name: "Pitch Shift".to_string(),
            param_type: ParameterType::Continuous {
                min: -12.0,
                max: 12.0,
            },
            default: 0.0,
            description: Some("Transpose voice".to_string()),
            unit: Some("semitones".to_string()),
        };

        assert_eq!(param.id, "pitch_shift");
    }

    #[test]
    fn test_fallback_parameter() {
        let param = parameter_from_tensor_name("input_0", 0);

        assert_eq!(param.id, "input_0");
        assert_eq!(param.display_name, "Parameter 0");
        assert!(param.description.is_some());
    }
}
