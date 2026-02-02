//! Neural Model Registration for NodeRegistry
//!
//! Provides registration functions for neural audio models (synths, vocoders, amp sims, etc.)
//! to be used with tutti's NodeRegistry system.

use crate::error::Error as NeuralError;
use crate::system::NeuralSystem;
use std::path::{Path, PathBuf};
use tutti_core::{NodeRegistry, NodeRegistryError};

// Allow NeuralError to convert to NodeRegistryError
impl From<NeuralError> for NodeRegistryError {
    fn from(e: NeuralError) -> Self {
        NodeRegistryError::Neural(e.to_string())
    }
}

/// Register a neural model by path with a NeuralSystem instance
///
/// # Example
/// ```ignore
/// let neural = NeuralSystem::builder()
///     .sample_rate(44100.0)
///     .build()?;
/// let registry = NodeRegistry::default();
///
/// register_neural_model(&registry, &neural, "guitar_amp", "/models/guitar_amp.onnx")?;
///
/// // Later, create instances:
/// let amp = registry.create("guitar_amp", &NodeParams::new())?;
/// ```
pub fn register_neural_model<P: AsRef<Path>>(
    registry: &NodeRegistry,
    neural_system: &NeuralSystem,
    name: impl Into<String>,
    path: P,
) -> Result<(), NeuralError> {
    let path_buf = path.as_ref().to_path_buf();
    let model_name = name.into();
    let neural = neural_system.clone(); // NeuralSystem is Clone (Arc internally)

    registry.register(model_name, move |_params| {
        // Convert PathBuf to &str
        let path_str = path_buf.to_str().ok_or_else(|| {
            NodeRegistryError::ConstructionFailed("Invalid UTF-8 in path".to_string())
        })?;

        // Load neural model and build voice
        let model = neural.load_synth_model(path_str)?;
        let voice = neural.synth().build_voice(&model)?;
        Ok(voice)
    });

    Ok(())
}

/// Register all neural models in a directory
///
/// Scans the directory for .onnx, .safetensors, and .pt files and registers them.
///
/// # Example
/// ```ignore
/// register_neural_directory(&registry, &neural, "/path/to/models")?;
/// ```
pub fn register_neural_directory<P: AsRef<Path>>(
    registry: &NodeRegistry,
    neural_system: &NeuralSystem,
    path: P,
) -> Result<Vec<String>, NeuralError> {
    let mut registered = Vec::new();
    let dir_path = path.as_ref();

    if !dir_path.is_dir() {
        return Err(NeuralError::InvalidPath(format!(
            "Not a directory: {}",
            dir_path.display()
        )));
    }

    // Scan for model files
    for entry in std::fs::read_dir(dir_path)? {
        let entry = entry?;
        let path = entry.path();

        if is_neural_model_file(&path) {
            // Use filename (without extension) as registry key
            if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                match register_neural_model(registry, neural_system, name, &path) {
                    Ok(_) => {
                        tracing::info!("Registered neural model: {} from {}", name, path.display());
                        registered.push(name.to_string());
                    }
                    Err(e) => {
                        tracing::warn!("Failed to register {}: {}", path.display(), e);
                    }
                }
            }
        }
    }

    Ok(registered)
}

/// Register neural synthesis models
///
/// Registers neural synthesizers (any architecture: DDSP, WaveRNN, vocoders, etc.)
///
/// # Example
/// ```ignore
/// register_neural_synth_models(&registry, &neural, "/models/synth")?;
/// ```
pub fn register_neural_synth_models<P: AsRef<Path>>(
    registry: &NodeRegistry,
    neural_system: &NeuralSystem,
    path: P,
) -> Result<Vec<String>, NeuralError> {
    let mut registered = Vec::new();
    let model_path = path.as_ref();

    if !model_path.exists() {
        return Err(NeuralError::InvalidPath(format!(
            "Neural synth model path does not exist: {}",
            model_path.display()
        )));
    }

    // If it's a directory, scan it
    if model_path.is_dir() {
        registered.extend(register_neural_directory(
            registry,
            neural_system,
            model_path,
        )?);
    } else {
        // Single model file
        if let Some(name) = model_path.file_stem().and_then(|s| s.to_str()) {
            register_neural_model(
                registry,
                neural_system,
                format!("synth_{}", name),
                model_path,
            )?;
            registered.push(format!("synth_{}", name));
        }
    }

    tracing::info!("Registered {} neural synth models", registered.len());
    Ok(registered)
}

/// Register neural effect models (amp sims, compressors, etc.)
///
/// # Example
/// ```ignore
/// register_neural_effects(&registry, &neural, "/models/effects")?;
/// ```
pub fn register_neural_effects<P: AsRef<Path>>(
    registry: &NodeRegistry,
    neural_system: &NeuralSystem,
    path: P,
) -> Result<Vec<String>, NeuralError> {
    let mut registered = Vec::new();
    let effects_path = path.as_ref();

    if !effects_path.exists() {
        return Err(NeuralError::InvalidPath(format!(
            "Neural effects path does not exist: {}",
            effects_path.display()
        )));
    }

    // If it's a directory, scan it
    if effects_path.is_dir() {
        registered.extend(register_neural_directory(
            registry,
            neural_system,
            effects_path,
        )?);
    } else {
        // Single effect file
        if let Some(name) = effects_path.file_stem().and_then(|s| s.to_str()) {
            register_neural_model(
                registry,
                neural_system,
                format!("fx_{}", name),
                effects_path,
            )?;
            registered.push(format!("fx_{}", name));
        }
    }

    tracing::info!("Registered {} neural effects", registered.len());
    Ok(registered)
}

/// Register all neural models from standard locations
///
/// Convenience function that scans common model directories.
pub fn register_all_neural_models(
    registry: &NodeRegistry,
    neural_system: &NeuralSystem,
) -> Result<Vec<String>, NeuralError> {
    let mut registered = Vec::new();

    // Try common model locations
    let model_paths = get_neural_model_search_paths();

    for path in model_paths {
        if path.exists() {
            match register_neural_directory(registry, neural_system, &path) {
                Ok(mut models) => registered.append(&mut models),
                Err(e) => tracing::warn!("Failed to scan {}: {}", path.display(), e),
            }
        }
    }

    tracing::info!("Registered {} neural models total", registered.len());
    Ok(registered)
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Check if a path is a neural model file
fn is_neural_model_file(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        matches!(ext, "onnx" | "safetensors" | "pt" | "pth" | "bin")
    } else {
        false
    }
}

/// Get neural model search paths for the current platform
fn get_neural_model_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // User's home directory models
    if let Ok(home) = std::env::var("HOME") {
        paths.push(PathBuf::from(format!("{}/.tutti/models", home)));
        paths.push(PathBuf::from(format!("{}/Documents/Tutti/Models", home)));
    }

    // System-wide models
    #[cfg(target_os = "macos")]
    {
        paths.push(PathBuf::from("/Library/Application Support/Tutti/Models"));
    }

    #[cfg(target_os = "windows")]
    {
        paths.push(PathBuf::from("C:\\ProgramData\\Tutti\\Models"));
    }

    #[cfg(target_os = "linux")]
    {
        paths.push(PathBuf::from("/usr/share/tutti/models"));
        paths.push(PathBuf::from("/usr/local/share/tutti/models"));
    }

    // Current working directory
    paths.push(PathBuf::from("./models"));

    paths
}

/// Model metadata helper
#[derive(Debug, Clone)]
pub struct NeuralModelMetadata {
    pub name: String,
    pub path: PathBuf,
    pub model_type: ModelType,
    pub sample_rate: Option<f64>,
    pub latency_samples: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelType {
    DDSP,
    Vocoder,
    AmpSim,
    Compressor,
    Reverb,
    Other,
}

impl ModelType {
    /// Infer model type from filename or path
    pub fn from_path(path: &Path) -> Self {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();

        if name.contains("ddsp") {
            ModelType::DDSP
        } else if name.contains("vocoder") {
            ModelType::Vocoder
        } else if name.contains("amp") || name.contains("guitar") || name.contains("distortion") {
            ModelType::AmpSim
        } else if name.contains("comp") || name.contains("limiter") {
            ModelType::Compressor
        } else if name.contains("reverb") {
            ModelType::Reverb
        } else {
            ModelType::Other
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_neural_model_file() {
        assert!(is_neural_model_file(Path::new("/path/to/model.onnx")));
        assert!(is_neural_model_file(Path::new(
            "/path/to/model.safetensors"
        )));
        assert!(is_neural_model_file(Path::new("/path/to/model.pt")));
        assert!(!is_neural_model_file(Path::new("/path/to/model.txt")));
        assert!(!is_neural_model_file(Path::new("/path/to/model")));
    }

    #[test]
    fn test_model_type_inference() {
        assert_eq!(
            ModelType::from_path(Path::new("guitar_amp.onnx")),
            ModelType::AmpSim
        );
        assert_eq!(
            ModelType::from_path(Path::new("ddsp_synth.pt")),
            ModelType::DDSP
        );
        assert_eq!(
            ModelType::from_path(Path::new("compressor.onnx")),
            ModelType::Compressor
        );
    }
}
