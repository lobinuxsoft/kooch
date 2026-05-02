//! Convenient re-exports for common usage.
//!
//! ```ignore
//! use ome_core::prelude::*;
//! ```

pub use crate::app::App;
pub use crate::asset_loader::{AssetError, AssetLoader, AssetResult, AssetServer, LoadContext};
pub use crate::assets::{Asset, Assets, Handle};
pub use crate::buffer::{BufferPool, GpuBuffer, StagingBuffer, UniformBuffer};
pub use crate::event::{AppExit, EventReader, EventRegistry, Events};
pub use crate::gpu::{GpuContext, GpuError};
pub use crate::plugin::{CorePlugin, MinimalPlugins, Plugin, PluginGroup, PluginGroupBuilder};
pub use crate::resource::Resources;
pub use crate::runner::{default_runner, run_for_frames, run_once, Runner};
pub use crate::schedule::{Schedule, SystemFn};
pub use crate::stage::Stage;
pub use crate::system::{FunctionSystem, GpuSystem, System};
pub use crate::time::Time;

#[cfg(feature = "dynamic")]
pub use crate::dynamic::{
    DynamicPlugin, EntityBridge, PluginData, PluginLoadError, PluginLoader, ResourceRegistry,
};
