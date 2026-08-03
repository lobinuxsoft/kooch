//! Input Map panel — where bindings are configured.
//!
//! # What it draws, and what it does not evaluate
//!
//! The panel shows the map and **what the running host says each action
//! is worth**. It does not resolve the map itself.
//!
//! That distinction is the whole design. If the editor evaluated actions
//! for display while the host evaluated them for play, the same value
//! would exist in two places computed by two code paths — the single
//! shape behind all five prefab bugs in #611. So the live column is
//! empty when nothing is playing, and that is correct rather than a gap:
//! "what is this action worth" has no meaning with no simulation.
//!
//! # Rebinding
//!
//! Click a binding, press an input, it is stored. The editor grew its own
//! input backend in #711 (`bootstrap.rs`), registered *after* the egui
//! layer so a key typed into a focused text field stops there — which is
//! exactly what a "press any key" prompt must not steal.

use kooch_input::actions::{
    Action, ActionMap, Binding, ControlPath, ControlType, DeviceClass, PartName, Role,
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
}

/// What the host says an action is worth right now.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LiveAction {
    pub value: glam::Vec2,
    pub pressed: bool,
}

/// What the properties pane is editing.
///
/// Unity splits this into four views — action, binding, composite,
/// composite part — chosen by what is selected in the tree. One enum
/// covers the same ground here because the panel is one column rather
/// than three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Selection {
    Action(usize),
    Binding(BindingAddress),
}

/// Which binding a click was on — an action index and a binding index
/// inside it.
///
/// Positional rather than an id, because a binding has no identity of its
/// own and does not need one: the list is the data, and an edit is
/// applied to the same list the click came from, in the same frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BindingAddress {
    pub action: usize,
    pub binding: usize,
}

/// What the user did, for the caller to apply.
///
/// The panel returns intent rather than mutating the map: the map may
/// live behind an asset handle, an undo stack, or a socket, and a panel
/// that writes through all three is a panel that knows about all three.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum InputMapAction {
    /// Write the map back to its file.
    Save,
    /// Select an action or a binding — what the properties pane edits.
    Select(Selection),
    /// Rename an action. The name is what gameplay resolves, so this is
    /// the single most consequential edit in the panel.
    RenameAction {
        action: usize,
        name: String,
    },
    /// Change what an action produces.
    SetControlType {
        action: usize,
        control_type: ControlType,
    },
    /// Start listening for an input to put on this binding.
    BeginRebind(BindingAddress),
    /// Stop listening without changing anything.
    CancelRebind,
    /// Replace what this binding reads.
    Rebind {
        at: BindingAddress,
        path: ControlPath,
    },
    RemoveBinding(BindingAddress),
    AddBinding {
        action: usize,
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

    ui.horizontal(|ui| {
        ui.label(format!("{} {}", icons::GAME_CONTROLLER, map.name));
        ui.weak(format!("priority {}", map.priority));
        if ui.button(format!("{} Action", icons::PLUS)).clicked() {
            actions.push(InputMapAction::AddAction);
        }
        // Edits live in memory until this is pressed, the same contract a
        // prefab has. The marker is what makes that visible rather than
        // something you find out by closing the tab.
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

    // The tree, then what is selected in it. Unity puts the properties
    // in a third column; one panel is narrower than a Unity window, so
    // they go underneath — same relationship, less horizontal room.
    let properties_height = 190.0;
    let tree_height = (ui.available_height() - properties_height).max(80.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .max_height(tree_height)
        .id_salt("input_map_actions")
        .show(ui, |ui| {
            for (index, action) in map.actions.iter().enumerate() {
                draw_action(ui, index, action, &view, &mut actions);
            }
        });

    ui.separator();
    draw_properties(ui, map, &view, &mut actions);

    actions
}

/// Edits whatever is selected. Empty when nothing is.
fn draw_properties(
    ui: &mut egui::Ui,
    map: &ActionMap,
    view: &InputMapView<'_>,
    out: &mut Vec<InputMapAction>,
) {
    let Some(selected) = view.selected else {
        ui.weak("Select an action or a binding to edit it.");
        return;
    };

    match selected {
        Selection::Action(index) => {
            let Some(action) = map.actions.get(index) else {
                return;
            };
            ui.label("Action");
            ui.horizontal(|ui| {
                ui.label("Name");
                // Kept in egui's memory while being typed and committed
                // on the way out: an edit per keystroke would push a
                // rename through the document — and through `resolve` —
                // for every intermediate spelling.
                let id = ui.make_persistent_id(("input_action_name", index));
                let mut name = ui
                    .data(|d| d.get_temp::<String>(id))
                    .unwrap_or_else(|| action.name.clone());
                let response = ui.text_edit_singleline(&mut name);
                if response.changed() {
                    ui.data_mut(|d| d.insert_temp(id, name.clone()));
                }
                if response.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let trimmed = name.trim();
                    // An empty name resolves to nothing, and a duplicate
                    // makes `resolve` a coin toss. Refusing here is
                    // cheaper than a control that silently stops working.
                    let taken = map
                        .actions
                        .iter()
                        .enumerate()
                        .any(|(other, a)| other != index && a.name == trimmed);
                    if !trimmed.is_empty() && !taken && trimmed != action.name {
                        out.push(InputMapAction::RenameAction {
                            action: index,
                            name: trimmed.to_owned(),
                        });
                    }
                    ui.data_mut(|d| d.remove::<String>(id));
                }
            });
            ui.horizontal(|ui| {
                ui.label("Type");
                for control_type in [ControlType::Button, ControlType::Axis, ControlType::Vector2] {
                    if ui
                        .selectable_label(
                            action.control_type == control_type,
                            control_type_label(control_type),
                        )
                        .clicked()
                        && action.control_type != control_type
                    {
                        out.push(InputMapAction::SetControlType {
                            action: index,
                            control_type,
                        });
                    }
                }
            });
        }
        Selection::Binding(at) => {
            let Some(binding) = map
                .actions
                .get(at.action)
                .and_then(|a| a.bindings.get(at.binding))
            else {
                return;
            };
            match &binding.role {
                Role::CompositeHead(composite) => {
                    ui.label("Composite");
                    ui.weak(format!("{composite:?}"));
                    ui.weak("Its parts are the rows underneath.");
                }
                Role::Whole(path) | Role::Part { path, .. } => {
                    ui.label(match binding.role {
                        Role::Part { name, .. } => format!("Binding — {}", part_label(name)),
                        _ => "Binding".to_owned(),
                    });
                    draw_control_picker(ui, at, *path, out);
                }
            }
        }
    }
}

/// Picks the control a binding reads.
///
/// Two dropdowns rather than Unity's control-path tree. Theirs has to
/// parse `<Gamepad>/buttonSouth` out of a string and offer wildcards and
/// usage tags; ours is a closed enum, so the device narrows the list and
/// the list is exhaustive by construction — `ALL` comes from the same
/// macro expansion as the variants.
fn draw_control_picker(
    ui: &mut egui::Ui,
    at: BindingAddress,
    current: ControlPath,
    out: &mut Vec<InputMapAction>,
) {
    ui.horizontal(|ui| {
        ui.label("Device");
        for device in [
            DeviceClass::Keyboard,
            DeviceClass::Mouse,
            DeviceClass::Gamepad,
        ] {
            if ui
                .selectable_label(current.device() == device, format!("{device:?}"))
                .clicked()
                && current.device() != device
            {
                // Switching device rebinds to that device's first
                // control. Leaving it unbound would be a binding that
                // reads nothing, which looks identical to a broken one.
                if let Some(path) = first_control(device) {
                    out.push(InputMapAction::Rebind { at, path });
                }
            }
        }
    });

    ui.horizontal(|ui| {
        ui.label("Control");
        egui::ComboBox::from_id_salt(("control_picker", at.action, at.binding))
            .selected_text(control_label(current))
            .show_ui(ui, |ui| {
                for path in controls_of(current.device()) {
                    if ui
                        .selectable_label(path == current, control_label(path))
                        .clicked()
                        && path != current
                    {
                        out.push(InputMapAction::Rebind { at, path });
                    }
                }
            });
    });
}

/// Every control a device class offers, for the picker.
fn controls_of(device: DeviceClass) -> Vec<ControlPath> {
    match device {
        DeviceClass::Keyboard => kooch_input::ids::KeyCode::ALL
            .iter()
            .map(|k| ControlPath::Key(*k))
            .collect(),
        DeviceClass::Mouse => kooch_input::ids::MouseButton::ALL
            .iter()
            .map(|b| ControlPath::Mouse(*b))
            .collect(),
        // Buttons and axes both, since a binding on a pad can be either
        // and forcing that choice into a third dropdown would be a
        // distinction the author does not think in.
        DeviceClass::Gamepad => kooch_input::ids::GamepadButton::ALL
            .iter()
            .map(|b| ControlPath::Button(*b))
            .chain(
                kooch_input::ids::GamepadAxis::ALL
                    .iter()
                    .map(|a| ControlPath::Axis(*a)),
            )
            .collect(),
    }
}

fn first_control(device: DeviceClass) -> Option<ControlPath> {
    controls_of(device).into_iter().next()
}

/// A control without its device prefix, for a list already grouped by one.
fn control_label(path: ControlPath) -> String {
    match path {
        ControlPath::Key(key) => format!("{key:?}"),
        ControlPath::Mouse(button) => format!("{button:?}"),
        ControlPath::Button(button) => format!("{button:?}"),
        ControlPath::Axis(axis) => format!("{axis:?} (axis)"),
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
    let mut state =
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

                // The live half. Only meaningful while something plays,
                // and deliberately absent rather than zeroed otherwise —
                // a zero would read as "not firing", which is a different
                // statement from "nobody is asking".
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
            ui.horizontal(|ui| {
                if ui
                    .small_button(format!("{} Binding", icons::PLUS))
                    .clicked()
                {
                    out.push(InputMapAction::AddBinding { action: index });
                }
                if ui
                    .small_button(format!("{} Action", icons::TRASH))
                    .clicked()
                {
                    out.push(InputMapAction::RemoveAction { action: index });
                }
            });
        });
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
        // A click selects; the properties pane below is where it is
        // edited. Unity does the same, and the reason is that a click
        // that immediately started listening for a key would make it
        // impossible to look at a binding without arming it.
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
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            for processor in &binding.processors {
                ui.weak(format!("{processor:?}"));
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
    }
}

fn control_type_label(control_type: ControlType) -> &'static str {
    match control_type {
        ControlType::Button => "button",
        ControlType::Axis => "axis",
        ControlType::Vector2 => "vector2",
    }
}

fn control_type_icon(control_type: ControlType) -> &'static str {
    match control_type {
        ControlType::Button => icons::CUBE,
        ControlType::Axis => icons::SLIDERS,
        ControlType::Vector2 => icons::ARROWS_CLOCKWISE,
    }
}

/// Formats a live value the way the action's type reads it.
fn format_value(control_type: ControlType, value: glam::Vec2) -> String {
    match control_type {
        ControlType::Button => format!("{:.0}", value.x),
        ControlType::Axis => format!("{:+.2}", value.x),
        ControlType::Vector2 => format!("{:+.2}, {:+.2}", value.x, value.y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kooch_input::actions::{Composite, Vector2Mode};
    use kooch_input::ids::{GamepadButton, KeyCode};

    fn map() -> ActionMap {
        ActionMap::new("gameplay")
            .add(Action::new("move", ControlType::Vector2).bind_all([
                Binding::composite(Composite::Vector2 {
                    mode: Vector2Mode::DigitalNormalized,
                }),
                Binding::part(PartName::Up, ControlPath::Key(KeyCode::KeyW)),
            ]))
            .add(
                Action::new("jump", ControlType::Button)
                    .bind(Binding::to(ControlPath::Key(KeyCode::Space)))
                    .bind(Binding::to(ControlPath::Button(GamepadButton::South))),
            )
    }

    fn with_ui<R>(body: impl FnOnce(&mut egui::Ui) -> R) -> R {
        let ctx = egui::Context::default();
        let mut body = Some(body);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(500.0, 700.0),
            )),
            ..Default::default()
        };
        let mut out = None;
        ctx.run_ui(input, |ui| {
            let body = body.take().expect("run_ui called the closure twice");
            egui::CentralPanel::default().show(ui, |ui| out = Some(body(ui)));
        });
        out.expect("central panel did not run")
    }

    /// With no map, the panel says how to make one rather than drawing an
    /// empty list that looks broken.
    #[test]
    fn with_no_map_it_says_where_to_get_one() {
        let actions = with_ui(|ui| {
            draw_input_map_content(
                ui,
                InputMapView {
                    map: None,
                    live: &[],
                    awaiting: None,
                    dirty: false,
                    selected: None,
                },
            )
        });
        assert!(actions.is_empty());
    }

    /// Drawing must not depend on the game running — editing bindings is
    /// the half that works with nothing playing.
    #[test]
    fn it_draws_with_no_live_values() {
        let map = map();
        with_ui(|ui| {
            draw_input_map_content(
                ui,
                InputMapView {
                    map: Some(&map),
                    live: &[],
                    awaiting: None,
                    dirty: false,
                    selected: None,
                },
            )
        });
    }

    /// And with them, without needing one per action — a host that
    /// reports fewer must not panic the editor.
    #[test]
    fn fewer_live_values_than_actions_is_not_fatal() {
        let map = map();
        let live = [LiveAction {
            value: glam::Vec2::new(1.0, 0.0),
            pressed: true,
        }];
        with_ui(|ui| {
            draw_input_map_content(
                ui,
                InputMapView {
                    map: Some(&map),
                    live: &live,
                    awaiting: None,
                    dirty: false,
                    selected: None,
                },
            )
        });
    }

    /// A live value reads as the action's type, not as a raw vector: a
    /// button showing `1.00, 0.00` invites the question of what the
    /// second number means.
    #[test]
    fn a_value_is_formatted_as_its_control_type() {
        let v = glam::Vec2::new(1.0, 0.0);
        assert_eq!(format_value(ControlType::Button, v), "1");
        assert_eq!(format_value(ControlType::Axis, v), "+1.00");
        assert_eq!(format_value(ControlType::Vector2, v), "+1.00, +0.00");
    }

    /// A control has to name its device, or `South` and `S` look alike in
    /// a list and a binding gets clicked by mistake.
    #[test]
    fn a_control_names_its_device() {
        assert_eq!(describe(ControlPath::Key(KeyCode::Space)), "Key / Space");
        assert_eq!(
            describe(ControlPath::Button(GamepadButton::South)),
            "Pad / South"
        );
    }
}
