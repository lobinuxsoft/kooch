//! What a project calls its layers (#1218).
//!
//! One table of 32 names, read by a renderer's mask and by a collider's groups alike, so a bit
//! means the same thing wherever it is ticked. The table lives in a `.layers` file beside the
//! project's other settings; a project without one gets the default names and nothing breaks.

use serde::{Deserialize, Serialize};

use crate::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};

/// What a layers file is called.
pub const LAYERS_EXTENSION: &str = "layers";
/// Bits a mask holds. A `u32` is the mask, and the instance buffer carries it as one.
pub const LAYER_COUNT: usize = 32;
/// The bit every renderer and every collider starts in.
pub const DEFAULT_LAYER: u32 = 1;

/// Static type name [`AssetEntry`](crate::asset_database::AssetEntry)s carry for a layers file.
pub const LAYERS_TYPE_NAME: &str = "kooch_core::layers::LayerNames";

/// The project's layer names, in bit order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerNames {
    /// Up to [`LAYER_COUNT`]; a missing or empty entry reads as `Layer n`.
    #[serde(default)]
    pub names: Vec<String>,
}

impl Default for LayerNames {
    fn default() -> Self {
        // Only the first is named: the rest read as their number until a project says otherwise,
        // which is what Unity's own table does with everything past Default.
        Self {
            names: vec!["Default".to_owned()],
        }
    }
}

impl LayerNames {
    /// What bit `index` is called. Out of range, unnamed or blank reads as its number, so a mask
    /// never shows an empty box nobody can identify.
    pub fn label(&self, index: usize) -> String {
        match self.names.get(index).map(String::as_str) {
            Some(name) if !name.trim().is_empty() => name.to_owned(),
            _ => format!("Layer {index}"),
        }
    }

    /// Names bit `index`, growing the table to reach it.
    pub fn set(&mut self, index: usize, name: impl Into<String>) {
        if index >= LAYER_COUNT {
            return;
        }
        if self.names.len() <= index {
            self.names.resize(index + 1, String::new());
        }
        self.names[index] = name.into();
    }

    /// Every bit's name, in order: what a checklist draws.
    pub fn labels(&self) -> Vec<String> {
        (0..LAYER_COUNT).map(|index| self.label(index)).collect()
    }
}

/// Reads a `.layers` file.
#[derive(Debug, Default, Clone, Copy)]
pub struct LayerNamesLoader;

impl AssetLoader<LayerNames> for LayerNamesLoader {
    fn extensions(&self) -> &[&'static str] {
        &[LAYERS_EXTENSION]
    }

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<LayerNames> {
        let text = std::str::from_utf8(bytes).map_err(|e| AssetError::Loader(Box::new(e)))?;
        ron::from_str(text).map_err(|e| AssetError::Loader(Box::new(e)))
    }
}

crate::register_asset!(LayerNames, LayerNamesLoader);

/// Publishes the project's layer names, or the defaults when it has none. Every consumer reads the
/// resource, so a project without a `.layers` file still shows named boxes.
pub fn publish_layer_names_system(resources: &mut crate::resource::Resources) {
    let guid = resources
        .get::<crate::asset_database::AssetDatabase>()
        .and_then(|db| {
            db.entries_of_type(LAYERS_TYPE_NAME)
                .next()
                .map(|(guid, _)| guid)
        });
    let names = guid
        .and_then(|guid| load(resources, guid))
        .unwrap_or_default();
    if resources.get::<LayerNames>() != Some(&names) {
        resources.insert(names);
    }
}

/// The names in the file `guid` points at.
fn load(resources: &mut crate::resource::Resources, guid: crate::Guid) -> Option<LayerNames> {
    let mut server = resources.remove::<crate::asset_loader::AssetServer>()?;
    let handle = server.load_by_guid::<LayerNames>(guid, resources);
    resources.insert(server);
    let assets = resources.get::<crate::assets::Assets<LayerNames>>()?;
    assets.get(handle.ok()?).cloned()
}

#[cfg(test)]
mod tests;
