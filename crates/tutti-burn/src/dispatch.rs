//! CPU/GPU dispatch for neural models.

use burn::backend::wgpu::{Wgpu, WgpuDevice};
use burn::backend::NdArray;
use burn::prelude::*;

use crate::backend_pool::CpuDevice;
use crate::fusion::NeuralModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DevicePlacement {
    #[default]
    Cpu,
    Gpu,
}

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
    pub fn forward_flat(&self, data: &[f32], shape: [usize; 2]) -> Vec<f32> {
        match self {
            DeviceModel::Cpu { model, device } => {
                let input = Tensor::<NdArray, 1>::from_floats(data, device).reshape(shape);
                model.forward(input).into_data().to_vec().expect("to_vec")
            }
            DeviceModel::Gpu { model, device } => {
                let input = Tensor::<Wgpu, 1>::from_floats(data, device).reshape(shape);
                model.forward(input).into_data().to_vec().expect("to_vec")
            }
        }
    }

    pub fn placement(&self) -> DevicePlacement {
        match self {
            DeviceModel::Cpu { .. } => DevicePlacement::Cpu,
            DeviceModel::Gpu { .. } => DevicePlacement::Gpu,
        }
    }
}
