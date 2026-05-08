/// Errors that can occur during GPU context initialization.
#[derive(Debug)]
pub enum GpuError {
    /// No suitable GPU adapter was found.
    NoAdapter(wgpu::RequestAdapterError),
    /// Failed to create the wgpu surface.
    Surface(wgpu::CreateSurfaceError),
    /// Failed to request a GPU device.
    Device(wgpu::RequestDeviceError),
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAdapter(e) => write!(f, "no suitable GPU adapter found: {e}"),
            Self::Surface(e) => write!(f, "failed to create surface: {e}"),
            Self::Device(e) => write!(f, "failed to request device: {e}"),
        }
    }
}

impl std::error::Error for GpuError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NoAdapter(e) => Some(e),
            Self::Surface(e) => Some(e),
            Self::Device(e) => Some(e),
        }
    }
}

impl From<wgpu::CreateSurfaceError> for GpuError {
    fn from(e: wgpu::CreateSurfaceError) -> Self {
        Self::Surface(e)
    }
}

impl From<wgpu::RequestDeviceError> for GpuError {
    fn from(e: wgpu::RequestDeviceError) -> Self {
        Self::Device(e)
    }
}
