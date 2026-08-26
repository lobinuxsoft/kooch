use super::*;
use kooch_input::actions::{Composite, VectorMode};
use kooch_input::ids::{GamepadButton, KeyCode};

fn map() -> ActionMap {
    ActionMap::new("gameplay")
        .add(Action::new("move", ControlType::Vector2).bind_all([
            Binding::composite(Composite::Vector2 {
                mode: VectorMode::DigitalNormalized,
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
    with_ui_sized(egui::vec2(500.0, 700.0), body)
}

/// Draws at an exact size, for tests that care about width.
fn with_ui_sized<R>(size: egui::Vec2, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let ctx = egui::Context::default();
    let mut body = Some(body);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        ..Default::default()
    };
    let mut out = None;
    ctx.run_ui(input, |ui| {
        let body = body.take().expect("run_ui called the closure twice");
        egui::CentralPanel::default().show(ui, |ui| out = Some(body(ui)));
    });
    out.expect("central panel did not run")
}

/// Draws the arrangement the editor shows — nested panels, a
/// `ComboBox` in the narrow one. The empty case returns before
/// reaching any of it, so it proves nothing about the layout.
#[test]
fn it_draws_the_tree_and_the_properties_column_together() {
    let map = map();
    for selected in [
        Selection::Action(0),
        Selection::Binding(BindingAddress {
            action: 1,
            binding: 0,
        }),
    ] {
        // Including widths too narrow for the device buttons on
        // one line, which is what the wrapped layouts are for.
        for width in [1600.0, 500.0, 260.0, 160.0] {
            with_ui_sized(egui::vec2(width, 480.0), |ui| {
                draw_input_map_content(
                    ui,
                    InputMapView {
                        map: Some(&map),
                        live: &[],
                        awaiting: None,
                        dirty: true,
                        selected: Some(selected),
                        single_action: false,
                    },
                )
            });
        }
    }
}

/// 🔴 A labelled control never asks for more room than it has.
///
/// The reported clipping: a `.max(60.0)` floor on the remaining
/// width overflowed the row, drawing `Binding — left` as `g — left`.
/// Same mistake as a fixed panel width, one widget down.
#[test]
fn a_labelled_control_never_exceeds_the_room_it_has() {
    for width in [600.0, 300.0, 160.0, 90.0, 40.0] {
        let (available, room) = with_ui_sized(egui::vec2(width, 200.0), |ui| {
            let available = ui.available_width();
            let mut room = f32::NAN;
            labeled_control(ui, "Control", |_, r| room = r);
            (available, room)
        });
        assert!(
            room <= available + 0.5,
            "at {width}px the control asked for {room} of {available} available",
        );
        assert!(room > 0.0, "at {width}px the control got no room at all");
    }
}

/// 🔴 Narrow the tab and the properties narrow with it.
///
/// `size_range(190.0..=420.0)` in pixels meant that on a 260px tab
/// the floor alone exceeded half of it: the tree got 70px and the
/// properties drew past the right edge. Asserted against the tab's
/// own width, which is what fails for pixels at every narrow size.
#[test]
fn the_properties_column_is_a_fraction_of_the_tab() {
    for tab in [3840.0, 1600.0, 900.0, 500.0, 320.0, 240.0, 120.0] {
        let (default_width, range) = properties_column(tab);
        assert!(
            *range.end() <= tab * 0.5 + f32::EPSILON,
            "at {tab}px the column may grow to {}, more than half the tab",
            range.end(),
        );
        assert!(
            range.contains(&default_width),
            "at {tab}px the starting width {default_width} is outside {range:?}",
        );
        assert!(
            *range.start() > 0.0 && *range.start() < *range.end(),
            "at {tab}px the range is degenerate: {range:?}",
        );
    }
}

/// The old constants, as the thing this must not become again.
#[test]
fn a_pixel_floor_would_swallow_a_narrow_tab() {
    let narrow = 260.0_f32;
    assert!(
        190.0 > narrow * 0.5,
        "the regression this guards is only meaningful if a 190px \
             floor really does exceed half of a {narrow}px tab",
    );
    assert!(*properties_column(narrow).1.start() < narrow * 0.5);
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
                single_action: false,
            },
        )
    });
    assert!(actions.is_empty());
}

/// Editing bindings is the half that works with nothing playing.
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
                single_action: false,
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
        value: glam::Vec3::new(1.0, 0.0, 0.0),
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
                single_action: false,
            },
        )
    });
}

/// A live value reads as the action's type, not as a raw vector: a
/// button showing `1.00, 0.00` invites the question of what the
/// second number means.
#[test]
fn a_value_is_formatted_as_its_control_type() {
    let v = glam::Vec3::new(1.0, 0.0, -0.5);
    assert_eq!(format_value(ControlType::Button, v), "1");
    assert_eq!(format_value(ControlType::Axis, v), "+1.00");
    assert_eq!(format_value(ControlType::Vector2, v), "+1.00, +0.00");
    assert_eq!(
        format_value(ControlType::Vector3, v),
        "+1.00, +0.00, -0.50",
        "a 3D action must show the component only it has"
    );
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
