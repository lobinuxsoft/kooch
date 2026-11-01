//! In-game UI (#96): one entity per document, and the document's nodes as SoA over a `TaffyTree`
//! rather than as entities. Overlay and World share one pipeline; only the matrix differs.

mod atlas;
mod bindings;
mod build;
mod cascade;
#[cfg(test)]
mod cascade_tests;
mod controls;
mod document;
mod documents;
mod draw;
mod focus;
mod images;
mod layout;
pub mod markup;
mod plugin;
mod pointer;
mod restyle;
pub mod source;
pub mod style;
mod templates;
mod text;
mod tree;
mod trees;
mod ui_input;

pub use atlas::{AtlasGlyph, AtlasImage, GlyphAtlas, GlyphKey};
pub use bindings::bind_values;
pub use build::{DEFAULT_FONT_SIZE, build_tree};
pub use controls::{SLIDER_FILL, SLIDER_REST, SLIDER_THUMB, TOGGLE_CHECK};
pub use document::{MODE_CHOICES, MODE_OVERLAY, MODE_WORLD, UiDocument};
pub use documents::{build_documents, build_markup};
pub use draw::{draw_documents, draw_tree};
pub use focus::{UiFocus, navigate_focus};
pub use images::{UiImage, load_images};
pub use layout::{CANVAS_HEIGHT, canvas, layout_documents};
pub use plugin::{UiComponentsPlugin, UiPlugin};
pub use pointer::{UiAction, UiActions, UiEvent, UiEventKind, UiPointer, pick_pointer, ui_actions};
pub use restyle::restyle_documents;
pub use text::UiFonts;
pub use tree::{GlyphQuad, Paint, UiNode, UiRect, UiTree};
pub use trees::{UiTrees, prune_trees};

/// Node styles are taffy's own: flexbox and CSS Grid as the web defines them.
pub use taffy;
