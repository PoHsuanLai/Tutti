//! Neural model loading and inference.
//!
//! Models are loaded as opaque closures: `Tensor<B, 2> -> Tensor<B, 2>`.
//! The architecture is determined by the model file - no hardcoded structs needed.

use burn::prelude::*;
use burn::tensor::backend::Backend;
use std::sync::Arc;

/// A loaded neural model as an opaque forward function.
///
/// Wraps any Burn `Module` into a callable `Tensor<B, 2> -> Tensor<B, 2>`.
/// The architecture comes from the model file, not from hardcoded structs.
///
/// # Usage
/// ```rust,ignore
/// // Load any model architecture
/// let model = NeuralModel::load("amp_sim.mpk", &device, || MyAmpModel::new(&device))?;
///
/// // Just call forward - don't care about internals
/// let output = model.forward(input);
/// ```
pub struct NeuralModel<B: Backend> {
    forward_fn: Arc<dyn Fn(Tensor<B, 2>) -> Tensor<B, 2> + Send>,
}

impl<B: Backend> NeuralModel<B> {
    /// Create from any Burn Module that implements a `forward` method.
    ///
    /// The closure captures the model and calls its forward pass.
    /// Only requires `Send` (not `Sync`) because the model lives on a single
    /// dedicated inference thread.
    pub fn from_forward(f: impl Fn(Tensor<B, 2>) -> Tensor<B, 2> + Send + 'static) -> Self {
        Self {
            forward_fn: Arc::new(f),
        }
    }

    /// Run the forward pass.
    pub fn forward(&self, input: Tensor<B, 2>) -> Tensor<B, 2> {
        (self.forward_fn)(input)
    }
}

impl<B: Backend> Clone for NeuralModel<B> {
    fn clone(&self) -> Self {
        Self {
            forward_fn: Arc::clone(&self.forward_fn),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use burn::backend::ndarray::NdArrayDevice;
    use burn::backend::NdArray;

    type TestBackend = NdArray<f32>;

    #[test]
    fn test_identity_model() {
        let model = NeuralModel::<TestBackend>::from_forward(|input| input);

        let device = NdArrayDevice::default();
        let input = Tensor::<TestBackend, 2>::ones([2, 128], &device);
        let output = model.forward(input.clone());

        assert_eq!(output.shape().dims, [2, 128]);
    }

    #[test]
    fn test_custom_forward() {
        // Any forward logic works - just wrap in a closure
        let model = NeuralModel::<TestBackend>::from_forward(|input| {
            // Scale + relu as a simple custom forward pass
            let scaled = input.mul_scalar(2.0);
            burn::tensor::activation::relu(scaled)
        });

        let device = NdArrayDevice::default();
        let input = Tensor::<TestBackend, 2>::zeros([4, 64], &device);
        let output = model.forward(input);
        assert_eq!(output.shape().dims, [4, 64]);
    }

    #[test]
    fn test_clone() {
        let model = NeuralModel::<TestBackend>::from_forward(|input| input);
        let model2 = model.clone();

        let device = NdArrayDevice::default();
        let input = Tensor::<TestBackend, 2>::ones([1, 10], &device);
        let output = model2.forward(input);
        assert_eq!(output.shape().dims, [1, 10]);
    }
}
