//! The NVIDIA DLSS seam (#536).
//!
//! DLSS is not a render pass that can be bolted on at the end: NGX
//! wants Vulkan instance and device extensions registered while those
//! objects are being created, so the decision lands here, in the crate
//! that owns [`GpuContext`](super::GpuContext), rather than in
//! `kooch_render` where the upscaler itself lives.
//!
//! 🔴 **It is a compile-time feature and cannot be anything else.**
//! `dlss_wgpu`'s build script links `libnvsdk_ngx` statically and runs
//! bindgen over NVIDIA's headers, so `DLSS_SDK` and `VULKAN_SDK` have
//! to be set when the binary is built. A build without the `dlss`
//! feature still compiles, runs and upscales — with the engine's own
//! techniques, which is the default on every adapter anyway.
//!
//! ⚠️ With the feature on, the instance is Vulkan-only. On Linux that
//! changes nothing; on Windows it moves every player off D3D12, NVIDIA
//! or not. That cost is why the feature exists rather than the code
//! being unconditional.

use wgpu::{Adapter, Device, DeviceDescriptor, Instance, InstanceDescriptor, Limits, Queue};

use super::error::GpuError;

/// The identity this engine reports to NGX.
///
/// One per application is what NVIDIA asks for. A game built with
/// Kóoch inherits it rather than minting its own, which is honest —
/// NGX uses it to look up per-title tuning it does not have for us
/// either way.
pub const PROJECT_ID: &str = "30faed7b-a4cd-4ab4-b4d1-56962b8342f6";

/// Which DLSS features this process can actually run.
///
/// Populated during context creation and false in every build without
/// the `dlss` feature, so a caller reads one field instead of a `cfg`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DlssSupport {
    /// DLSS Super Resolution is usable: the extensions registered, the
    /// adapter is an NVIDIA one that carries the feature, and the SDK
    /// initialised.
    pub super_resolution: bool,
}

#[cfg(feature = "dlss")]
pub use dlss_wgpu::DlssSdk;

/// The stand-in for [`DlssSdk`] when the crate was built without the
/// SDK.
///
/// Uninhabited on purpose: `Option<Sdk>` is then provably `None`, so
/// [`GpuContext`](super::GpuContext) keeps ONE shape instead of two and
/// nothing downstream needs a `cfg` to ask whether DLSS is there.
#[cfg(not(feature = "dlss"))]
pub enum DlssSdk {}

/// The application-wide DLSS object, shared with whoever creates a
/// per-camera context out of it.
pub type Sdk = std::sync::Arc<std::sync::Mutex<DlssSdk>>;

/// The application-wide DLSS handles, as a resource.
///
/// 🔴 Lifted out of [`GpuContext`](super::GpuContext) rather than read
/// through it: the render systems REMOVE the context from `Resources`
/// for the duration of a frame, so a pass that reaches for the adapter
/// mid-frame would find nothing there. Cloning is cheap — both fields
/// are handles.
#[derive(Clone)]
pub struct DlssRuntime {
    /// `render` needs it to translate wgpu textures into Vulkan images.
    pub adapter: Adapter,
    pub sdk: Option<Sdk>,
    pub support: DlssSupport,
}

/// Creates the wgpu instance, registering the DLSS instance extensions
/// when the feature is on.
///
/// Never fails on DLSS's account: an adapter that cannot carry the
/// extensions gets a plain instance and `super_resolution: false`.
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

/// Creates the device, registering the DLSS device extensions when the
/// instance managed to register its own.
///
/// Clears `support` rather than failing when the device cannot be
/// opened with them, and opens a plain one instead.
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
