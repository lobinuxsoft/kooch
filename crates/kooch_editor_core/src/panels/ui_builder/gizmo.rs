//! The selected element's rect gizmo (#280 8.3): handles write its size, and an absolute one's
//! insets, to its inline style in px. Taffy places it; there are no anchors.

use kooch_ui::style::InlineStyle;

use crate::ui_builder::UiBuilder;

/// Handle side, in points.
const HANDLE: f32 = 7.0;

/// Which edges a handle moves, per axis: -1 the start one, 1 the end one, 0 neither.
type Sides = [i8; 2];

/// An element in the flow is placed by the layout: only its size is the gizmo's.
const FLOW: [Sides; 3] = [[1, 0], [0, 1], [1, 1]];
const ALL: [Sides; 8] = [
    [-1, -1],
    [0, -1],
    [1, -1],
    [1, 0],
    [1, 1],
    [0, 1],
    [-1, 1],
    [-1, 0],
];
/// Dragging the canvas from inside it moves an absolute element.
const BODY: Sides = [0, 0];

/// What a drag started from, so each frame writes from the start and the total delta.
#[derive(Clone)]
struct Grab {
    sides: Sides,
    pointer: egui::Pos2,
    style: String,
    size: [f32; 2],
    /// Its rect's corner past the parent's border, where an unpinned inset counts from.
    offset: [f32; 2],
    /// `[start, end]` per axis: left/right, top/bottom.
    insets: [[Option<f32>; 2]; 2],
}

/// `screen` is the element's rect in points, `units` canvas units per point, `canvas` the canvas's
/// own response: the body drags through it, so a middle drag over the element still pans.
pub(super) fn draw(
    ui: &mut egui::Ui,
    builder: &mut UiBuilder,
    origin: u32,
    screen: egui::Rect,
    units: f32,
    canvas: &egui::Response,
) {
    let Some((rect, corner, absolute, [top, right, bottom, left])) = builder.placement(origin)
    else {
        return;
    };
    let Some(style) = builder.inspect(origin).map(|inspected| inspected.style) else {
        return;
    };
    let id = ui.id().with(("ui_builder_gizmo", origin));
    let accent = ui.visuals().selection.stroke.color;
    let painter = ui.painter().clone();
    let handles: &[Sides] = if absolute { &ALL } else { &FLOW };
    let mut grab: Option<Grab> = ui.data(|data| data.get_temp(id));
    let mut drags = Vec::with_capacity(handles.len() + 1);
    if absolute {
        let over = canvas.hover_pos().is_some_and(|pos| screen.contains(pos));
        if over && grab.is_none() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
        }
        let pressed = ui.input(|input| input.pointer.press_origin());
        if !canvas.drag_started() || pressed.is_some_and(|pos| screen.contains(pos)) {
            drags.push((BODY, canvas.clone()));
        }
    }
    for &sides in handles {
        let at = egui::pos2(
            egui::lerp(screen.x_range(), (sides[0] + 1) as f32 / 2.0),
            egui::lerp(screen.y_range(), (sides[1] + 1) as f32 / 2.0),
        );
        let square = egui::Rect::from_center_size(at, egui::Vec2::splat(HANDLE));
        let response = ui.interact(square.expand(2.0), id.with(sides), egui::Sense::drag());
        painter.rect_filled(square, 1.0, ui.visuals().extreme_bg_color);
        painter.rect_stroke(
            square,
            1.0,
            egui::Stroke::new(1.5, accent),
            egui::StrokeKind::Inside,
        );
        drags.push((sides, response.on_hover_cursor(cursor(sides))));
    }
    for (sides, response) in &drags {
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(pointer) = ui.input(|input| input.pointer.press_origin())
        {
            grab = Some(Grab {
                sides: *sides,
                pointer,
                style: style.clone(),
                size: rect.size.to_array(),
                offset: (rect.min - corner).to_array(),
                insets: [[left, right], [top, bottom]],
            });
        }
    }
    let held = grab
        .as_ref()
        .and_then(|grab| drags.iter().find(|(sides, _)| *sides == grab.sides))
        .map(|(_, response)| response)
        .filter(|response| response.dragged_by(egui::PointerButton::Primary));
    match (&grab, held.and_then(egui::Response::interact_pointer_pos)) {
        (Some(started), Some(pointer)) => {
            let delta = (pointer - started.pointer) * units;
            let written = resized(started, [delta.x.round(), delta.y.round()], absolute);
            if written != style {
                builder.set_attribute(origin, "style", &written);
            }
        }
        _ => grab = None,
    }
    ui.data_mut(|data| match grab {
        Some(grab) => {
            data.insert_temp(id, grab);
        }
        None => data.remove::<Grab>(id),
    });
}

/// The inline style the drag leaves, from where it started.
fn resized(grab: &Grab, delta: [f32; 2], absolute: bool) -> String {
    let mut style = InlineStyle::parse(&grab.style);
    let moving = grab.sides == BODY;
    let keys = [["width", "left", "right"], ["height", "top", "bottom"]];
    for axis in 0..2 {
        let [size_key, start_key, end_key] = keys[axis];
        let [start, end] = grab.insets[axis];
        let (d, size) = (delta[axis], grab.size[axis]);
        let mut set = |key: &str, value: f32| style.set(key, Some(&format!("{}px", value.round())));
        match grab.sides[axis] {
            -1 => {
                set(size_key, (size - d).max(1.0));
                if absolute && end.is_none() {
                    set(start_key, start.unwrap_or(grab.offset[axis]) + d);
                }
            }
            1 => {
                set(size_key, (size + d).max(1.0));
                if let (true, Some(end), None) = (absolute, end, start) {
                    set(end_key, end - d);
                }
            }
            _ if moving && d != 0.0 => match (start, end) {
                (None, Some(end)) => set(end_key, end - d),
                (start, _) => set(start_key, start.unwrap_or(grab.offset[axis]) + d),
            },
            _ => {}
        }
    }
    style.write()
}

fn cursor(sides: Sides) -> egui::CursorIcon {
    match sides {
        [0, _] => egui::CursorIcon::ResizeVertical,
        [_, 0] => egui::CursorIcon::ResizeHorizontal,
        [-1, -1] | [1, 1] => egui::CursorIcon::ResizeNwSe,
        _ => egui::CursorIcon::ResizeNeSw,
    }
}
