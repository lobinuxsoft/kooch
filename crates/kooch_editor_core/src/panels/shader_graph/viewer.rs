//! How the nodes draw, connect and are edited inside egui-snarl (#1159).

use egui::emath::TSTransform;
use egui_snarl::ui::SnarlViewer;
use egui_snarl::{InPin, NodeId, OutPin, Snarl};

use super::editors::{FIELD, choice, components, name_field, number_editor};
use super::framed;
use super::pin::{NamedPin, Side, reserve};
use crate::panels::inspector::AssetCatalogEntry;
use crate::shader_graph::{
    BLEND_MODES, Category, NOISE_BASES, NOISE_FRACTALS, Node, TEXTURE_FALLBACKS, VORONOI_METRICS,
    palette,
};

/// How the nodes draw and connect.
pub(super) struct Viewer<'a> {
    /// The whole graph's bounds, when the view should be framed around them this frame.
    pub(super) fit: Option<egui::Rect>,
    /// Every asset, for a texture node's preview image.
    pub(super) catalog: &'a [AssetCatalogEntry],
    /// How far the panel moved since the last frame.
    pub(super) drift: egui::Vec2,
    /// Where the minimap asked to look, in graph space.
    pub(super) look_at: Option<egui::Pos2>,
    /// The panel, for centring a jump.
    pub(super) panel: egui::Rect,
    /// This frame's view transform, taken back out for the minimap to draw what is on screen.
    pub(super) transform: TSTransform,
}

impl SnarlViewer<Node> for Viewer<'_> {
    /// The one hook that runs between the pan/zoom handling and the transform being stored, which
    /// is the only place a view can be corrected from outside the widget.
    fn current_transform(&mut self, to_global: &mut TSTransform, _snarl: &mut Snarl<Node>) {
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

    fn title(&mut self, node: &Node) -> String {
        node.title()
    }

    fn inputs(&mut self, node: &Node) -> usize {
        node.inputs().len()
    }

    fn outputs(&mut self, node: &Node) -> usize {
        node.outputs().len()
    }

    fn show_input(
        &mut self,
        pin: &InPin,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) -> impl egui_snarl::ui::SnarlPin + 'static {
        let name = snarl
            .get_node(pin.id.node)
            .and_then(|node| node.inputs().get(pin.id.input).copied())
            .unwrap_or("in");
        reserve(ui, name);
        NamedPin {
            name: Some(name),
            side: Side::Input,
        }
    }

    fn show_output(
        &mut self,
        pin: &OutPin,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) -> impl egui_snarl::ui::SnarlPin + 'static {
        let output = pin.id.output;
        let name = snarl
            .get_node(pin.id.node)
            .and_then(|node| node.outputs().get(output))
            .map_or("out", |&(name, _)| name);
        reserve(ui, name);
        NamedPin {
            name: Some(name),
            side: Side::Output,
        }
    }

    // 🔴 Fields go under the pins, not beside the first output: beside it, that pin's name had to sit
    // under the fields, level with nothing.
    fn has_footer(&mut self, node: &Node) -> bool {
        has_fields(node)
    }

    fn show_footer(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) {
        // A value node carries its own value, so the node itself is where it is edited.
        let catalog = self.catalog;
        let Some(node) = snarl.get_node_mut(node) else {
            return;
        };
        // Capped: a top-down layout claims all the width it is offered, and inside a snarl node that
        // is everything up to the panel's edge.
        let column = egui::vec2(FIELD + 16.0, 0.0);
        let top_down = egui::Layout::top_down(egui::Align::Min);
        ui.allocate_ui_with_layout(column, top_down, |ui| {
            match node {
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
                } => {
                    name_field(ui, name);
                    components(ui, &mut default[..(*width).clamp(2, 4) as usize]);
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
                _ => {}
            }
        });
    }

    /// One wire per input: a second one replaces the first, which is what every graph tool does.
    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<Node>) {
        for &remote in &to.remotes {
            snarl.disconnect(remote, to.id);
        }
        snarl.connect(from.id, to.id);
    }

    fn has_graph_menu(&mut self, _pos: egui::Pos2, _snarl: &mut Snarl<Node>) -> bool {
        true
    }

    /// Grouped, because a flat list of forty nodes is a list nobody reads.
    fn show_graph_menu(&mut self, pos: egui::Pos2, ui: &mut egui::Ui, snarl: &mut Snarl<Node>) {
        ui.label("Add node");
        for category in Category::ALL {
            ui.menu_button(category.label(), |ui| {
                for node in palette()
                    .into_iter()
                    .filter(|node| node.category() == category)
                {
                    if ui.button(node.title()).clicked() {
                        snarl.insert_node(pos, node);
                        ui.close();
                    }
                }
            });
        }
    }

    fn has_node_menu(&mut self, _node: &Node) -> bool {
        true
    }

    fn show_node_menu(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) {
        if ui
            .button(format!("{} Remove", crate::icons::TRASH))
            .clicked()
        {
            snarl.remove_node(node);
            ui.close();
        }
    }
}

/// Whether a node draws fields of its own beside its first output.
fn has_fields(node: &Node) -> bool {
    matches!(
        node,
        Node::Float { .. }
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
    )
}
