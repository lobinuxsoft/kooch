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
mod layout;
mod plugin;
pub mod source;
pub mod style;
mod text;
mod tree;
mod trees;

pub use atlas::{AtlasGlyph, GlyphAtlas, GlyphKey};
pub use build::{DEFAULT_FONT_SIZE, build_tree};
pub use document::{MODE_CHOICES, MODE_OVERLAY, MODE_WORLD, UiDocument};
pub use documents::build_documents;
pub use draw::draw_documents;
pub use layout::{CANVAS_HEIGHT, canvas, layout_documents};
pub use plugin::{UiComponentsPlugin, UiPlugin};
pub use text::UiFonts;
pub use tree::{GlyphQuad, Paint, UiNode, UiRect, UiTree};
pub use trees::{UiTrees, prune_trees};

/// Node styles are taffy's own: flexbox and CSS Grid as the web defines them.
pub use taffy;
