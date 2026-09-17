//! Shader Graph panel — the nodes, and the file they write (#1159).

use egui::emath::TSTransform;
use egui_snarl::ui::SnarlWidget;

use crate::panels::inspector::AssetCatalogEntry;

mod editors;
mod pin;
mod preview;
mod viewer;

use crate::shader_graph::{Graph, arrange};
use preview::draw_preview;
use viewer::Viewer;

/// What the panel needs to draw one frame.
pub(crate) struct ShaderGraphView<'a> {
    /// The open graph, edited in place: `egui-snarl` moves nodes and wires while it draws them.
    pub graph: Option<&'a mut Graph>,
    /// The file it generates, for the header.
    pub path: Option<&'a std::path::Path>,
    /// Whether the graph diverges from that file.
    pub dirty: bool,
    /// The shader on a shape, beside the nodes that wrote it.
    pub preview: PreviewView<'a>,
    /// Every asset, for a texture node to pick the image its preview samples.
    pub catalog: &'a [AssetCatalogEntry],
    /// What the saved shader cost the GPU last frame, across every material using it.
    pub cost_ms: Option<f32>,
}

/// The preview column: what to draw, on what, and how to ask for something else.
pub(crate) struct PreviewView<'a> {
    pub texture: egui::TextureId,
    /// Which of `Primitive::CANONICAL` is showing.
    pub primitive: usize,
    /// Why the shader did not build, if it did not.
    pub refusal: Option<&'a str>,
    /// The shape the panel wants next frame. `Some` also means the panel was drawn at all, which
    /// is what keeps the preview from rendering behind a tab nobody opened.
    pub request: &'a mut Option<crate::viewport::PreviewRequest>,
}

/// What the panel asks for.
pub(crate) enum ShaderGraphAction {
    /// Generate the shader and write it.
    Save,
}

/// Draws the panel. Returns what the user asked for.
pub(crate) fn draw_shader_graph_content(
    ui: &mut egui::Ui,
    view: ShaderGraphView<'_>,
) -> Vec<ShaderGraphAction> {
    let mut actions = Vec::new();
    let Some(graph) = view.graph else {
        ui.weak("No shader graph open.");
        ui.label("Create one in the Asset Browser: New Shader Graph, or open a generated .shader.");
        return actions;
    };
    draw_preview(ui, view.preview);

    let mut refit = false;
    ui.horizontal(|ui| {
        if let Some(path) = view.path {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            ui.label(format!("{name}{}", if view.dirty { " *" } else { "" }));
        }
        if ui
            .button("Save")
            .on_hover_text("Generate the .shader from this graph and write it")
            .clicked()
        {
            actions.push(ShaderGraphAction::Save);
        }
        if ui
            .button("Arrange")
            .on_hover_text("Lay the nodes out left to right, in layers")
            .clicked()
        {
            arrange(graph);
            refit = true;
        }
        if ui
            .button("Fit")
            .on_hover_text("Frame the whole graph in the panel")
            .clicked()
        {
            refit = true;
        }
        let mut showing = minimap_shown(ui);
        if ui.checkbox(&mut showing, "Minimap").changed() {
            show_minimap(ui, showing);
        }
        cost(ui, graph, view.cost_ms);
        ui.weak("Right-click the background to add a node.");
    });
    ui.separator();

    // 🔴 The graph follows its panel. `egui-snarl` keeps the view in GLOBAL screen coordinates and
    // re-anchors it never, so a panel that moved left its graph behind — off screen, with nothing
    // to do but pan blindly looking for it (#1167).
    let panel = ui.max_rect();
    // Framed on the frames right after a graph opens, while its window settles on a size, and on request.
    let fit = (refit || opening(ui, view.path))
        .then(|| crate::panels::graph_minimap::bounds(graph))
        .flatten();
    let mut viewer = Viewer {
        fit,
        catalog: view.catalog,
        drift: drift(ui, panel),
        look_at: taken_look(ui),
        panel,
        transform: TSTransform::IDENTITY,
    };

    SnarlWidget::new()
        .id(egui::Id::new("shader_graph"))
        .show(graph, &mut viewer, ui);

    if minimap_shown(ui)
        && let Some(at) = crate::panels::graph_minimap::draw(ui, panel, graph, viewer.transform)
    {
        look_at(ui, at);
    }
    actions
}

/// The view that shows all of `bounds` centred in `panel`: zoomed out far enough to hold every node,
/// never zoomed in past actual size.
fn framed(bounds: egui::Rect, panel: egui::Rect) -> TSTransform {
    let scaling = (panel.width() / bounds.width())
        .min(panel.height() / bounds.height())
        .clamp(0.2, 1.0);
    TSTransform {
        scaling,
        translation: panel.center().to_vec2() - scaling * bounds.center().to_vec2(),
    }
}

/// Whether this is one of the first frames showing the graph at `path`. The window a graph opens in
/// takes a frame or two to settle on its size, and a view framed before that is framed wrong.
fn opening(ui: &egui::Ui, path: Option<&std::path::Path>) -> bool {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    let key = hasher.finish();
    let (seen, frames) = ui
        .ctx()
        .data(|d| d.get_temp::<(u64, u8)>(opened_id()))
        .unwrap_or((0, u8::MAX));
    let frames = if seen == key {
        frames.saturating_add(1)
    } else {
        0
    };
    ui.ctx()
        .data_mut(|d| d.insert_temp(opened_id(), (key, frames)));
    frames < 3
}

fn opened_id() -> egui::Id {
    egui::Id::new("shader_graph_opened")
}

/// Forgets which graph was on screen, so the next time the panel is drawn it frames the graph again —
/// called on a frame the panel was not drawn at all.
pub(crate) fn forget_opening(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove_temp::<(u64, u8)>(opened_id()));
}

/// How far the panel moved since the last frame — what the view has to travel to stay with it.
fn drift(ui: &egui::Ui, panel: egui::Rect) -> egui::Vec2 {
    let id = egui::Id::new("shader_graph_panel_rect");
    let before = ui.ctx().data(|d| d.get_temp::<egui::Rect>(id));
    ui.ctx().data_mut(|d| d.insert_temp(id, panel));
    match before {
        Some(before) if before.is_finite() && panel.is_finite() => panel.min - before.min,
        _ => egui::Vec2::ZERO,
    }
}

/// Whether the minimap is showing, and where the user asked to look. Both live in egui's own store:
/// how a graph is being *viewed* is not part of the graph, and must never reach the file.
fn minimap_shown(ui: &egui::Ui) -> bool {
    ui.ctx()
        .data(|d| d.get_temp::<bool>(egui::Id::new("shader_graph_minimap")))
        .unwrap_or(true)
}

fn show_minimap(ui: &egui::Ui, showing: bool) {
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new("shader_graph_minimap"), showing));
}

fn look_at(ui: &egui::Ui, at: egui::Pos2) {
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new("shader_graph_look_at"), at));
}

/// Takes the pending jump, so it is applied once and not every frame after.
fn taken_look(ui: &egui::Ui) -> Option<egui::Pos2> {
    let id = egui::Id::new("shader_graph_look_at");
    ui.ctx().data_mut(|d| {
        let at = d.get_temp::<egui::Pos2>(id);
        d.remove_temp::<egui::Pos2>(id);
        at
    })
}

#[cfg(test)]
mod tests;

/// The graph's size and the saved shader's measured GPU time, against the handheld's frame.
fn cost(ui: &mut egui::Ui, graph: &Graph, ms: Option<f32>) {
    let nodes = graph.node_ids().count();
    let time = ms.map_or_else(|| "— ms".to_owned(), |ms| format!("{ms:.2} ms"));
    ui.label(format!("{nodes} nodes · {time} GPU"))
        .on_hover_text(
            "The last finished frame's GPU time shading this shader, summed over every material \
         using it and every view drawing it — the Edit and Game views both count. Read it against \
         a 13.9 ms handheld frame. It measures the saved file, not unsaved edits; \"—\" means \
         nothing on screen uses it, or this GPU cannot time passes.",
        );
}
