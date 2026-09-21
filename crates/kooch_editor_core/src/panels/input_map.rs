//! Input Map panel — where bindings are configured.

use kooch_input::actions::{
    Action, ActionMap, Binding, BothHeld, Composite, ControlPath, ControlType, DeviceClass,
    PartName, Processor, Role, VectorMode,
};

use crate::icons;
use crate::widgets::SelectableRow;

/// What the panel needs to draw one frame.
pub(crate) struct InputMapView<'a> {
    /// The map being edited, if one is open.
    pub map: Option<&'a ActionMap>,
    /// Per-action live values, in the map's order, as reported by the
    /// host. Empty when nothing is playing.
    pub live: &'a [LiveAction],
    /// The binding waiting for a key, if a rebind is in progress.
    pub awaiting: Option<BindingAddress>,
    /// Whether the open map diverges from its file.
    pub dirty: bool,
    /// What the properties pane at the bottom is editing.
    pub selected: Option<Selection>,
    /// Whether the open document is a single `.inputaction`.
    pub single_action: bool,
}

/// What the host says an action is worth right now.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LiveAction {
    pub value: glam::Vec3,
    pub pressed: bool,
}

/// What the properties pane is editing. Unity splits this into four
/// views; one enum covers the same ground in a single column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Selection {
    Action(usize),
    Binding(BindingAddress),
}

/// Which binding a click was on. Positional rather than an id: a binding
/// has no identity of its own, and the edit is applied to the same list
/// the click came from, in the same frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct BindingAddress {
    pub action: usize,
    pub binding: usize,
}

/// Whose processor list an edit is aimed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ProcessorTarget {
    Binding(BindingAddress),
    Action(usize),
}

/// What the user did, for the caller to apply. Intent rather than
/// mutation: the map may live behind an asset handle, an undo stack or a
/// socket, and a panel that writes through all three knows all three.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum InputMapAction {
    /// Write the map back to its file.
    Save,
    /// Select an action or a binding — what the properties pane edits.
    Select(Selection),
    /// Rename an action — the name is what gameplay resolves, so this
    /// is the most consequential edit here.
    RenameAction {
        action: usize,
        name: String,
    },
    /// Change what an action produces.
    SetControlType {
        action: usize,
        control_type: ControlType,
    },
    /// Replace what this binding reads.
    Rebind {
        at: BindingAddress,
        path: ControlPath,
    },
    RemoveBinding(BindingAddress),
    AddBinding {
        action: usize,
    },
    /// Add a composite and one unbound part per name it declares.
    AddComposite {
        action: usize,
        composite: Composite,
    },
    /// Change a composite's parameters — its mode, or which side wins.
    SetComposite {
        at: BindingAddress,
        composite: Composite,
    },
    AddProcessor {
        to: ProcessorTarget,
        processor: Processor,
    },
    /// Replace one in place — how its parameters are edited.
    SetProcessor {
        to: ProcessorTarget,
        index: usize,
        processor: Processor,
    },
    RemoveProcessor {
        to: ProcessorTarget,
        index: usize,
    },
    /// Move one up (`-1`) or down (`+1`).
    MoveProcessor {
        to: ProcessorTarget,
        index: usize,
        delta: i32,
    },
    AddAction,
    RemoveAction {
        action: usize,
    },
}

/// Draws the panel. Returns what the user asked for.
pub(crate) fn draw_input_map_content(
    ui: &mut egui::Ui,
    view: InputMapView<'_>,
) -> Vec<InputMapAction> {
    let mut actions = Vec::new();

    let Some(map) = view.map else {
        ui.weak("No input map open.");
        ui.label("Create one in the asset browser: New → Input Map.");
        return actions;
    };

    // Wrapped so `Save` stays reachable in a narrow tab: edits live in
    // memory until it is pressed.
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("{} {}", icons::GAME_CONTROLLER, map.name));
        if !view.single_action {
            ui.weak(format!("priority {}", map.priority));
            if ui.button(format!("{} Action", icons::PLUS)).clicked() {
                actions.push(InputMapAction::AddAction);
            }
        }
        // Same contract a prefab has; the marker makes it visible.
        if ui
            .add_enabled(view.dirty, egui::Button::new("Save"))
            .on_hover_text("Write these bindings back to the file")
            .clicked()
        {
            actions.push(InputMapAction::Save);
        }
        if view.dirty {
            ui.weak("• unsaved");
        }
    });
    if view.live.is_empty() {
        ui.weak("Values appear while the game is playing.");
    }
    ui.separator();

    // Unity's third column. A column rather than a strip along the bottom: the strip's fixed height
    // came off the tree whether or not anything was selected, and the tree is the part that grows.
    let (default_width, width_range) = properties_column(ui.available_width());
    // `Panel::right` rather than `SidePanel`: egui 0.35 folded the four
    // side/top/bottom builders into one `Panel`, so the old name does not
    // resolve and `default_width`/`width_range` are now size-agnostic.
    egui::Panel::right("input_map_properties")
        .resizable(true)
        .default_size(default_width)
        .size_range(width_range)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .id_salt("input_map_properties")
                .show(ui, |ui| {
                    draw_properties(ui, map, &view, &mut actions);
                });
        });

    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt("input_map_actions")
            .show(ui, |ui| {
                for (index, action) in map.actions.iter().enumerate() {
                    draw_action(ui, index, action, &view, &mut actions);
                }
            });
    });

    actions
}

/// Starting width of the properties column and the range a drag may take it to, for a tab of
/// `tab_width`.
fn properties_column(tab_width: f32) -> (f32, std::ops::RangeInclusive<f32>) {
    (tab_width * 0.34, (tab_width * 0.25)..=(tab_width * 0.5))
}

/// A labelled control: side by side when there is room, stacked when there is not. `add` receives
/// the width to take.
fn labeled_control<R>(
    ui: &mut egui::Ui,
    label: &str,
    add: impl FnOnce(&mut egui::Ui, f32) -> R,
) -> R {
    let side_by_side = ui.text_style_height(&egui::TextStyle::Body) * 12.0;
    if ui.available_width() >= side_by_side {
        ui.horizontal(|ui| {
            ui.label(label);
            let room = (ui.available_width() - ui.spacing().item_spacing.x).max(1.0);
            add(ui, room)
        })
        .inner
    } else {
        ui.label(label);
        let room = ui.available_width().max(1.0);
        add(ui, room)
    }
}

fn draw_action(
    ui: &mut egui::Ui,
    index: usize,
    action: &Action,
    view: &InputMapView<'_>,
    out: &mut Vec<InputMapAction>,
) {
    let live = view.live.get(index).copied();
    let id = ui.make_persistent_id(("input_action", index));
    let state =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, index == 0);

    state
        .show_header(ui, |ui| {
            ui.horizontal(|ui| {
                let header = SelectableRow::new(format!(
                    "{} {}",
                    control_type_icon(action.control_type),
                    action.name
                ))
                .selected(view.selected == Some(Selection::Action(index)))
                .show(ui);
                if header.clicked() {
                    out.push(InputMapAction::Select(Selection::Action(index)));
                }
                ui.weak(control_type_label(action.control_type));

                // The live half. Only meaningful while something plays, and deliberately absent
                // rather than zeroed otherwise — a zero would read as "not firing", which is a
                // different statement from "nobody is asking".
                if let Some(live) = live {
                    ui.separator();
                    if live.pressed {
                        ui.colored_label(egui::Color32::from_rgb(120, 220, 120), "▶");
                    } else {
                        ui.weak("·");
                    }
                    ui.weak(format_value(action.control_type, live.value));
                }
            });
        })
        .body(|ui| {
            for (binding_index, binding) in action.bindings.iter().enumerate() {
                let at = BindingAddress {
                    action: index,
                    binding: binding_index,
                };
                draw_binding(
                    ui,
                    at,
                    binding,
                    view.awaiting == Some(at),
                    view.selected == Some(Selection::Binding(at)),
                    out,
                );
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .small_button(format!("{} Binding", icons::PLUS))
                    .clicked()
                {
                    out.push(InputMapAction::AddBinding { action: index });
                }
                draw_add_composite_menu(ui, index, action.control_type, out);
                // A single-action file has nothing left once its action
                // is gone, so removing it is not offered.
                if !view.single_action
                    && ui
                        .small_button(format!("{} Action", icons::TRASH))
                        .clicked()
                {
                    out.push(InputMapAction::RemoveAction { action: index });
                }
            });
        });
}

/// The "+ Composite" menu, listing what fits `control_type`.
fn draw_add_composite_menu(
    ui: &mut egui::Ui,
    action: usize,
    control_type: ControlType,
    out: &mut Vec<InputMapAction>,
) {
    ui.menu_button(format!("{} Composite", icons::PLUS), |ui| {
        let fits: Vec<Composite> = Composite::ALL
            .iter()
            .copied()
            .filter(|c| c.control_type() == control_type)
            .collect();
        let offered = if fits.is_empty() {
            Composite::ALL.to_vec()
        } else {
            fits
        };
        for composite in offered {
            if ui.button(composite.label()).clicked() {
                out.push(InputMapAction::AddComposite { action, composite });
                ui.close();
            }
        }
    })
    .response
    .on_hover_text("Several controls read as one value — WASD, or Ctrl+S");
}

fn draw_binding(
    ui: &mut egui::Ui,
    at: BindingAddress,
    binding: &Binding,
    awaiting: bool,
    selected: bool,
    out: &mut Vec<InputMapAction>,
) {
    let label = match (&binding.role, awaiting) {
        (_, true) => "  press any key…  (Esc to cancel)".to_owned(),
        (Role::Whole(path), _) => format!("  {}", describe(*path)),
        (Role::CompositeHead(composite), _) => {
            format!("  {} {composite:?}", icons::TREE_STRUCTURE)
        }
        (Role::Part { name, path }, _) => {
            format!("      {} — {}", part_label(*name), describe(*path))
        }
    };

    ui.horizontal(|ui| {
        let response = SelectableRow::new(label)
            .selected(selected || awaiting)
            .show(ui);
        // A click selects; the pane edits. Listening immediately would
        // make it impossible to look at a binding without arming it.
        if response.clicked() {
            out.push(InputMapAction::Select(Selection::Binding(at)));
        }
        response.context_menu(|ui| {
            if ui.button(format!("{} Remove", icons::TRASH)).clicked() {
                out.push(InputMapAction::RemoveBinding(at));
                ui.close();
            }
        });
    });

    if !binding.processors.is_empty() {
        // The one row drawn without `SelectableRow`, so the one that
        // did not inherit its truncation.
        ui.horizontal_wrapped(|ui| {
            ui.add_space(24.0);
            for processor in &binding.processors {
                ui.add(
                    egui::Label::new(egui::RichText::new(format!("{processor:?}")).weak())
                        .truncate(),
                );
            }
        });
    }
}

/// A control, as an author would name it.
fn describe(path: ControlPath) -> String {
    let device = match path.device() {
        DeviceClass::Keyboard => "Key",
        DeviceClass::Mouse => "Mouse",
        DeviceClass::Gamepad => "Pad",
    };
    let control = match path {
        ControlPath::Key(key) => format!("{key:?}"),
        ControlPath::Mouse(button) => format!("{button:?}"),
        ControlPath::Button(button) => format!("{button:?}"),
        ControlPath::Axis(axis) => format!("{axis:?}"),
        ControlPath::MouseMotion(axis) => format!("Motion {axis:?}"),
    };
    format!("{device} / {control}")
}

fn part_label(name: PartName) -> &'static str {
    match name {
        PartName::Positive => "positive",
        PartName::Negative => "negative",
        PartName::Up => "up",
        PartName::Down => "down",
        PartName::Left => "left",
        PartName::Right => "right",
        PartName::Forward => "forward",
        PartName::Backward => "backward",
        PartName::Modifier => "modifier",
        PartName::Modifier2 => "modifier 2",
        PartName::Value => "value",
    }
}

fn control_type_label(control_type: ControlType) -> &'static str {
    match control_type {
        ControlType::Button => "button",
        ControlType::Axis => "axis",
        ControlType::Vector2 => "vector2",
        ControlType::Vector3 => "vector3",
    }
}

fn control_type_icon(control_type: ControlType) -> &'static str {
    match control_type {
        ControlType::Button => icons::CUBE,
        ControlType::Axis => icons::SLIDERS,
        ControlType::Vector2 | ControlType::Vector3 => icons::ARROWS_CLOCKWISE,
    }
}

/// Formats a live value the way the action's type reads it.
fn format_value(control_type: ControlType, value: glam::Vec3) -> String {
    match control_type {
        ControlType::Button => format!("{:.0}", value.x),
        ControlType::Axis => format!("{:+.2}", value.x),
        ControlType::Vector2 => format!("{:+.2}, {:+.2}", value.x, value.y),
        ControlType::Vector3 => {
            format!("{:+.2}, {:+.2}, {:+.2}", value.x, value.y, value.z)
        }
    }
}

mod properties;

use properties::*;

#[cfg(test)]
mod tests;
