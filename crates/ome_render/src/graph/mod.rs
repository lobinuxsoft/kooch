//! Render graph — declarative pass scheduling.
//!
//! Game / editor code declares passes ([`RenderNode`]) + inter-pass
//! dependencies ([`RenderGraph::connect`]). The graph topologically
//! sorts the dependency DAG and runs nodes in that order with one
//! shared `wgpu::CommandEncoder`.
//!
//! # Scope (PR-1 of #392)
//!
//! Covered:
//! - DAG storage with cycle detection
//! - Topological sort (Kahn's algorithm — deterministic insertion order)
//! - Trait-based [`RenderNode`] + closure adapter [`FnNode`]
//! - Encoder-shared execution
//!
//! Deferred (follow-ups):
//! - Resource graph (typed texture / buffer handles, transient lifetime
//!   tracking, automatic barrier insertion). wgpu handles intra-encoder
//!   barriers today; cross-pass aliasing arrives when meshlet pipeline
//!   needs it (#117).
//! - Migration of existing `MeshPassRenderer` / `SkyRenderPass` to
//!   graph nodes (separate PRs — keeps this one reviewable).
//! - Conditional execution / pass skipping based on runtime predicates.
//! - Parallel scheduling across non-dependent nodes (modern command
//!   buffer split).

mod node;
mod render_graph;

pub use node::{FnNode, RenderContext, RenderNode};
pub use render_graph::{GraphError, NodeId, RenderGraph};
