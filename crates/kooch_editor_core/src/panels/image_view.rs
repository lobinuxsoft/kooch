//! The Image view's panel (#1792): the image fitted to the panel, a guide per slice side to drag,
//! and the four sides as numbers. A drag is held here and saved once, on release: one undo step.

use std::sync::Arc;

use crate::actions::EditorAction;
use crate::image_view::OpenImage;

/// Pixels either side of a guide that grab it.
const GRAB: f32 = 5.0;

/// The slice being dragged or typed and not yet saved, and whether the sides move together.
#[derive(Clone, Default)]
struct Draft {
    slice: Option<[u16; 4]>,
    linked: bool,
}

/// The texture egui shows, and the pixels it was made from.
#[derive(Clone)]
struct Shown(Arc<[u8]>, egui::TextureHandle);

pub(crate) fn draw(ui: &mut egui::Ui, open: Option<&OpenImage>, actions: &mut Vec<EditorAction>) {
    let Some(open) = open else {
        ui.centered_and_justified(|ui| {
            ui.weak("Double-click an image in the Asset Browser to edit its 9-slice.");
        });
        return;
    };
    let id = egui::Id::new(("image_view", open.guid));
    let mut draft: Draft = ui.data(|data| data.get_temp(id)).unwrap_or_default();
    let mut slice = draft.slice.unwrap_or(open.slice);
    let before = slice;

    ui.horizontal_wrapped(|ui| {
        ui.strong(open.path.file_name().map_or_else(
            || open.path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        ));
        ui.weak(format!("{} × {}", open.size[0], open.size[1]));
    });
    ui.horizontal_wrapped(|ui| {
        let limits = [open.size[1], open.size[0], open.size[1], open.size[0]];
        for (index, name) in ["Top", "Right", "Bottom", "Left"].into_iter().enumerate() {
            ui.label(name);
            let limit = limits[index].min(u16::MAX as u32) as u16;
            ui.add(
                egui::DragValue::new(&mut slice[index])
                    .range(0..=limit)
                    .suffix(" px"),
            )
            .on_hover_text(format!(
                "Texels the {} edge keeps unstretched.",
                name.to_lowercase()
            ));
        }
        ui.checkbox(&mut draft.linked, "Linked")
            .on_hover_text("Every side follows the one you change.");
    });
    if draft.linked
        && let Some(changed) = (0..4).find(|&side| slice[side] != before[side])
    {
        slice = [slice[changed]; 4];
    }
    ui.separator();
    let dragged = guides(ui, open, &mut slice, draft.linked);

    draft.slice = (slice != open.slice).then_some(slice);
    let pressed = ui.input(|input| input.pointer.any_down());
    if let Some(slice) = draft.slice.filter(|_| !pressed && !dragged) {
        actions.push(EditorAction::SetImageSlice {
            guid: open.guid,
            slice,
        });
        draft.slice = None;
    }
    ui.data_mut(|data| data.insert_temp(id, draft));
}

/// Draws the image fitted to what is left of the panel, with a draggable guide per side. Whether
/// a guide is being dragged.
fn guides(ui: &mut egui::Ui, open: &OpenImage, slice: &mut [u16; 4], linked: bool) -> bool {
    let texture = texture(ui, open);
    let available = ui.available_rect_before_wrap();
    let [width, height] = open.size.map(|side| side.max(1) as f32);
    let fit = (available.width() / width).min(available.height() / height);
    // Whole screen pixels per texel once it is enlarged, so each texel is the same size.
    let zoom = if fit >= 1.0 { fit.floor() } else { fit };
    let size = egui::vec2(width, height) * zoom;
    let rect = egui::Rect::from_min_size((available.center() - size / 2.0).round(), size);
    ui.allocate_rect(available, egui::Sense::hover());
    let painter = ui.painter_at(available);
    painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
    painter.image(
        texture.id(),
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );

    let to_texel = |screen: f32, origin: f32, limit: f32| {
        ((screen - origin) / zoom).round().clamp(0.0, limit) as u16
    };
    let stroke = egui::Stroke::new(1.5, ui.visuals().selection.stroke.color);
    let [wide, tall] = open.size.map(|side| side.min(u16::MAX as u32) as u16);
    let mut dragged = false;
    for side in 0..4 {
        let vertical = side % 2 == 1;
        let at = match side {
            0 => rect.top() + slice[0] as f32 * zoom,
            1 => rect.right() - slice[1] as f32 * zoom,
            2 => rect.bottom() - slice[2] as f32 * zoom,
            _ => rect.left() + slice[3] as f32 * zoom,
        };
        let line = match vertical {
            true => egui::Rect::from_x_y_ranges(at - GRAB..=at + GRAB, rect.y_range()),
            false => egui::Rect::from_x_y_ranges(rect.x_range(), at - GRAB..=at + GRAB),
        };
        let response = ui.interact(
            line,
            ui.id().with(("slice_guide", side)),
            egui::Sense::drag(),
        );
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(match vertical {
                true => egui::CursorIcon::ResizeHorizontal,
                false => egui::CursorIcon::ResizeVertical,
            });
        }
        if let Some(pointer) = response
            .interact_pointer_pos()
            .filter(|_| response.dragged())
        {
            dragged = true;
            let texel = match side {
                0 => to_texel(pointer.y, rect.top(), height),
                1 => wide.saturating_sub(to_texel(pointer.x, rect.left(), width)),
                2 => tall.saturating_sub(to_texel(pointer.y, rect.top(), height)),
                _ => to_texel(pointer.x, rect.left(), width),
            };
            match linked {
                true => *slice = [texel; 4],
                false => slice[side] = texel,
            }
        }
        let [from, to] = match vertical {
            true => [egui::pos2(at, rect.top()), egui::pos2(at, rect.bottom())],
            false => [egui::pos2(rect.left(), at), egui::pos2(rect.right(), at)],
        };
        painter.line_segment([from, to], stroke);
    }
    dragged
}

/// The image as an egui texture, made again only when the view opens other pixels.
fn texture(ui: &egui::Ui, open: &OpenImage) -> egui::TextureHandle {
    let id = egui::Id::new("image_view_texture");
    let shown: Option<Shown> = ui.data(|data| data.get_temp(id));
    if let Some(Shown(pixels, texture)) = shown
        && Arc::ptr_eq(&pixels, &open.pixels)
    {
        return texture;
    }
    let image =
        egui::ColorImage::from_rgba_unmultiplied(open.size.map(|side| side as usize), &open.pixels);
    let texture = ui
        .ctx()
        .load_texture("image_view", image, egui::TextureOptions::NEAREST);
    ui.data_mut(|data| data.insert_temp(id, Shown(open.pixels.clone(), texture.clone())));
    texture
}
