//! Phosphor icon constants for the editor UI.
//!
//! Uses unicode codepoints from the Phosphor Icons Regular font,
//! embedded via `include_bytes!` in the font setup.
//!
//! # Check a codepoint before adding one
//!
//! Eleven of the first thirty here were wrong: `FOLDER_OPEN` drew a flag,
//! `COPY` a compass rose, `GEAR` a funnel, `ROCKET` a registered-trademark
//! sign. Nothing catches it — a wrong codepoint is a valid glyph, so it
//! renders something, and only a person looking at it notices.
//!
//! The authoritative mapping is `egui-phosphor`'s
//! [`src/variants/regular.rs`](https://github.com/amPerl/egui-phosphor).
//! Copy the value from there rather than reading it off an icon gallery,
//! which numbers them differently.
//!
//! `PLAY` and `STOP` are deliberately *not* Phosphor: they are the Unicode
//! geometric shapes, drawn by the text font.

/// Game-controller — an input map asset, and anything about bindings.
///
/// Verified twice, the way the note above asks: the codepoint comes from
/// `egui-phosphor`'s `regular.rs`, and the glyph was confirmed present in
/// the `Phosphor.ttf` this crate embeds — a valid codepoint missing from
/// *this* font renders as a blank box, which is the same failure as a
/// wrong one.
pub const GAME_CONTROLLER: &str = "\u{e26e}";

/// Arrows-out-cardinal — translate / move tool (4 cardinal arrows from center).
pub const ARROWS_OUT_CARDINAL: &str = "\u{e0a4}";

/// Arrows-clockwise — rotate tool (two curved arrows forming a cycle).
pub const ARROWS_CLOCKWISE: &str = "\u{e094}";

/// Arrows-out — scale tool (4 diagonal corner arrows pointing outward).
pub const ARROWS_OUT: &str = "\u{e0a2}";

/// Arrows-out-simple — alternative scale icon (cleaner two-arrow style).
pub const ARROWS_OUT_SIMPLE: &str = "\u{e0a6}";

/// Globe icon — used for "World" panel tab.
pub const GLOBE: &str = "\u{e288}";

/// Globe-simple icon — used for World-space rotation toggle.
pub const GLOBE_SIMPLE: &str = "\u{e28e}";

/// Map-pin-simple-area icon — used for Local-space rotation toggle.
pub const MAP_PIN_SIMPLE_AREA: &str = "\u{ee3c}";

/// Eye icon — used for "View" panel tab.
pub const EYE: &str = "\u{e220}";

/// Sliders icon — used for "Inspector" panel tab.
pub const SLIDERS: &str = "\u{e432}";

/// Cube icon — used for entity items.
pub const CUBE: &str = "\u{e1da}";

/// Plus icon — used for spawn/add buttons.
pub const PLUS: &str = "\u{e3d4}";

/// Minus icon — used for remove buttons.
pub const MINUS: &str = "\u{e32a}";

/// Trash icon — used for despawn/delete buttons.
pub const TRASH: &str = "\u{e4a6}";

/// Copy icon — used for the World panel's "Duplicate Entity" button.
pub const COPY: &str = "\u{e1ca}";

/// X/Close icon — used for remove component buttons.
pub const X: &str = "\u{e4f6}";

/// Puzzle piece icon — used for components.
pub const PUZZLE_PIECE: &str = "\u{e596}";

/// A prefab. Deliberately *not* `PUZZLE_PIECE`, which already means
/// "component" in the Components panel, the Archetypes panel and every
/// Inspector section header — a prefab is not one of those, and a shared
/// glyph is a claim that it is.
pub const PACKAGE: &str = "\u{e390}";

/// Magnifying glass icon — used for search.
pub const MAGNIFYING_GLASS: &str = "\u{e30c}";

/// Tree structure icon — used for "Archetypes" panel tab.
pub const TREE_STRUCTURE: &str = "\u{e67c}";

/// Faders icon — used for settings.
pub const FADERS: &str = "\u{e228}";

/// Stack icon — used for archetype groups.
pub const STACK: &str = "\u{e466}";

/// List bullets icon — used for Components tab.
pub const LIST_BULLETS: &str = "\u{e2f2}";

/// Chart bar icon — used for "Performance" panel tab (#463).
pub const CHART_BAR: &str = "\u{e150}";

/// Folder open icon — used for "Open Project" button.
pub const FOLDER_OPEN: &str = "\u{e256}";

/// Folder icon — a closed folder, for a row whose contents are hidden.
pub const FOLDER: &str = "\u{e24a}";

/// Folder plus icon — used for "New Project" button.
pub const FOLDER_PLUS: &str = "\u{e258}";

/// Play icon — filled triangle (classic media control).
pub const PLAY: &str = "\u{25b6}";

/// Stop icon — filled square (classic media control).
pub const STOP: &str = "\u{25a0}";

/// Gear icon — used for compiling/building status.
pub const GEAR: &str = "\u{e270}";

/// Arrow left icon — used for "Back" button.
pub const ARROW_LEFT: &str = "\u{e058}";

/// Rocket icon — used for launching projects.
pub const ROCKET: &str = "\u{e3fc}";

/// Terminal icon — used for output console.
pub const TERMINAL: &str = "\u{e47e}";
