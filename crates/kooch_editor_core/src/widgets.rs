//! Widgets shared by more than one panel.
//!
//! Anything here exists because two panels needed the same thing and the
//! second one silently did without. [`SelectableRow`] is the first: the
//! World panel worked out how to draw a full-width list row, wrote down
//! why, and the asset browser kept using `ui.selectable_label` — which
//! stops at the end of the text and leaves the rest of the panel dead to
//! the pointer.

/// Height of one list row, in points.
///
/// A single definition on purpose. A virtualized list tells egui how tall
/// its rows are before drawing any of them — it has to place a scrollbar
/// for six hundred entities while laying out the twenty that fit. Two
/// copies of this formula drift by a pixel a row, and by the bottom of a
/// long list the scrollbar is lying about where it is.
///
/// # Without the spacing between rows
///
/// `ScrollArea::show_rows` names its parameter `row_height_sans_spacing`
/// and adds `item_spacing.y` itself. This used to include the spacing, so
/// egui reserved a row's height *plus two* gaps while each row occupied
/// its height plus one — four pixels of nothing per row. Invisible on ten
/// rows, a finger's width of empty panel on forty, and growing with the
/// panel because the number of visible rows does. That is what finally
/// identified it (#708): the gap scaled with the height.
pub(crate) fn row_height(ui: &egui::Ui) -> f32 {
    use egui::NumExt as _;
    let line = ui.text_style_height(&egui::TextStyle::Button);
    (line + 2.0 * ui.spacing().button_padding.y).at_least(ui.spacing().interact_size.y)
}

/// A list row that spans the full width of its panel.
///
/// # Why not `ui.selectable_label`
///
/// That widget is as wide as its text. A row is a *target* — for a click,
/// for a drop, for the highlight that says which item is selected — and a
/// target that stops where the text stops leaves most of the panel dead
/// to the pointer. It also makes selection look ragged, since the
/// highlight is a different width on every row.
///
/// The text is truncated rather than wrapped: a wrapped name would make
/// its own row taller than [`row_height`] promised, and in a virtualized
/// list every row below it would be drawn where the scrollbar says the
/// previous one ended rather than where it actually did.
pub(crate) struct SelectableRow {
    text: egui::WidgetText,
    selected: bool,
    sense: egui::Sense,
    dimmed: bool,
}

impl SelectableRow {
    pub(crate) fn new(text: impl Into<egui::WidgetText>) -> Self {
        Self {
            text: text.into(),
            selected: false,
            sense: egui::Sense::click(),
            dimmed: false,
        }
    }

    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Widen what the row reacts to. Click and drag on one response
    /// rather than two widgets, so a drag overlay cannot steal the click
    /// that would have selected the row.
    pub(crate) fn sense(mut self, sense: egui::Sense) -> Self {
        self.sense = sense;
        self
    }

    /// Fade the text — for an item currently being dragged somewhere
    /// else, so the row it came from reads as "in transit".
    pub(crate) fn dimmed(mut self, dimmed: bool) -> Self {
        self.dimmed = dimmed;
        self
    }

    pub(crate) fn show(self, ui: &mut egui::Ui) -> egui::Response {
        let button_padding = ui.spacing().button_padding;
        let wrap_width = ui.available_width() - 2.0 * button_padding.x;
        let galley = self.text.into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            wrap_width,
            egui::TextStyle::Button,
        );

        let desired_size = egui::vec2(ui.available_width(), row_height(ui));
        let (rect, resp) = ui.allocate_at_least(desired_size, self.sense);

        if ui.is_rect_visible(rect) {
            // Left-aligned and vertically centred, stated rather than
            // inherited from the layout: now that the row is as wide as
            // the panel, asking the layout where to put the text would
            // centre a short name in the middle of a wide row — and lose
            // the leading indentation that shows the hierarchy.
            let inner = rect.shrink2(button_padding);
            let text_pos = egui::pos2(inner.left(), inner.center().y - galley.size().y * 0.5);
            let visuals = ui.style().interact_selectable(&resp, self.selected);
            if self.selected || resp.hovered() || resp.highlighted() || resp.has_focus() {
                ui.painter().rect(
                    rect.expand(visuals.expansion),
                    visuals.corner_radius,
                    visuals.bg_fill,
                    visuals.bg_stroke,
                    egui::StrokeKind::Inside,
                );
            }
            let mut text_color = visuals.text_color();
            if self.dimmed {
                text_color = text_color.gamma_multiply(0.3);
            }
            ui.painter().galley(text_pos, galley, text_color);
        }
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `body` against a real `Ui`, since this reads egui's layout.
    fn with_ui<R>(width: f32, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
        let ctx = egui::Context::default();
        let mut body = Some(body);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 600.0),
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

    /// The bug this widget exists for: a short name must still give a
    /// row that reaches the far edge of the panel, or everything to the
    /// right of the text ignores the pointer.
    #[test]
    fn a_short_name_still_fills_the_panel() {
        let (row, available) = with_ui(400.0, |ui| {
            let available = ui.available_width();
            let resp = SelectableRow::new("a").show(ui);
            (resp.rect.width(), available)
        });
        assert!(
            (row - available).abs() < 1.0,
            "row was {row} wide in {available} of panel"
        );
    }

    /// …and a name far too long must not push past it either, or the
    /// panel scrolls sideways to reveal nothing.
    #[test]
    fn a_very_long_name_does_not_widen_the_row() {
        let (row, available) = with_ui(400.0, |ui| {
            let available = ui.available_width();
            let resp = SelectableRow::new("name ".repeat(200)).show(ui);
            (resp.rect.width(), available)
        });
        assert!(
            row <= available + 1.0,
            "row was {row} wide in {available} of panel — the text was not truncated"
        );
    }

    /// Two rows of very different name lengths have to be the same size,
    /// or the selection highlight is ragged and a virtualized list
    /// cannot predict where row N sits.
    #[test]
    fn every_row_is_the_same_size_whatever_it_says() {
        let (short, long, expected) = with_ui(400.0, |ui| {
            let expected = row_height(ui);
            let short = SelectableRow::new("x").show(ui).rect;
            let long = SelectableRow::new("a much, much longer label")
                .show(ui)
                .rect;
            (short, long, expected)
        });
        assert!((short.width() - long.width()).abs() < 1.0, "widths differ");
        assert!(
            (short.height() - expected).abs() < 1.0,
            "row height {} is not what row_height promised ({expected})",
            short.height()
        );
    }
}
