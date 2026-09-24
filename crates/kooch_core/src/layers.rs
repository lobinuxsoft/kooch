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

/// The project's layer names and which of them collide, in bit order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerNames {
    /// Up to [`LAYER_COUNT`]; a missing or empty entry reads as `Layer n`.
    #[serde(default)]
    pub names: Vec<String>,
    /// Which layers each layer collides with: one mask per layer, symmetric.
    ///
    /// 🔴 One table for the whole project, as Unity does it, rather than four masks on every
    /// collider: a relationship is between two layers, so authoring it on each side is the same
    /// fact written twice and a chance to write it differently (#1302). A file with no table
    /// collides everything with everything, which is what every project did before there was one.
    #[serde(default)]
    pub matrix: Option<Vec<u32>>,
}

impl Default for LayerNames {
    fn default() -> Self {
        // Only the first is named: the rest read as their number until a project says otherwise,
        // which is what Unity's own table does with everything past Default.
        Self {
            names: vec!["Default".to_owned()],
            matrix: None,
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

    /// Whether a body on layer `a` meets one on layer `b`. Without a table, everything meets
    /// everything: a project that never opened the matrix keeps the behaviour it had.
    pub fn collide(&self, a: usize, b: usize) -> bool {
        let Some(matrix) = self.matrix.as_ref() else {
            return true;
        };
        match matrix.get(a) {
            Some(row) => row & (1 << b.min(LAYER_COUNT - 1)) != 0,
            None => true,
        }
    }

    /// Everything layer `index` meets, as the mask a collider's filter is built from.
    pub fn meets(&self, index: usize) -> u32 {
        let Some(matrix) = self.matrix.as_ref() else {
            return u32::MAX;
        };
        matrix.get(index).copied().unwrap_or(u32::MAX)
    }

    /// Everything a collider on `mask`'s layers meets: the union of their rows.
    ///
    /// A collider belongs to as many layers as it is ticked into, so it meets whatever any of them
    /// meets. An empty mask meets nothing, which is what unticking every box says.
    pub fn met_by(&self, mask: u32) -> u32 {
        if mask == 0 {
            return 0;
        }
        let Some(matrix) = self.matrix.as_ref() else {
            return u32::MAX;
        };
        (0..LAYER_COUNT)
            .filter(|index| mask & (1 << index) != 0)
            .map(|index| matrix.get(index).copied().unwrap_or(u32::MAX))
            .fold(0, |met, row| met | row)
    }

    /// Sets a pair both ways: the table is symmetric, and half of it is a table that disagrees with
    /// itself.
    pub fn set_collide(&mut self, a: usize, b: usize, collide: bool) {
        if a >= LAYER_COUNT || b >= LAYER_COUNT {
            return;
        }
        let matrix = self
            .matrix
            .get_or_insert_with(|| vec![u32::MAX; LAYER_COUNT]);
        if matrix.len() < LAYER_COUNT {
            matrix.resize(LAYER_COUNT, u32::MAX);
        }
        for (from, to) in [(a, b), (b, a)] {
            match collide {
                true => matrix[from] |= 1 << to,
                false => matrix[from] &= !(1 << to),
            }
        }
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
