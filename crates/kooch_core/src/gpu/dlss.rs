//! The NVIDIA DLSS seam (#536).

use wgpu::{Adapter, Device, DeviceDescriptor, Instance, InstanceDescriptor, Limits, Queue};

use super::error::GpuError;

/// The identity this engine reports to NGX.
pub const PROJECT_ID: &str = "30faed7b-a4cd-4ab4-b4d1-56962b8342f6";

/// Which DLSS features this process can actually run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DlssSupport {
    /// DLSS Super Resolution is usable: the extensions registered, the
    /// adapter is an NVIDIA one that carries the feature, and the SDK
    /// initialised.
    pub super_resolution: bool,
}

#[cfg(feature = "dlss")]
pub use dlss_wgpu::DlssSdk;

/// The stand-in for [`DlssSdk`] when the crate was built without the SDK.
#[cfg(not(feature = "dlss"))]
pub enum DlssSdk {}

/// The application-wide DLSS object, shared with whoever creates a
/// per-camera context out of it.
pub type Sdk = std::sync::Arc<std::sync::Mutex<DlssSdk>>;

/// The application-wide DLSS handles, as a resource.
#[derive(Clone)]
pub struct DlssRuntime {
    /// `render` needs it to translate wgpu textures into Vulkan images.
    pub adapter: Adapter,
    pub sdk: Option<Sdk>,
    pub support: DlssSupport,
}

/// Creates the wgpu instance, registering the DLSS instance extensions when the feature is on.
pub(crate) fn instance(descriptor: InstanceDescriptor) -> (Instance, DlssSupport) {
    #[cfg(feature = "dlss")]
    {
        let mut found = dlss_wgpu::FeatureSupport::default();
        match dlss_wgpu::create_instance(project_id(), &descriptor, &mut found) {
            Ok(instance) => {
                let support = DlssSupport {
                    super_resolution: found.super_resolution_supported,
                };
                tracing::info!(?support, "DLSS instance extensions registered");
                return (instance, support);
            }
            Err(error) => {
                tracing::warn!("DLSS instance unavailable, using a plain one: {error}");
            }
        }
    }
    (Instance::new(descriptor), DlssSupport::default())
}

/// Creates the device, registering the DLSS device extensions when the instance managed to register
/// its own.
#[cfg_attr(not(feature = "dlss"), allow(unused_variables))]
pub(crate) fn device(
    adapter: &Adapter,
    descriptor: &DeviceDescriptor<'_>,
    limits: &Limits,
    support: &mut DlssSupport,
) -> Result<(Device, Queue), GpuError> {
    #[cfg(feature = "dlss")]
    if support.super_resolution {
        let mut found = dlss_wgpu::FeatureSupport {
            super_resolution_supported: true,
            ray_reconstruction_supported: false,
        };
        match dlss_wgpu::request_device(
            project_id(),
            adapter,
            descriptor,
            &mut found,
            Some(limits.clone()),
        ) {
            Ok(pair) => {
                support.super_resolution = found.super_resolution_supported;
                return Ok(pair);
            }
            Err(error) => {
                tracing::warn!("DLSS device unavailable, opening a plain one: {error}");
                support.super_resolution = false;
            }
        }
    }
    Ok(pollster::block_on(adapter.request_device(descriptor))?)
}

/// Initialises the application-wide SDK, which is where an adapter that
/// carries the extensions but not the feature is finally caught.
#[cfg_attr(not(feature = "dlss"), allow(unused_variables))]
pub(crate) fn sdk(device: &Device, support: &mut DlssSupport) -> Option<Sdk> {
    #[cfg(feature = "dlss")]
    if support.super_resolution {
        match DlssSdk::new(project_id(), device.clone()) {
            Ok(sdk) => {
                tracing::info!("DLSS super resolution available");
                return Some(sdk);
            }
            Err(error) => {
                // Not a warning: this is the expected outcome of running
                // a DLSS-enabled build on AMD or Intel, which is most of
                // the machines the engine targets.
                tracing::info!("DLSS not supported on this adapter: {error}");
            }
        }
    }
    support.super_resolution = false;
    None
}

#[cfg(feature = "dlss")]
fn project_id() -> uuid::Uuid {
    uuid::Uuid::parse_str(PROJECT_ID).expect("PROJECT_ID is a literal UUID")
}

#[cfg(test)]
mod tests;
