//! [`AssetPlugin`] — wires the engine's asset infrastructure into the `App`'s `Resources`.

mod eager;
mod plugin;
#[cfg(test)]
mod tests;

pub use eager::eager_import_with;
pub use plugin::AssetPlugin;
