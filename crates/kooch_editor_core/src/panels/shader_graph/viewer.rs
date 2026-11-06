//! How the nodes draw, connect and are edited inside egui-snarl (#1159).

use egui::emath::TSTransform;
use egui_snarl::ui::SnarlViewer;
use egui_snarl::{InPin, NodeId, OutPin, Snarl};

use super::editors::{FIELD, choice, components, name_field, number_editor, pin_field};
use super::framed;
use super::pin::{NamedPin, Side, label, reserve};
use crate::panels::inspector::AssetCatalogEntry;
use crate::shader_graph::subgraph::{call_node, is_subgraph, relative};
use crate::shader_graph::{
    BLEND_MODES, Category, GraphNode, NOISE_BASES, NOISE_FRACTALS, Node, OUTPUT_KINDS,
    TEXTURE_FALLBACKS, VORONOI_METRICS, Width, functions, palette,
};

/// How the nodes draw and connect.
pub(super) struct Viewer<'a> {
    /// The whole graph's bounds, when the view should be framed around them this frame.
    pub(super) fit: Option<egui::Rect>,
    /// Every asset, for a texture node's preview image and the subgraphs a graph can call.
    pub(super) catalog: &'a [AssetCatalogEntry],
    /// The file the graph generates.
    pub(super) path: Option<&'a std::path::Path>,
    /// How far the panel moved since the last frame.
    pub(super) drift: egui::Vec2,
    /// Where the minimap asked to look, in graph space.
    pub(super) look_at: Option<egui::Pos2>,
    /// The panel, for centring a jump.
    pub(super) panel: egui::Rect,
    /// This frame's view transform, taken back out for the minimap to draw what is on screen.
    pub(super) transform: TSTransform,
    /// Where the menu asked for a note, in graph space; the panel adds it after the widget.
    pub(super) new_note: Option<egui::Pos2>,
    /// A node its menu took out of its group; the panel applies it after the widget.
    pub(super) ungroup: Option<NodeId>,
}

impl SnarlViewer<GraphNode> for Viewer<'_> {
    /// The one hook that runs between the pan/zoom handling and the transform being stored, which
    /// is the only place a view can be corrected from outside the widget.
    fn current_transform(&mut self, to_global: &mut TSTransform, _snarl: &mut Snarl<GraphNode>) {
        to_global.translation += self.drift;
        if let Some(at) = self.look_at {
            // That point of the graph, under the middle of the panel.
            to_global.translation =
                self.panel.center().to_vec2() - to_global.scaling * at.to_vec2();
        }
        if let Some(bounds) = self.fit {
            *to_global = framed(bounds, self.panel);
        }
        self.transform = *to_global;
    }

    fn title(&mut self, node: &GraphNode) -> String {
        node.title()
    }

    /// Tinted by category, so a graph reads at a glance: inputs blue, noise violet, output red.
    fn header_frame(
        &mut self,
        default: egui::Frame,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        snarl: &Snarl<GraphNode>,
    ) -> egui::Frame {
        match snarl.get_node(node) {
            Some(node) => default.fill(tint(node.category())),
            None => default,
        }
    }

    fn has_on_hover_popup(&mut self, _node: &GraphNode) -> bool {
        true
    }

    fn show_on_hover_popup(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<GraphNode>,
    ) {
        if let Some(node) = snarl.get_node(node) {
            ui.label(node.about());
        }
    }

    fn inputs(&mut self, node: &GraphNode) -> usize {
        node.inputs().len()
    }

    fn outputs(&mut self, node: &GraphNode) -> usize {
        node.outputs().len()
    }

    fn show_input(
        &mut self,
        pin: &InPin,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<GraphNode>,
    ) -> impl egui_snarl::ui::SnarlPin + 'static {
        let (name, (width, about)) = snarl
            .get_node(pin.id.node)
            .and_then(|node| {
                let name = *node.inputs().get(pin.id.input)?;
                let doc = node.input_docs().get(pin.id.input).copied();
                Some((name, doc.unwrap_or((node.pin_width(pin.id.input), ""))))
            })
            .unwrap_or(("in", (Width::Any, "")));
        let label = label(name, width);
        reserve(ui, &label);
        if pin.remotes.is_empty()
            && let Some(cell) = snarl.get_node_mut(pin.id.node)
        {
            pin_field(ui, cell, pin.id, width);
        }
        NamedPin {
            label,
            side: Side::Input,
            width,
            about,
            id: egui::Id::new(("pin", pin.id)),
        }
    }

    fn show_output(
        &mut self,
        pin: &OutPin,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<GraphNode>,
    ) -> impl egui_snarl::ui::SnarlPin + 'static {
        let output = pin.id.output;
        let (name, (width, about)) = snarl
            .get_node(pin.id.node)
            .and_then(|node| Some((node.outputs().get(output)?.0, node.output_doc(output))))
            .unwrap_or(("out", (Width::Any, "")));
        let label = label(name, width);
        reserve(ui, &label);
        NamedPin {
            label,
            side: Side::Output,
            width,
            about,
            id: egui::Id::new(("pin", pin.id)),
        }
    }

    // 🔴 Fields go under the pins, not beside the first output: beside it, that pin's name had to sit
    // under the fields, level with nothing.
    fn has_footer(&mut self, node: &GraphNode) -> bool {
        has_fields(node)
    }

    fn show_footer(
        &mut self,
        id: NodeId,
        inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<GraphNode>,
    ) {
        // A value node carries its own value, so the node itself is where it is edited.
        let catalog = self.catalog;
        let Some(node) = snarl.get_node_mut(id) else {
            return;
        };
        // Capped: a top-down layout claims all the width it is offered, and inside a snarl node that
        // is everything up to the panel's edge.
        let column = egui::vec2(FIELD + 16.0, 0.0);
        let top_down = egui::Layout::top_down(egui::Align::Min);
        ui.allocate_ui_with_layout(column, top_down, |ui| {
            match &mut node.node {
                Node::Argument { name } | Node::Result { name } => name_field(ui, name),
                Node::Float {
                    name,
                    default,
                    range,
                } => number_editor(ui, name, default, range, false),
                Node::Int {
                    name,
                    default,
                    range,
                } => number_editor(ui, name, default, range, true),
                Node::Vector {
                    name,
                    width,
                    default,
                    point,
                } => {
                    name_field(ui, name);
                    components(ui, &mut default[..(*width).clamp(2, 4) as usize]);
                    if *width == 2 {
                        ui.checkbox(point, "Point")
                            .on_hover_text("A position across the node, dragged in the UI Builder");
                    }
                }
                Node::Color { name, default } => {
                    name_field(ui, name);
                    ui.color_edit_button_rgba_unmultiplied(default);
                }
                Node::Texture {
                    name,
                    fallback,
                    preview,
                } => {
                    name_field(ui, name);
                    choice(ui, "fallback", fallback, &TEXTURE_FALLBACKS);
                    // Seen in the preview only; a material still starts from `fallback`.
                    ui.weak("preview");
                    let picked = ui
                        .push_id("preview", |ui| {
                            crate::panels::inspector::draw_asset_picker(
                                ui,
                                *preview,
                                crate::panels::inspector::IMAGE_TYPE,
                                catalog,
                            )
                        })
                        .inner;
                    if let Some(kooch_ecs::reflect::ReflectValue::AssetRef { guid, .. }) = picked {
                        *preview = guid;
                    }
                }
                Node::ConstFloat(value) => {
                    ui.add(crate::numeric::drag(value).speed(0.01));
                }
                Node::ConstInt(value) => {
                    ui.add(egui::DragValue::new(value).speed(1.0).fixed_decimals(0));
                    *value = value.round();
                }
                Node::ConstVector { width, value } => {
                    components(ui, &mut value[..(*width).clamp(2, 4) as usize]);
                }
                Node::Constant(value) => components(ui, value),
                Node::ConstColor(value) => {
                    ui.color_edit_button_rgba_unmultiplied(value);
                }
                Node::Swizzle { pattern } => {
                    ui.add(
                        egui::TextEdit::singleline(pattern)
                            .desired_width(FIELD)
                            .hint_text("xyzw"),
                    );
                }
                Node::Blend { mode } => choice(ui, "blend", mode, &BLEND_MODES),
                Node::FractalNoise { basis, fractal } => {
                    choice(ui, "basis", basis, &NOISE_BASES);
                    choice(ui, "fractal", fractal, &NOISE_FRACTALS);
                }
                Node::VoronoiNoise { metric } => choice(ui, "metric", metric, &VORONOI_METRICS),
                Node::ShaderOutput { kind } => choice(ui, "kind", kind, &OUTPUT_KINDS),
                _ => {}
            }
        });
        // A kind with fewer pins drops the wires and values of the ones it no longer has.
        let count = snarl.get_node(id).map_or(0, |n| n.inputs().len());
        if let Some(node) = snarl.get_node_mut(id) {
            node.pins.retain(|&input, _| input < count);
        }
        for pin in inputs.iter().filter(|pin| pin.id.input >= count) {
            for &remote in &pin.remotes {
                snarl.disconnect(remote, pin.id);
            }
        }
    }

    /// One wire per input: a second one replaces the first, which is what every graph tool does.
    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<GraphNode>) {
        for &remote in &to.remotes {
            snarl.disconnect(remote, to.id);
        }
        snarl.connect(from.id, to.id);
    }

    fn has_graph_menu(&mut self, _pos: egui::Pos2, _snarl: &mut Snarl<GraphNode>) -> bool {
        true
    }

    /// Grouped, because a flat list of forty nodes is a list nobody reads.
    fn show_graph_menu(
        &mut self,
        pos: egui::Pos2,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<GraphNode>,
    ) {
        if ui.button("Add note").clicked() {
            self.new_note = Some(pos);
            ui.close();
        }
        ui.separator();
        ui.label("Add node");
        let in_subgraph = self.path.is_some_and(is_subgraph);
        for category in Category::ALL {
            let nodes: Vec<Node> = palette()
                .into_iter()
                .filter(|node| node.category() == category && node.offered(in_subgraph))
                .collect();
            if nodes.is_empty() {
                continue;
            }
            ui.menu_button(category.label(), |ui| {
                for node in nodes {
                    if ui
                        .button(node.title())
                        .on_hover_text(node.about())
                        .clicked()
                    {
                        snarl.insert_node(pos, node.into());
                        ui.close();
                    }
                }
            });
        }
        self.subgraph_menu(pos, ui, snarl);
        self.function_menu(pos, ui, snarl);
    }

    fn has_node_menu(&mut self, _node: &GraphNode) -> bool {
        true
    }

    fn show_node_menu(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<GraphNode>,
    ) {
        if ui.button("Remove from group").clicked() {
            self.ungroup = Some(node);
            ui.close();
        }
        if ui
            .button(format!("{} Remove", crate::icons::TRASH))
            .clicked()
        {
            snarl.remove_node(node);
            ui.close();
        }
    }
}

impl Viewer<'_> {
    /// The engine's library and every documented function of the project's `.wgsl` files.
    fn function_menu(&self, pos: egui::Pos2, ui: &mut egui::Ui, snarl: &mut Snarl<GraphNode>) {
        let folder = self.path.and_then(std::path::Path::parent);
        let offered = offered_functions(ui.ctx(), folder, self.catalog);
        ui.menu_button("Functions", |ui| {
            for (node, about) in offered.iter() {
                if ui.button(node.title()).on_hover_text(about).clicked() {
                    snarl.insert_node(pos, node.clone().into());
                    ui.close();
                }
            }
        });
    }

    /// The project's subgraphs, each a node calling it. Never the open file: a function calling
    /// itself does not compile.
    fn subgraph_menu(&self, pos: egui::Pos2, ui: &mut egui::Ui, snarl: &mut Snarl<GraphNode>) {
        let Some(open) = self.path else {
            return;
        };
        let mut files: Vec<_> = self
            .catalog
            .iter()
            .filter(|entry| is_subgraph(&entry.path) && entry.path != open)
            .collect();
        if files.is_empty() {
            return;
        }
        files.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        for (label, looped) in [("Subgraphs", false), ("Loop over", true)] {
            ui.menu_button(label, |ui| {
                for entry in &files {
                    if !ui.button(&entry.display_name).clicked() {
                        continue;
                    }
                    let folder = open.parent().unwrap_or(std::path::Path::new(""));
                    let path = relative(folder, &entry.path);
                    let node = std::fs::read_to_string(&entry.path)
                        .ok()
                        .and_then(|source| call_node(&entry.path, &source, path));
                    match node {
                        Some(node) if looped => {
                            snarl.insert_node(pos, node.looped().into());
                        }
                        Some(node) => {
                            snarl.insert_node(pos, node.into());
                        }
                        None => tracing::error!(
                            file = %entry.path.display(),
                            "this subgraph could not be read",
                        ),
                    }
                    ui.close();
                }
            });
        }
    }
}

/// What the menu lists, read again every couple of seconds: it is drawn every frame it is open, and
/// the project's files are on disk.
fn offered_functions(
    ctx: &egui::Context,
    folder: Option<&std::path::Path>,
    catalog: &[AssetCatalogEntry],
) -> std::sync::Arc<Vec<(Node, String)>> {
    type Cached = (f64, std::sync::Arc<Vec<(Node, String)>>);
    const REFRESH: f64 = 2.0;
    let id = egui::Id::new("shader_graph_functions");
    let now = ctx.input(|input| input.time);
    if let Some((at, offered)) = ctx.data(|d| d.get_temp::<Cached>(id))
        && now - at < REFRESH
    {
        return offered;
    }
    let mut offered: Vec<(Node, String)> = kooch_core::shader_uses::LIBRARY
        .iter()
        .flat_map(|(name, text)| functions(name, text))
        .collect();
    if let Some(folder) = folder {
        let hand_written = catalog.iter().filter(|entry| {
            let extension = entry.path.extension();
            extension.is_some_and(|e| e == kooch_render::material::SHADER_EXTENSION)
        });
        for entry in hand_written {
            if let Ok(text) = std::fs::read_to_string(&entry.path) {
                offered.extend(functions(&relative(folder, &entry.path), &text));
            }
        }
    }
    let offered = std::sync::Arc::new(offered);
    ctx.data_mut(|d| d.insert_temp::<Cached>(id, (now, offered.clone())));
    offered
}

/// Whether a node draws fields of its own beside its first output.
fn has_fields(node: &Node) -> bool {
    matches!(
        node,
        Node::Argument { .. }
            | Node::Result { .. }
            | Node::Float { .. }
            | Node::FractalNoise { .. }
            | Node::VoronoiNoise { .. }
            | Node::Int { .. }
            | Node::Vector { .. }
            | Node::Color { .. }
            | Node::Texture { .. }
            | Node::ConstFloat(_)
            | Node::ConstInt(_)
            | Node::ConstVector { .. }
            | Node::Constant(_)
            | Node::ConstColor(_)
            | Node::Swizzle { .. }
            | Node::Blend { .. }
            | Node::ShaderOutput { .. }
    )
}

/// Dark enough for the title's light text on every one.
fn tint(category: Category) -> egui::Color32 {
    let [r, g, b] = match category {
        Category::Input => [0x3A, 0x5A, 0x8C],
        Category::Constant => [0x2F, 0x6F, 0x6F],
        Category::Math => [0x4F, 0x5B, 0x66],
        Category::Vector => [0x6B, 0x5B, 0x2E],
        Category::Uv => [0x2E, 0x6B, 0x45],
        Category::Effect => [0x6B, 0x3A, 0x6B],
        Category::Shape => [0x7A, 0x4A, 0x2A],
        Category::Noise => [0x5A, 0x4A, 0x7A],
        Category::Ui => [0x2E, 0x6B, 0x4F],
        Category::Subgraph => [0x2F, 0x5F, 0x7A],
        Category::Output => [0x8C, 0x2F, 0x2F],
    };
    egui::Color32::from_rgb(r, g, b)
}
