//! GPU backend pool with automatic device detection.

use burn::backend::wgpu::{init_device, RuntimeOptions, WgpuDevice, WgpuSetup};
use burn::backend::{Autodiff, NdArray};
use std::marker::PhantomData;
use std::sync::Arc;
use tutti_core::neural::inference::InferenceError;
use wgpu::{Backends, DeviceDescriptor, Features, Limits, PowerPreference};

pub(crate) type GpuBackend = Autodiff<burn::backend::wgpu::Wgpu>;
pub(crate) type CpuBackend = Autodiff<NdArray>;
pub(crate) type CpuDevice = burn::backend::ndarray::NdArrayDevice;

pub(crate) struct BackendPool {
    gpu_device: Option<Arc<WgpuDevice>>,
    cpu_device: Arc<CpuDevice>,
    _gpu_backend: PhantomData<GpuBackend>,
    _cpu_backend: PhantomData<CpuBackend>,
}

impl BackendPool {
    pub fn new() -> Result<Self, InferenceError> {
        let cpu_device = Arc::new(CpuDevice::default());
        let gpu_device = Self::init_gpu().ok().map(Arc::new);

        Ok(Self {
            gpu_device,
            cpu_device,
            _gpu_backend: PhantomData,
            _cpu_backend: PhantomData,
        })
    }

    fn init_gpu() -> Result<WgpuDevice, InferenceError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: Self::preferred_backends(),
            ..Default::default()
        });

        let adapter = pollster::block_on(async {
            instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: PowerPreference::HighPerformance,
                    force_fallback_adapter: false,
                    compatible_surface: None,
                })
                .await
        })
        .map_err(|_| InferenceError::BackendInit("No GPU adapter available".into()))?;

        let adapter_info = adapter.get_info();
        tracing::debug!("Selected GPU adapter: {:?}", adapter_info);

        let (device, queue) = pollster::block_on(async {
            adapter
                .request_device(&DeviceDescriptor {
                    label: Some("tutti-burn GPU"),
                    required_features: Features::empty(),
                    required_limits: Limits::default(),
                    memory_hints: Default::default(),
                    trace: Default::default(),
                })
                .await
        })
        .map_err(|e| InferenceError::BackendInit(e.to_string()))?;

        let setup = WgpuSetup {
            instance,
            adapter,
            device,
            queue,
            backend: adapter_info.backend,
        };

        Ok(init_device(setup, RuntimeOptions::default()))
    }

    fn preferred_backends() -> Backends {
        #[cfg(target_os = "macos")]
        {
            Backends::METAL
        }
        #[cfg(target_os = "windows")]
        {
            Backends::DX12 | Backends::VULKAN
        }
        #[cfg(target_os = "linux")]
        {
            Backends::VULKAN
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Backends::all()
        }
    }

    pub fn has_gpu(&self) -> bool {
        self.gpu_device.is_some()
    }

    pub fn gpu_device(&self) -> Option<&Arc<WgpuDevice>> {
        self.gpu_device.as_ref()
    }

    pub fn cpu_device(&self) -> &Arc<CpuDevice> {
        &self.cpu_device
    }
}

impl Default for BackendPool {
    fn default() -> Self {
        Self::new().unwrap_or_else(|_| Self {
            gpu_device: None,
            cpu_device: Arc::new(CpuDevice::default()),
            _gpu_backend: PhantomData,
            _cpu_backend: PhantomData,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_pool_creation() {
        let pool = BackendPool::new().unwrap();
        let _cpu = pool.cpu_device();
        // GPU may or may not be available
    }
}
