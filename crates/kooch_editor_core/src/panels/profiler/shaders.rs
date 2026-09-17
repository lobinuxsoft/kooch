//! What each shader costs the GPU, per frame (#1159): the materials a scene is slow because of,
//! named, instead of one "shade" row in the flamegraph.

use std::path::Path;

use egui::Ui;

use crate::panels::inspector::AssetCatalogEntry;

/// The handheld's frame, the budget every shader is read against.
const FRAME_MS: f32 = 13.9;

pub(super) fn draw(ui: &mut Ui, costs: &[(String, f32)]) {
    egui::CollapsingHeader::new("Shaders (GPU)")
        .default_open(true)
        .show(ui, |ui| {
            if costs.is_empty() {
                ui.weak("No shader measured: nothing on screen, or this GPU cannot time passes.");
                return;
            }
            egui::Grid::new("shader_costs")
                .num_columns(3)
                .striped(true)
                .show(ui, |ui| {
                    for (name, ms) in costs {
                        ui.label(name);
                        ui.label(format!("{ms:.2} ms"));
                        ui.add(
                            egui::ProgressBar::new((ms / FRAME_MS).clamp(0.0, 1.0))
                                .desired_width(120.0)
                                .text(format!("{:.0}% of 13.9 ms", ms / FRAME_MS * 100.0)),
                        );
                        ui.end_row();
                    }
                });
        });
    ui.separator();
}

/// What the shader at `path` cost the GPU last frame, across every material using it. The catalog
/// holds project-relative paths and the open graph an absolute one, hence the suffix match.
pub(crate) fn shader_cost(
    catalog: &[AssetCatalogEntry],
    costs: &[(String, f32)],
    path: &Path,
) -> Option<f32> {
    let entry = catalog.iter().find(|e| path.ends_with(&e.path))?;
    let label = kooch_render::meshlet::shader_scope(Some(entry.guid));
    costs
        .iter()
        .find(|(scope, _)| *scope == label)
        .map(|(_, ms)| *ms)
}

/// The costs under their file names, most expensive first.
pub(crate) fn named_shader_costs(
    catalog: &[AssetCatalogEntry],
    costs: &[(String, f32)],
) -> Vec<(String, f32)> {
    let mut named: Vec<(String, f32)> = costs
        .iter()
        .map(|(scope, ms)| {
            let name = catalog
                .iter()
                .find(|e| kooch_render::meshlet::shader_scope(Some(e.guid)) == *scope)
                .map_or_else(
                    || scope.trim_start_matches("shader ").to_owned(),
                    |e| e.display_name.clone(),
                );
            (name, *ms)
        })
        .collect();
    named.sort_by(|a, b| b.1.total_cmp(&a.1));
    named
}

#[cfg(test)]
mod tests;
