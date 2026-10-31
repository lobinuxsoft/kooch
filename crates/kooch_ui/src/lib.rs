//! In-game UI (#96): one entity per document, and the document's nodes as SoA over a `TaffyTree`
//! rather than as entities. Overlay and World share one pipeline; only the matrix differs.

mod atlas;
mod build;
mod cascade;
#[cfg(test)]
mod cascade_tests;
mod document;
mod documents;
mod draw;
mod focus;
mod images;
mod layout;
mod plugin;
mod pointer;
mod restyle;
pub mod source;
pub mod style;
mod text;
mod tree;
mod trees;
mod ui_input;

pub use atlas::{AtlasGlyph, AtlasImage, GlyphAtlas, GlyphKey};
pub use build::{DEFAULT_FONT_SIZE, build_tree};
pub use document::{MODE_CHOICES, MODE_OVERLAY, MODE_WORLD, UiDocument};
pub use documents::build_documents;
pub use draw::draw_documents;
pub use focus::{UiFocus, navigate_focus};
pub use images::{UiImage, load_images};
pub use layout::{CANVAS_HEIGHT, canvas, layout_documents};
pub use plugin::{UiComponentsPlugin, UiPlugin};
pub use pointer::{UiAction, UiActions, UiEvent, UiEventKind, UiPointer, pick_pointer};
pub use restyle::restyle_documents;
pub use text::UiFonts;
pub use tree::{GlyphQuad, Paint, UiNode, UiRect, UiTree};
pub use trees::{UiTrees, prune_trees};

/// Node styles are taffy's own: flexbox and CSS Grid as the web defines them.
pub use taffy;
