//! In-game UI (#96): one entity per document, and the document's nodes as SoA over a `TaffyTree`
//! rather than as entities. Overlay and World share one pipeline; only the matrix differs.

mod document;
mod layout;
mod plugin;
mod tree;
mod trees;

pub use document::{MODE_CHOICES, MODE_OVERLAY, MODE_WORLD, UiDocument};
pub use layout::{CANVAS_HEIGHT, canvas, layout_documents};
pub use plugin::{UiComponentsPlugin, UiPlugin};
pub use tree::{UiNode, UiRect, UiTree};
pub use trees::{UiTrees, prune_trees};

/// Node styles are taffy's own: flexbox and CSS Grid as the web defines them.
pub use taffy;
