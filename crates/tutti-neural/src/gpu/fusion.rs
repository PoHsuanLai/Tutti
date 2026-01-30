//! Neural synth models with automatic kernel fusion.

use burn::prelude::*;
use burn::tensor::backend::Backend;

/// Neural synth model.
#[derive(Module, Debug)]
pub struct FusedNeuralSynthModel<B: Backend> {
    pub harmonic_net: HarmonicNetwork<B>,
    pub filter_net: FilterNetwork<B>,
    pub output_proj: nn::Linear<B>,
}

#[derive(Module, Debug)]
pub struct HarmonicNetwork<B: Backend> {
    pub fc1: nn::Linear<B>,
    pub fc2: nn::Linear<B>,
    pub fc3: nn::Linear<B>,
}

#[derive(Module, Debug)]
pub struct FilterNetwork<B: Backend> {
    pub fc1: nn::Linear<B>,
    pub fc2: nn::Linear<B>,
}

impl<B: Backend> FusedNeuralSynthModel<B> {
    /// Create a model with random weights.
    pub fn new(device: &B::Device) -> Self {
        let harmonic_net = HarmonicNetwork {
            fc1: nn::LinearConfig::new(128, 256).init(device),
            fc2: nn::LinearConfig::new(256, 256).init(device),
            fc3: nn::LinearConfig::new(256, 64).init(device),
        };

        let filter_net = FilterNetwork {
            fc1: nn::LinearConfig::new(128, 128).init(device),
            fc2: nn::LinearConfig::new(128, 64).init(device),
        };

        let output_proj = nn::LinearConfig::new(128, 2).init(device);

        Self {
            harmonic_net,
            filter_net,
            output_proj,
        }
    }

    /// Load model from file (supports .onnx, .mpk, .safetensors).
    pub fn load_from_file(
        path: impl AsRef<std::path::Path>,
        device: &B::Device,
    ) -> Result<Self, String>
    where
        B::FloatElem: burn::serde::de::DeserializeOwned,
    {
        let path = path.as_ref();
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .ok_or_else(|| "Invalid file extension".to_string())?;

        match ext {
            "onnx" => Self::load_from_onnx(path, device),
            "mpk" => Self::load_from_burn_mpk(path, device),
            "safetensors" => Self::load_from_safetensors(path, device),
            _ => Err(format!("Unsupported model format: .{}", ext)),
        }
    }

    /// Load from ONNX format
    ///
    /// ONNX runtime loading is not supported by Burn. ONNX models must be
    /// pre-converted to Burn's native `.mpk` format or to `.safetensors`
    /// using the `burn-import` CLI tool at build time.
    ///
    /// ```bash
    /// # Convert ONNX to Burn format:
    /// burn-import onnx model.onnx --out-type burn model.mpk
    /// ```
    fn load_from_onnx(_path: &std::path::Path, _device: &B::Device) -> Result<Self, String> {
        Err(
            "ONNX runtime loading is not supported. \
             Pre-convert your model using: burn-import onnx <model.onnx> --out-type burn <model.mpk> \
             or export as .safetensors from your training framework."
                .to_string(),
        )
    }

    /// Load from Burn's native MessagePack format
    ///
    /// This is Burn's native format - fastest and most optimized.
    fn load_from_burn_mpk(path: &std::path::Path, device: &B::Device) -> Result<Self, String>
    where
        B::FloatElem: burn::serde::de::DeserializeOwned,
    {
        use burn::record::{BinFileRecorder, FullPrecisionSettings, Recorder};

        // Create recorder for loading
        let recorder = BinFileRecorder::<FullPrecisionSettings>::default();

        // Load model record from file
        let record = recorder
            .load(path.to_path_buf(), device)
            .map_err(|e| format!("Failed to load Burn model: {:?}", e))?;

        // Initialize model structure and load weights
        let model = Self::new(device).load_record(record);

        Ok(model)
    }

    /// Load from SafeTensors format
    ///
    /// SafeTensors is a secure format from HuggingFace - prevents arbitrary code execution.
    /// Uses `burn-import`'s SafeTensorsFileRecorder for runtime loading.
    ///
    /// Key names in the SafeTensors file must match the Burn model structure:
    /// - `harmonic_net.fc1.weight`, `harmonic_net.fc1.bias`
    /// - `harmonic_net.fc2.weight`, `harmonic_net.fc2.bias`
    /// - etc.
    ///
    /// If the model was exported from PyTorch, use key remapping to adapt names.
    #[cfg(feature = "safetensors")]
    fn load_from_safetensors(path: &std::path::Path, device: &B::Device) -> Result<Self, String> {
        use burn::record::{FullPrecisionSettings, Recorder};
        use burn_import::safetensors::{LoadArgs, SafetensorsFileRecorder};

        let recorder = SafetensorsFileRecorder::<FullPrecisionSettings>::default();
        let args = LoadArgs::new(path.to_path_buf());

        let record = recorder
            .load(args, device)
            .map_err(|e| format!("Failed to load SafeTensors model: {:?}", e))?;

        let model = Self::new(device).load_record(record);
        Ok(model)
    }

    /// Load from SafeTensors format (stub when feature is disabled)
    #[cfg(not(feature = "safetensors"))]
    fn load_from_safetensors(_path: &std::path::Path, _device: &B::Device) -> Result<Self, String> {
        Err("SafeTensors loading requires the 'safetensors' feature. \
             Enable it in Cargo.toml: tutti-neural = { features = [\"safetensors\"] }"
            .to_string())
    }

    /// Forward pass with automatic kernel fusion
    ///
    /// CubeCL will automatically fuse:
    /// - Linear → ReLU → Linear chains
    /// - Multiple small tensor operations
    /// - Memory transfers between operations
    pub fn forward(&self, input: Tensor<B, 2>) -> Tensor<B, 2> {
        // Harmonic path - these ops will be fused by CubeCL
        let h1 = self.harmonic_net.fc1.forward(input.clone());
        let h1 = burn::tensor::activation::relu(h1);
        let h2 = self.harmonic_net.fc2.forward(h1);
        let h2 = burn::tensor::activation::relu(h2);
        let harmonics = self.harmonic_net.fc3.forward(h2);

        // Filter path - these ops will also be fused
        let f1 = self.filter_net.fc1.forward(input);
        let f1 = burn::tensor::activation::relu(f1);
        let filters = self.filter_net.fc2.forward(f1);

        // Concatenate and project
        let combined = Tensor::cat(vec![harmonics, filters], 1);

        // CubeCL fusion optimizations applied:
        // - Linear operations batched
        // - ReLU fused with preceding ops
        // - Reduced memory transfers
        self.output_proj.forward(combined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use burn::backend::ndarray::NdArrayDevice;
    use burn::backend::NdArray;

    type TestBackend = NdArray<f32>;

    #[test]
    fn test_fused_model_creation() {
        let device = NdArrayDevice::default();
        let _model: FusedNeuralSynthModel<TestBackend> = FusedNeuralSynthModel::new(&device);
        // Model should be created successfully (no panic)
    }

    #[test]
    fn test_fused_forward_pass() {
        let device = NdArrayDevice::default();
        let model: FusedNeuralSynthModel<TestBackend> = FusedNeuralSynthModel::new(&device);

        // Create dummy input [batch_size=4, features=128]
        let input = Tensor::<TestBackend, 2>::zeros([4, 128], &device);

        // Run forward pass
        let output = model.forward(input);

        // Check output shape [batch_size=4, output=2]
        assert_eq!(output.shape().dims, [4, 2]);
    }

    #[test]
    fn test_save_and_load_mpk() {
        use burn::record::{BinFileRecorder, FullPrecisionSettings, Recorder};
        use std::fs;

        let device = NdArrayDevice::default();

        // Create a model with random weights
        let model: FusedNeuralSynthModel<TestBackend> = FusedNeuralSynthModel::new(&device);

        // Save to .mpk file
        let temp_dir = std::env::temp_dir();
        let model_path = temp_dir.join("test_model.mpk");

        let recorder = BinFileRecorder::<FullPrecisionSettings>::default();
        let record = model.clone().into_record();
        recorder
            .record(record, model_path.clone())
            .expect("Failed to save model");

        // Load from .mpk file
        let loaded_model = FusedNeuralSynthModel::load_from_file(&model_path, &device)
            .expect("Failed to load model");

        // Test that loaded model produces output
        let input = Tensor::<TestBackend, 2>::ones([2, 128], &device);
        let output = loaded_model.forward(input);
        assert_eq!(output.shape().dims, [2, 2]);

        // Cleanup
        let _ = fs::remove_file(model_path);
    }

    #[test]
    fn test_onnx_error_message() {
        let device = NdArrayDevice::default();
        let result = FusedNeuralSynthModel::<TestBackend>::load_from_file("fake.onnx", &device);

        assert!(result.is_err());
        let err_msg = result.unwrap_err();
        assert!(err_msg.contains("burn-import"));
        assert!(err_msg.contains("ONNX"));
    }
}
