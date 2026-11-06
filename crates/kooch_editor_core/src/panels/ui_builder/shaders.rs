//! Which shaders a node can wear: the catalog types every shader alike, `kind: ui` is in the file.

use std::collections::HashSet;
use std::sync::Arc;

use kooch_core::Guid;
use kooch_render::material::{SHADER_TYPE_NAME, Shader, ShaderKind};

use crate::panels::inspector::AssetCatalogEntry;

/// Every `kind: ui` shader in `catalog`. Read again every couple of seconds: the field is drawn
/// each frame and the answer is in files on disk.
pub(super) fn ui_shaders(ctx: &egui::Context, catalog: &[AssetCatalogEntry]) -> Arc<HashSet<Guid>> {
    type Cached = (f64, Arc<HashSet<Guid>>);
    const REFRESH: f64 = 2.0;
    let id = egui::Id::new("ui_builder_shaders");
    let now = ctx.input(|input| input.time);
    if let Some((at, usable)) = ctx.data(|d| d.get_temp::<Cached>(id))
        && now - at < REFRESH
    {
        return usable;
    }
    let usable: HashSet<Guid> = catalog
        .iter()
        .filter(|entry| entry.type_name == SHADER_TYPE_NAME)
        .filter(|entry| std::fs::read_to_string(&entry.path).is_ok_and(|text| is_ui(&text)))
        .map(|entry| entry.guid)
        .collect();
    let usable = Arc::new(usable);
    ctx.data_mut(|d| d.insert_temp::<Cached>(id, (now, usable.clone())));
    usable
}

fn is_ui(source: &str) -> bool {
    Shader::parse(source).is_ok_and(|shader| shader.kind == ShaderKind::Ui)
}
