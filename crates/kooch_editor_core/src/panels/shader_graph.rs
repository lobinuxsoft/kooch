//! Shader Graph panel — the nodes, and the file they write (#1159).

use egui::emath::TSTransform;
use egui_snarl::ui::{PinInfo, SnarlViewer, SnarlWidget};
use egui_snarl::{InPin, NodeId, OutPin, Snarl};

use crate::shader_graph::{
    BLEND_MODES, Category, Graph, Node, TEXTURE_FALLBACKS, arrange, palette,
};

/// What the panel needs to draw one frame.
pub(crate) struct ShaderGraphView<'a> {
    /// The open graph, edited in place: `egui-snarl` moves nodes and wires while it draws them.
    pub graph: Option<&'a mut Graph>,
    /// The file it generates, for the header.
    pub path: Option<&'a std::path::Path>,
    /// Whether the graph diverges from that file.
    pub dirty: bool,
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
        }
        let mut showing = minimap_shown(ui);
        if ui.checkbox(&mut showing, "Minimap").changed() {
            show_minimap(ui, showing);
        }
        ui.weak("Right-click the background to add a node.");
    });
    ui.separator();

    // 🔴 The graph follows its panel. `egui-snarl` keeps the view in GLOBAL screen coordinates and
    // re-anchors it never, so a panel that moved left its graph behind — off screen, with nothing
    // to do but pan blindly looking for it (#1167).
    let panel = ui.max_rect();
    let mut viewer = Viewer {
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

/// How the nodes draw and connect.
struct Viewer {
    /// How far the panel moved since the last frame.
    drift: egui::Vec2,
    /// Where the minimap asked to look, in graph space.
    look_at: Option<egui::Pos2>,
    /// The panel, for centring a jump.
    panel: egui::Rect,
    /// This frame's view transform, taken back out for the minimap to draw what is on screen.
    transform: TSTransform,
}

impl SnarlViewer<Node> for Viewer {
    /// The one hook that runs between the pan/zoom handling and the transform being stored, which
    /// is the only place a view can be corrected from outside the widget.
    fn current_transform(&mut self, to_global: &mut TSTransform, _snarl: &mut Snarl<Node>) {
        to_global.translation += self.drift;
        if let Some(at) = self.look_at {
            // That point of the graph, under the middle of the panel.
            to_global.translation =
                self.panel.center().to_vec2() - to_global.scaling * at.to_vec2();
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
        usize::from(node.has_output())
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
        ui.label(name);
        PinInfo::circle()
    }

    fn show_output(
        &mut self,
        pin: &OutPin,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) -> impl egui_snarl::ui::SnarlPin + 'static {
        // A value node carries its own value, so the node itself is where it is edited.
        if let Some(node) = snarl.get_node_mut(pin.id.node) {
            match node {
                Node::Constant(value) => {
                    ui.horizontal(|ui| {
                        for component in value.iter_mut() {
                            ui.add(crate::numeric::drag(component).speed(0.01));
                        }
                    });
                }
                Node::Param {
                    name,
                    width,
                    color,
                    default,
                } => {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(name).desired_width(70.0));
                        ui.add(egui::DragValue::new(width).range(1..=4).prefix("x"));
                        // Only four wide: the hint it writes is a `vec4<f32>` one, and it cannot
                        // outlive a width the user narrowed under it.
                        *color &= *width == 4;
                        if *width == 4 {
                            ui.checkbox(color, "color");
                        }
                    });
                    ui.horizontal(|ui| {
                        for component in default.iter_mut().take(*width as usize) {
                            ui.add(crate::numeric::drag(component).speed(0.01));
                        }
                    });
                }
                Node::Texture { name, fallback } => {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(name).desired_width(70.0));
                        for option in TEXTURE_FALLBACKS {
                            if ui.selectable_label(fallback == option, option).clicked() {
                                *fallback = option.to_owned();
                            }
                        }
                    });
                }
                Node::Swizzle { pattern } => {
                    ui.add(
                        egui::TextEdit::singleline(pattern)
                            .desired_width(50.0)
                            .hint_text("xyzw"),
                    );
                }
                Node::Blend { mode } => {
                    ui.horizontal(|ui| {
                        for option in BLEND_MODES {
                            if ui.selectable_label(mode == option, option).clicked() {
                                *mode = option.to_owned();
                            }
                        }
                    });
                }
                _ => {
                    ui.label("out");
                }
            }
        }
        PinInfo::circle()
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
