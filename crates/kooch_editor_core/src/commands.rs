//! What the Edit View responds to, declared once and both obeyed and shown (#1436).
//!
//! 🔴 One table, two readers. `editor_camera::input::collect` asks these whether they fired, and
//! the overlay asks the same values how to print themselves. A hint written as a separate string
//! is a hint free to lie — and a label announcing a key that no longer does that is worse than no
//! label at all, because it costs trust in every other row beside it.
//!
//! ⚠️ A mouse gesture has no key to bind, so [`Chord::Mouse`] is display-only and its handler is
//! elsewhere. Those rows are the ones that can still drift; there are five of them and they are
//! marked here so the next person knows which half of the table is load-bearing.

/// Which modifiers a chord wants. Exact, not "at least": `W` must not fire while Ctrl is down,
/// or switching handle mode would also happen mid grid-snap.
///
/// Only the two the keyboard commands use. The Ctrl chords in this editor are all pointer drags,
/// which [`Chord::Mouse`] carries — a variant kept "for later" is a second answer free to
/// disagree with the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mods {
    None,
    Shift,
}

impl Mods {
    fn matches(self, input: &egui::InputState) -> bool {
        let m = input.modifiers;
        let ctrl = m.ctrl || m.command;
        match self {
            Self::None => !ctrl && !m.shift && !m.alt,
            Self::Shift => !ctrl && m.shift,
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Shift => "Shift+",
        }
    }
}

/// How a command is asked for.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Chord {
    Key {
        key: egui::Key,
        mods: Mods,
    },
    /// A pointer gesture. Shown, never tested — egui reports buttons through a `Response`, which
    /// this table has no business holding.
    Mouse(&'static str),
}

/// One thing the viewport does, and what asks for it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Command {
    pub(crate) chord: Chord,
    /// Written as the verb it performs, so the row reads as an instruction.
    pub(crate) what: &'static str,
}

impl Command {
    const fn key(key: egui::Key, mods: Mods, what: &'static str) -> Self {
        Self {
            chord: Chord::Key { key, mods },
            what,
        }
    }

    const fn mouse(gesture: &'static str, what: &'static str) -> Self {
        Self {
            chord: Chord::Mouse(gesture),
            what,
        }
    }

    /// Whether this command was asked for this frame. Always `false` for a pointer gesture.
    pub(crate) fn pressed(self, input: &egui::InputState) -> bool {
        match self.chord {
            Chord::Key { key, mods } => mods.matches(input) && input.key_pressed(key),
            Chord::Mouse(_) => false,
        }
    }

    /// How the overlay prints the chord.
    pub(crate) fn label(self) -> String {
        match self.chord {
            Chord::Key { key, mods } => format!("{}{}", mods.prefix(), key.name()),
            Chord::Mouse(gesture) => gesture.to_owned(),
        }
    }
}

// --- The view itself -------------------------------------------------------

pub(crate) const MOVE: Command = Command::key(egui::Key::W, Mods::None, "Move");
pub(crate) const ROTATE: Command = Command::key(egui::Key::E, Mods::None, "Rotate");
pub(crate) const SCALE: Command = Command::key(egui::Key::R, Mods::None, "Scale");
pub(crate) const FOCUS: Command = Command::key(egui::Key::F, Mods::None, "Frame the selection");

/// 🔴 Shift+E, because plain E is the rotate handle and asking one key to mean two things made
/// every extrude also switch mode.
pub(crate) const EXTRUDE: Command =
    Command::key(egui::Key::E, Mods::Shift, "Pull the selected faces out");

/// Adding a knot where the cursor is, rather than in the Inspector at the end of the list.
pub(crate) const ADD_KNOT: Command =
    Command::key(egui::Key::A, Mods::None, "Add a knot under the cursor");

/// What a click inside a block selects. Parallel to `ElementMode::ALL`, which is what `collect`
/// zips this against — the two lists are one list in two places and must stay the same length.
pub(crate) const ELEMENTS: [Command; 4] = [
    Command::key(egui::Key::Num1, Mods::None, "Select objects"),
    Command::key(egui::Key::Num2, Mods::None, "Select vertices"),
    Command::key(egui::Key::Num3, Mods::None, "Select edges"),
    Command::key(egui::Key::Num4, Mods::None, "Select faces"),
];

const NAVIGATION: [Command; 4] = [
    Command::mouse("MMB", "Orbit"),
    Command::mouse("Shift+MMB", "Pan"),
    Command::mouse("RMB hold", "Fly — WASD, Q/E"),
    Command::mouse("RMB click", "Spawn here"),
];

const DRAGGING: [Command; 2] = [
    Command::mouse("Ctrl+drag", "Snap to the grid"),
    Command::mouse("Ctrl+Shift+drag", "Set down on the surface"),
];

/// What the viewport responds to right now.
///
/// Ordered by how close it is to what the author is holding: the tool's own commands first, the
/// ones that are always true last. A list that reorders itself as the selection changes is a list
/// nobody can learn the shape of.
pub(crate) fn for_context(
    selection_has_transform: bool,
    editing_a_block: bool,
    editing_a_spline: bool,
) -> Vec<(&'static str, Vec<Command>)> {
    let mut groups: Vec<(&'static str, Vec<Command>)> = Vec::new();

    if editing_a_block {
        let mut block = ELEMENTS.to_vec();
        block.push(EXTRUDE);
        groups.push(("Block", block));
    }
    if editing_a_spline {
        groups.push(("Spline", vec![ADD_KNOT]));
    }
    if selection_has_transform {
        groups.push(("Transform", vec![MOVE, ROTATE, SCALE]));
        groups.push(("Dragging", DRAGGING.to_vec()));
    }
    groups.push(("View", {
        let mut view = vec![FOCUS];
        view.extend(NAVIGATION);
        view
    }));
    groups
}

#[cfg(test)]
mod tests;
