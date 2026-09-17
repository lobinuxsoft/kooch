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

/// The shader scopes of the latest frame a view holds, summed by label: what a game on the handheld
/// sent over the network, read the way the local editor reads its own `GpuScopes`.
#[cfg(feature = "profiling")]
pub fn shader_costs_in(view: &puffin::FrameView) -> Vec<(String, f32)> {
    let Some(unpacked) = view.latest_frame().and_then(|frame| frame.unpacked().ok()) else {
        return Vec::new();
    };
    let mut totals: Vec<(String, f32)> = Vec::new();
    for stream in unpacked.thread_streams.values() {
        add_shader_scopes(view, &stream.stream, 0, &mut totals);
    }
    totals
}

/// 🔴 Recursive: a `Reader` walks one level of siblings, and the shader scopes are nested in the
/// shading pass. Reading only the top level found none.
#[cfg(feature = "profiling")]
fn add_shader_scopes(
    view: &puffin::FrameView,
    stream: &puffin::Stream,
    offset: u64,
    totals: &mut Vec<(String, f32)>,
) {
    let Ok(reader) = puffin::Reader::with_offset(stream, offset) else {
        return;
    };
    for scope in reader.flatten() {
        if let Some(details) = view.scope_collection().fetch_by_id(&scope.id) {
            let name = details.name();
            if name.starts_with("shader ") {
                let ms = scope.record.duration_ns as f32 / 1e6;
                match totals.iter_mut().find(|(label, _)| label == name) {
                    Some((_, total)) => *total += ms,
                    None => totals.push((name.to_string(), ms)),
                }
            }
        }
        add_shader_scopes(view, stream, scope.child_begin_position, totals);
    }
}

#[cfg(test)]
mod tests;
