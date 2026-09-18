//! Phosphor icon constants for the editor UI.

/// Game-controller — an input map asset, and anything about bindings.
pub const GAME_CONTROLLER: &str = "\u{e26e}";

/// Arrows-out-cardinal — translate / move tool (4 cardinal arrows from center).
pub const ARROWS_OUT_CARDINAL: &str = "\u{e0a4}";

/// Arrows-clockwise — rotate tool (two curved arrows forming a cycle).
pub const ARROWS_CLOCKWISE: &str = "\u{e094}";

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

/// A prefab. Deliberately *not* `PUZZLE_PIECE`, which already means "component" in the Components
/// panel, the Archetypes panel and every Inspector section header — a prefab is not one of those,
/// and a shared glyph is a claim that it is.
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

/// Arrow-up — move an item earlier in a list.
pub const ARROW_UP: &str = "\u{e08e}";

/// Arrow-down — move an item later in a list.
pub const ARROW_DOWN: &str = "\u{e03e}";

/// Rocket icon — used for launching projects.
pub const ROCKET: &str = "\u{e3fc}";

/// Terminal icon — used for output console.
pub const TERMINAL: &str = "\u{e47e}";

/// Dots-nine — vertex selection mode. Codepoint from `egui-phosphor`'s
/// `regular.rs`, glyph confirmed present in the embedded `Phosphor.ttf`.
pub const DOTS_NINE: &str = "\u{e1fc}";

/// Line-segment — edge selection mode.
pub const LINE_SEGMENT: &str = "\u{e6d2}";

/// Polygon — face selection mode.
pub const POLYGON: &str = "\u{e6d0}";
