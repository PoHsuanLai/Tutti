//! GPU backend pool for managing Burn backends across different devices

use crate::error::{GpuError, Result};
use burn::backend::wgpu::{init_device, RuntimeOptions, WgpuDevice, WgpuSetup};
use burn::backend::{Autodiff, NdArray};
use std::marker::PhantomData;
use std::sync::Arc;
use wgpu::{Backends, DeviceDescriptor, Features, Limits, PowerPreference};

/// GPU backend type using Wgpu (Burn's Wgpu backend with fusion + autodiff)
pub type GpuBackend = Autodiff<burn::backend::wgpu::Wgpu>;

/// CPU fallback backend using NdArray
pub type CpuBackend = Autodiff<NdArray>;

/// Device type for NdArray (always CPU)
pub type CpuDevice = burn::backend::ndarray::NdArrayDevice;

/// Pool of Burn backends for different compute devices
///
/// Note: Burn backends (GpuBackend, CpuBackend) are zero-sized types.
/// What we actually store are the devices (WgpuDevice, NdArrayDevice).
pub struct BackendPool {
    /// GPU device (if available)
    gpu_device: Option<Arc<WgpuDevice>>,

    /// CPU device (always available)
    cpu_device: Arc<CpuDevice>,

    /// Information about the selected GPU device
    gpu_info: Option<GpuInfo>,

    /// Marker for backend types (zero-sized)
    _gpu_backend: PhantomData<GpuBackend>,
    _cpu_backend: PhantomData<CpuBackend>,
}

/// GPU information
#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub name: String,
    pub backend: GpuBackendType,
    pub max_memory_mb: Option<u64>,
}

/// GPU backend type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuBackendType {
    Metal,
    Vulkan,
    Dx12,
    OpenGl,
}

impl BackendPool {
    /// Create a new backend pool with automatic GPU detection
    pub fn new() -> Result<Self> {
        // CPU device (always available)
        let cpu_device = Arc::new(CpuDevice::default());

        // Try to initialize GPU device
        let (gpu_device, gpu_info) = match Self::init_gpu() {
            Ok((device, info)) => (Some(Arc::new(device)), Some(info)),
            Err(_e) => {
                // GPU initialization failed, using CPU fallback
                (None, None)
            }
        };

        Ok(Self {
            gpu_device,
            cpu_device,
            gpu_info,
            _gpu_backend: PhantomData,
            _cpu_backend: PhantomData,
        })
    }

    /// Initialize GPU backend using wgpu and return the device
    fn init_gpu() -> Result<(WgpuDevice, GpuInfo)> {
        // Create wgpu instance
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: Self::preferred_backends(),
            ..Default::default()
        });

        // Request adapter
        let adapter = pollster::block_on(async {
            instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: PowerPreference::HighPerformance,
                    force_fallback_adapter: false,
                    compatible_surface: None,
                })
                .await
        })
        .map_err(|_| GpuError::NoGpuAvailable)?;

        // Get adapter info
        let adapter_info = adapter.get_info();
        let backend_type = Self::backend_type_from_wgpu(&adapter_info.backend);
        let wgpu_backend = adapter_info.backend;

        tracing::debug!("Selected GPU adapter: {:?}", adapter_info);

        // Request device and queue
        let (device, queue) = pollster::block_on(async {
            adapter
                .request_device(&DeviceDescriptor {
                    label: Some("DAWAI GPU Device"),
                    required_features: Features::empty(),
                    required_limits: Limits::default(),
                    memory_hints: Default::default(),
                    experimental_features: wgpu::ExperimentalFeatures::disabled(),
                    trace: Default::default(),
                })
                .await
        })
        .map_err(|e| GpuError::BackendInitFailed(e.to_string()))?;

        // Create WgpuSetup and initialize device with Burn
        let setup = WgpuSetup {
            instance,
            adapter,
            device,
            queue,
            backend: wgpu_backend,
        };

        let wgpu_device = init_device(setup, RuntimeOptions::default());

        let gpu_info = GpuInfo {
            name: adapter_info.name.clone(),
            backend: backend_type,
            max_memory_mb: None, // wgpu doesn't expose memory limits easily
        };

        Ok((wgpu_device, gpu_info))
    }

    /// Get preferred wgpu backends for the current platform
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

    /// Convert wgpu backend to our enum
    fn backend_type_from_wgpu(backend: &wgpu::Backend) -> GpuBackendType {
        match backend {
            wgpu::Backend::Metal => GpuBackendType::Metal,
            wgpu::Backend::Vulkan => GpuBackendType::Vulkan,
            wgpu::Backend::Dx12 => GpuBackendType::Dx12,
            wgpu::Backend::Gl => GpuBackendType::OpenGl,
            _ => GpuBackendType::Vulkan, // Default fallback
        }
    }

    /// Check if GPU backend is available
    pub fn has_gpu(&self) -> bool {
        self.gpu_device.is_some()
    }

    /// Get CPU device (always available)
    pub fn cpu_device(&self) -> &Arc<CpuDevice> {
        &self.cpu_device
    }

    /// Get GPU information
    pub fn gpu_info(&self) -> Option<&GpuInfo> {
        self.gpu_info.as_ref()
    }
}

impl Default for BackendPool {
    fn default() -> Self {
        Self::new().unwrap_or_else(|_e| {
            // Failed to create BackendPool, fallback to CPU-only
            Self {
                gpu_device: None,
                cpu_device: Arc::new(CpuDevice::default()),
                gpu_info: None,
                _gpu_backend: PhantomData,
                _cpu_backend: PhantomData,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_pool_creation() {
        let pool = BackendPool::new();
        assert!(pool.is_ok(), "BackendPool should initialize successfully");

        let pool = pool.unwrap();
        // CPU device should always be available
        let _cpu = pool.cpu_device();

        // GPU may or may not be available depending on the system
        if pool.has_gpu() {
            println!("GPU backend available: {:?}", pool.gpu_info());
        } else {
            println!("No GPU backend available, using CPU fallback");
        }
    }
}
