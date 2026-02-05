//! Dynamic CPU/GPU dispatch for neural models.
//!
//! Models can be placed on either CPU (NdArray) or GPU (Wgpu) at registration
//! time. The `DeviceModel` enum wraps both backend instantiations and provides
//! a unified `forward_flat()` method that handles tensor creation and extraction
//! on the appropriate device.

use burn::backend::wgpu::{Wgpu, WgpuDevice};
use burn::backend::NdArray;
use burn::prelude::*;

use crate::backend_pool::CpuDevice;
use crate::fusion::NeuralModel;

/// Where a model should execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DevicePlacement {
    /// CPU via NdArray backend (always available).
    #[default]
    Cpu,
    /// GPU via Wgpu backend (requires GPU availability).
    Gpu,
}

/// A neural model that can live on either CPU or GPU.
///
/// Each variant captures its device at construction time, so `forward_flat()`
/// is self-contained — no external device references needed.
pub(crate) enum DeviceModel {
    Cpu {
        model: NeuralModel<NdArray>,
        device: CpuDevice,
    },
    Gpu {
        model: NeuralModel<Wgpu>,
        device: WgpuDevice,
    },
}

impl DeviceModel {
    /// Run forward pass on the appropriate device.
    ///
    /// Accepts flat `&[f32]` data and `[batch, features]` shape. Creates a
    /// device-specific tensor, runs the model's forward pass, and extracts
    /// the result back to flat `Vec<f32>`.
    pub fn forward_flat(&self, data: &[f32], shape: [usize; 2]) -> Vec<f32> {
        match self {
            DeviceModel::Cpu { model, device } => {
                let input = Tensor::<NdArray, 1>::from_floats(data, device).reshape(shape);
                let output = model.forward(input);
                output.into_data().to_vec::<f32>().expect("tensor to vec")
            }
            DeviceModel::Gpu { model, device } => {
                let input = Tensor::<Wgpu, 1>::from_floats(data, device).reshape(shape);
                let output = model.forward(input);
                output.into_data().to_vec::<f32>().expect("tensor to vec")
            }
        }
    }

    /// Returns the placement of this model.
    pub fn placement(&self) -> DevicePlacement {
        match self {
            DeviceModel::Cpu { .. } => DevicePlacement::Cpu,
            DeviceModel::Gpu { .. } => DevicePlacement::Gpu,
        }
    }
}
