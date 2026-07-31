//! Keyboard navigation over the Inspector's component sections.
//!
//! # What each key belongs to
//!
//! Two layers, and keeping them apart is the whole design:
//!
//! - **Tab** moves between *fields*, and egui already does that — a
//!   focusable widget registers itself and `Key::Tab` becomes
//!   `FocusDirection::Next` in egui's own memory. There is nothing to
//!   build, and building it would fight the thing that works.
//! - **The arrows** move between *components*, and only while no field
//!   holds keyboard focus. Once Tab has landed in a `DragValue` the arrows
//!   are that widget's — they nudge the value — and taking them back would
//!   be worse than not having them.
//!
//! So the Inspector answers the arrows when you are reading it and gets
//! out of the way when you are editing it (#661).
//!
//! # Why a toggle is a request
//!
//! Same reason as the asset tree: a section's collapse state lives in
//! egui memory under an id from `Ui::make_persistent_id`, which mixes in
//! the salt of the `Ui` it was called on and cannot be rebuilt from
//! outside. The keyboard names a component and the renderer applies it on
//! the way past.

use kooch_ecs::component::ComponentId;

/// The Inspector's keyboard state.
#[derive(Default)]
pub(crate) struct InspectorNav {
    /// Which component section the cursor is on.
    ///
    /// A `ComponentId` rather than an index: the list changes as
    /// components are added and removed, and the selection can change
    /// under it entirely.
    pub(crate) cursor: Option<ComponentId>,
    /// The sections drawn last frame, in order.
    pub(crate) rows: Vec<ComponentId>,
    /// A section the keyboard asked to open or close.
    pub(crate) toggle: Option<(ComponentId, bool)>,
    /// Set when the cursor moves, so the view scrolls to follow.
    pub(crate) scroll_to_cursor: bool,
}

impl InspectorNav {
    fn index(&self) -> Option<usize> {
        let cursor = self.cursor?;
        self.rows.iter().position(|row| *row == cursor)
    }

    /// Moves the cursor by `delta` sections, clamped.
    ///
    /// With no cursor, or one whose component is gone — removed, or a
    /// different entity selected — this lands on the first section rather
    /// than doing nothing, which would read as a key that never arrived.
    pub(crate) fn step(&mut self, delta: isize) {
        if self.rows.is_empty() {
            self.cursor = None;
            return;
        }
        let last = self.rows.len() as isize - 1;
        let next = match self.index() {
            Some(i) => (i as isize + delta).clamp(0, last),
            None => 0,
        };
        self.cursor = Some(self.rows[next as usize]);
        self.scroll_to_cursor = true;
    }

    /// Asks for the section under the cursor to open or close.
    pub(crate) fn set_open(&mut self, open: bool) {
        match self.cursor {
            Some(cursor) if self.index().is_some() => {
                self.toggle = Some((cursor, open));
                self.scroll_to_cursor = true;
            }
            // No cursor yet: the first arrow puts one somewhere instead of
            // being swallowed.
            _ => self.step(0),
        }
    }

    /// Whether `component` is where the cursor is.
    pub(crate) fn is_cursor(&self, component: ComponentId) -> bool {
        self.cursor == Some(component)
    }

    /// Takes the pending toggle, if it names this component.
    pub(crate) fn take_toggle_for(&mut self, component: ComponentId) -> Option<bool> {
        match self.toggle {
            Some((c, open)) if c == component => {
                self.toggle = None;
                Some(open)
            }
            _ => None,
        }
    }

    /// Reads the arrows, if this panel owns them this frame.
    ///
    /// Returns without touching anything while a widget holds keyboard
    /// focus: a `DragValue` under the caret owns Up and Down, and a
    /// `TextEdit` owns Left and Right.
    pub(crate) fn handle_keyboard(&mut self, ui: &egui::Ui) {
        self.scroll_to_cursor = false;
        if ui.memory(|m| m.focused().is_some()) {
            return;
        }

        let (up, down, left, right) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
            )
        });

        if up {
            self.step(-1);
        }
        if down {
            self.step(1);
        }
        if right {
            self.set_open(true);
        }
        if left {
            self.set_open(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u32) -> ComponentId {
        ComponentId(n)
    }

    fn nav() -> InspectorNav {
        InspectorNav {
            rows: vec![id(1), id(2), id(3)],
            ..Default::default()
        }
    }

    #[test]
    fn the_first_step_lands_on_the_first_section() {
        let mut n = nav();
        n.step(1);
        assert_eq!(n.cursor, Some(id(1)));
    }

    #[test]
    fn stepping_stops_at_both_ends() {
        let mut n = nav();
        n.cursor = Some(id(1));
        n.step(-4);
        assert_eq!(n.cursor, Some(id(1)));
        n.step(9);
        assert_eq!(n.cursor, Some(id(3)));
    }

    #[test]
    fn opening_and_closing_name_the_section_under_the_cursor() {
        let mut n = nav();
        n.cursor = Some(id(2));
        n.set_open(false);
        assert_eq!(n.toggle, Some((id(2), false)));
        n.set_open(true);
        assert_eq!(n.toggle, Some((id(2), true)));
    }

    /// A removed component, or a different entity selected, leaves a
    /// cursor with no section. The next key has to recover rather than do
    /// nothing.
    #[test]
    fn a_cursor_on_a_component_that_is_gone_recovers() {
        let mut n = nav();
        n.cursor = Some(id(99));
        n.step(1);
        assert_eq!(n.cursor, Some(id(1)));
    }

    /// Right with no cursor should place one, not be swallowed.
    #[test]
    fn the_first_arrow_places_a_cursor_even_when_it_is_a_toggle() {
        let mut n = nav();
        n.set_open(true);
        assert_eq!(n.cursor, Some(id(1)));
        assert_eq!(n.toggle, None, "nothing to toggle until a cursor exists");
    }

    #[test]
    fn a_toggle_is_only_taken_by_the_component_it_names() {
        let mut n = nav();
        n.toggle = Some((id(3), true));
        assert_eq!(n.take_toggle_for(id(1)), None);
        assert_eq!(n.take_toggle_for(id(3)), Some(true));
        assert_eq!(n.take_toggle_for(id(3)), None, "taken once");
    }

    #[test]
    fn nothing_selected_means_nowhere_for_a_cursor() {
        let mut n = InspectorNav::default();
        n.step(1);
        n.set_open(true);
        assert_eq!(n.cursor, None);
    }
}
