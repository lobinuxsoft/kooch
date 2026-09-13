//! Render graph — declarative pass scheduling.

mod node;
mod render_graph;

pub use node::{FnNode, FrameInfo, RenderContext, RenderNode};
pub use render_graph::{GraphError, NodeId, RenderGraph};
