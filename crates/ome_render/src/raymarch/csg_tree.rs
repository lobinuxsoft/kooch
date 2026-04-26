//! Postfix RPN CSG tree for the SDF ray-marcher.
//!
//! Replaces the per-entity flat-fold composition (where iteration order
//! determined the visible result) with a CSG tree linearised to a flat
//! token array in postfix order. The shader iterates the array once with
//! a fixed-size evaluation stack — no GPU recursion, no order-dependent
//! results.
//!
//! ECS state lives as a flat list of primitive entities, each with an
//! optional `SdfBlend` describing its role (add / intersect / subtract /
//! smooth variants). At upload time, primitives are grouped by role and
//! combined into a default tree:
//!
//! ```text
//! smooth_subtract(
//!     smooth_intersect(
//!         smooth_union(adds, k = max),
//!         intersects, k = max,
//!     ),
//!     subs, k = max,
//! )
//! ```
//!
//! Each role's leaves are folded pairwise into a balanced subtree of
//! depth `ceil(log2(N))`. The total tree depth then sits at roughly
//! `ceil(log2(adds)) + ceil(log2(intersects)) + ceil(log2(subs)) + 2`,
//! comfortably below the shader stack of 16 for any practical scene.

use bytemuck::{Pod, Zeroable};

/// Maximum CSG evaluation stack depth on the GPU. Trees deeper than
/// this overflow the shader stack; serialisation refuses them at upload
/// time with a clear error.
pub const MAX_STACK_DEPTH: u32 = 16;

/// Token kinds. Must match the shader.
pub const TOKEN_KIND_LEAF: u32 = 0;
pub const TOKEN_KIND_OPERATOR: u32 = 1;

/// CSG operator codes. Must match the shader's `apply_op` switch.
///
/// Hard variants are part of the canonical operator set (the shader
/// dispatches all six) but the migration default tree only emits smooth
/// ones today — kept here as the source of truth for future tree-editor
/// UX.
#[allow(dead_code)]
pub const OP_HARD_UNION: u32 = 0;
pub const OP_SMOOTH_UNION: u32 = 1;
#[allow(dead_code)]
pub const OP_HARD_INTERSECT: u32 = 2;
pub const OP_SMOOTH_INTERSECT: u32 = 3;
#[allow(dead_code)]
pub const OP_HARD_SUBTRACT: u32 = 4;
pub const OP_SMOOTH_SUBTRACT: u32 = 5;

/// Flat GPU token. 16 bytes, naturally aligned for std430 storage buffer.
///
/// - When `kind == TOKEN_KIND_LEAF`: `primitive_index` indexes the
///   primitives SSBO; `op` and `smoothness` are unused.
/// - When `kind == TOKEN_KIND_OPERATOR`: `op` selects the CSG operator,
///   `smoothness` is the blend radius (smooth ops only); `primitive_index`
///   is unused.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default, Debug, PartialEq)]
pub struct Token {
    pub kind: u32,
    pub op: u32,
    pub smoothness: f32,
    pub primitive_index: u32,
}

/// CPU-side tree node. Built from ECS state, then DFS-walked in
/// post-order to produce the flat token array uploaded to the GPU.
#[derive(Debug, Clone)]
pub enum CsgNode {
    Leaf {
        primitive_index: u32,
    },
    Operator {
        op: u32,
        smoothness: f32,
        left: Box<CsgNode>,
        right: Box<CsgNode>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsgError {
    /// Serialised tree would require an evaluation stack deeper than
    /// `MAX_STACK_DEPTH`. The contained value is the depth that was
    /// computed during serialisation.
    StackOverflow { depth: u32 },
}

impl CsgNode {
    /// Build a balanced binary tree by repeatedly combining adjacent
    /// pairs of nodes with the same operator. Depth is `ceil(log2(N))`.
    ///
    /// Returns `None` for an empty input. Returns the single leaf
    /// unchanged for an input of length 1 (no operator wrapping).
    pub fn balanced_fold(leaves: Vec<u32>, op: u32, smoothness: f32) -> Option<CsgNode> {
        if leaves.is_empty() {
            return None;
        }
        let mut nodes: Vec<CsgNode> = leaves
            .into_iter()
            .map(|primitive_index| CsgNode::Leaf { primitive_index })
            .collect();
        while nodes.len() > 1 {
            let mut next: Vec<CsgNode> = Vec::with_capacity(nodes.len().div_ceil(2));
            let mut iter = nodes.into_iter();
            while let Some(left) = iter.next() {
                match iter.next() {
                    Some(right) => next.push(CsgNode::Operator {
                        op,
                        smoothness,
                        left: Box::new(left),
                        right: Box::new(right),
                    }),
                    // Odd node out at this level: carry forward unchanged.
                    None => next.push(left),
                }
            }
            nodes = next;
        }
        nodes.into_iter().next()
    }

    /// Serialise the tree to a flat token vector in DFS post-order.
    ///
    /// Returns `Err(StackOverflow)` when the required evaluation-stack
    /// depth would exceed [`MAX_STACK_DEPTH`].
    pub fn serialise_postfix(&self) -> Result<Vec<Token>, CsgError> {
        let mut tokens = Vec::new();
        let depth = self.walk_postfix(&mut tokens);
        if depth > MAX_STACK_DEPTH {
            return Err(CsgError::StackOverflow { depth });
        }
        Ok(tokens)
    }

    /// Recurse: emit children, then self. Returns the peak stack depth
    /// reached while evaluating this subtree on the GPU.
    fn walk_postfix(&self, out: &mut Vec<Token>) -> u32 {
        match self {
            CsgNode::Leaf { primitive_index } => {
                out.push(Token {
                    kind: TOKEN_KIND_LEAF,
                    op: 0,
                    smoothness: 0.0,
                    primitive_index: *primitive_index,
                });
                1
            }
            CsgNode::Operator {
                op,
                smoothness,
                left,
                right,
            } => {
                let left_depth = left.walk_postfix(out);
                let right_depth = right.walk_postfix(out);
                out.push(Token {
                    kind: TOKEN_KIND_OPERATOR,
                    op: *op,
                    smoothness: *smoothness,
                    primitive_index: 0,
                });
                // While we evaluate `right` the GPU stack already holds the
                // left subtree's result, so peak depth is the maximum of
                // (left alone) and (right with left still on the stack).
                left_depth.max(1 + right_depth)
            }
        }
    }
}

/// Per-role primitive index lists for the migration default tree.
#[derive(Debug, Default, Clone)]
pub struct DefaultTreeRoles {
    pub adds: Vec<u32>,
    pub intersects: Vec<u32>,
    pub subs: Vec<u32>,
    pub add_smoothness_max: f32,
    pub intersect_smoothness_max: f32,
    pub subtract_smoothness_max: f32,
}

/// Build the canonical default tree from role-grouped primitive indices.
///
/// Layout:
/// ```text
/// smooth_subtract(
///     smooth_intersect(
///         smooth_union(adds, add_k),
///         intersects, intersect_k,
///     ),
///     subs, subtract_k,
/// )
/// ```
///
/// Per-role subtrees collapse cleanly when a role is empty:
/// - No adds → no scene at all (returns `None`).
/// - No intersects → adds subtree passes through unchanged.
/// - No subs → previous result passes through unchanged.
///
/// Intersect / subtract roles without any adds make no geometric sense
/// (intersect-of-nothing, subtract-from-nothing) and are dropped — the
/// resulting `None` upstream surfaces as an empty token list and the
/// shader renders the sky background.
pub fn build_default_tree(roles: DefaultTreeRoles) -> Option<CsgNode> {
    let DefaultTreeRoles {
        adds,
        intersects,
        subs,
        add_smoothness_max,
        intersect_smoothness_max,
        subtract_smoothness_max,
    } = roles;

    let mut node = CsgNode::balanced_fold(adds, OP_SMOOTH_UNION, add_smoothness_max)?;

    if !intersects.is_empty() {
        let intersect_subtree =
            CsgNode::balanced_fold(intersects, OP_SMOOTH_INTERSECT, intersect_smoothness_max)
                .expect("non-empty intersects guarantees Some");
        node = CsgNode::Operator {
            op: OP_SMOOTH_INTERSECT,
            smoothness: intersect_smoothness_max,
            left: Box::new(node),
            right: Box::new(intersect_subtree),
        };
    }

    if !subs.is_empty() {
        let subs_subtree =
            CsgNode::balanced_fold(subs, OP_SMOOTH_SUBTRACT, subtract_smoothness_max)
                .expect("non-empty subs guarantees Some");
        node = CsgNode::Operator {
            op: OP_SMOOTH_SUBTRACT,
            smoothness: subtract_smoothness_max,
            left: Box::new(node),
            right: Box::new(subs_subtree),
        };
    }

    Some(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_layout_is_16_bytes() {
        assert_eq!(std::mem::size_of::<Token>(), 16);
        assert_eq!(std::mem::align_of::<Token>(), 4);
    }

    #[test]
    fn empty_fold_returns_none() {
        assert!(CsgNode::balanced_fold(vec![], OP_SMOOTH_UNION, 0.0).is_none());
    }

    #[test]
    fn single_leaf_fold_returns_leaf() {
        let n = CsgNode::balanced_fold(vec![7], OP_SMOOTH_UNION, 0.5).unwrap();
        match n {
            CsgNode::Leaf { primitive_index } => assert_eq!(primitive_index, 7),
            other => panic!("expected leaf, got {:?}", other),
        }
    }

    #[test]
    fn pair_fold_one_operator() {
        let tokens = CsgNode::balanced_fold(vec![0, 1], OP_SMOOTH_UNION, 0.3)
            .unwrap()
            .serialise_postfix()
            .unwrap();
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].kind, TOKEN_KIND_LEAF);
        assert_eq!(tokens[0].primitive_index, 0);
        assert_eq!(tokens[1].kind, TOKEN_KIND_LEAF);
        assert_eq!(tokens[1].primitive_index, 1);
        assert_eq!(tokens[2].kind, TOKEN_KIND_OPERATOR);
        assert_eq!(tokens[2].op, OP_SMOOTH_UNION);
        assert!((tokens[2].smoothness - 0.3).abs() < 1e-6);
    }

    #[test]
    fn balanced_depth_is_logarithmic() {
        // 8 leaves → depth 3 in a perfectly balanced tree.
        let tree = CsgNode::balanced_fold((0..8).collect(), OP_SMOOTH_UNION, 0.0).unwrap();
        let mut tokens = Vec::new();
        let depth = tree.walk_postfix(&mut tokens);
        assert_eq!(depth, 4); // peak = 1 + 3 (carry left subtree while we descend right)
        assert_eq!(tokens.len(), 8 + 7); // 8 leaves + 7 operators
    }

    #[test]
    fn depth_overflow_rejected() {
        // 2^17 leaves → depth 17 + 1 carry = 18, exceeds MAX_STACK_DEPTH.
        let count = 1usize << 17;
        let tree = CsgNode::balanced_fold((0..count as u32).collect(), OP_SMOOTH_UNION, 0.0)
            .unwrap();
        match tree.serialise_postfix() {
            Err(CsgError::StackOverflow { depth }) => assert!(depth > MAX_STACK_DEPTH),
            other => panic!("expected StackOverflow, got {:?}", other),
        }
    }

    #[test]
    fn migration_default_tree_shape_full_roles() {
        // 2 adds + 1 intersect + 1 sub.
        let roles = DefaultTreeRoles {
            adds: vec![10, 11],
            intersects: vec![20],
            subs: vec![30],
            add_smoothness_max: 0.1,
            intersect_smoothness_max: 0.2,
            subtract_smoothness_max: 0.3,
        };
        let tree = build_default_tree(roles).unwrap();
        let tokens = tree.serialise_postfix().unwrap();
        // postfix order: 10, 11, smooth_union, 20, smooth_intersect, 30, smooth_subtract
        assert_eq!(tokens.len(), 7);
        assert_eq!(tokens[6].op, OP_SMOOTH_SUBTRACT);
        assert!((tokens[6].smoothness - 0.3).abs() < 1e-6);
        assert_eq!(tokens[4].op, OP_SMOOTH_INTERSECT);
        assert_eq!(tokens[2].op, OP_SMOOTH_UNION);
    }

    #[test]
    fn migration_default_tree_adds_only() {
        let roles = DefaultTreeRoles {
            adds: vec![1, 2, 3],
            ..Default::default()
        };
        let tree = build_default_tree(roles).unwrap();
        let tokens = tree.serialise_postfix().unwrap();
        // 3 leaves + 2 union operators.
        assert_eq!(tokens.len(), 5);
        assert!(
            tokens
                .iter()
                .filter(|t| t.kind == TOKEN_KIND_OPERATOR)
                .all(|t| t.op == OP_SMOOTH_UNION)
        );
    }

    #[test]
    fn migration_no_adds_returns_none() {
        // Intersect / subtract without any add are geometrically empty.
        let roles = DefaultTreeRoles {
            adds: vec![],
            intersects: vec![1],
            subs: vec![2],
            ..Default::default()
        };
        assert!(build_default_tree(roles).is_none());
    }

    #[test]
    fn order_independence_within_role() {
        // Same primitives, different leaf order in the input vector — the
        // resulting trees differ in tree shape but produce the same op /
        // smoothness uniform across all operators (k = max). The actual
        // shader output equality is byte-by-byte verified by the
        // raymarch integration test, not at this level.
        let a = build_default_tree(DefaultTreeRoles {
            adds: vec![0, 1, 2],
            add_smoothness_max: 0.1,
            ..Default::default()
        })
        .unwrap();
        let b = build_default_tree(DefaultTreeRoles {
            adds: vec![2, 1, 0],
            add_smoothness_max: 0.1,
            ..Default::default()
        })
        .unwrap();
        let ta = a.serialise_postfix().unwrap();
        let tb = b.serialise_postfix().unwrap();
        // Same number of tokens, same operators with same k.
        assert_eq!(ta.len(), tb.len());
        let ops_a: Vec<u32> = ta
            .iter()
            .filter(|t| t.kind == TOKEN_KIND_OPERATOR)
            .map(|t| t.op)
            .collect();
        let ops_b: Vec<u32> = tb
            .iter()
            .filter(|t| t.kind == TOKEN_KIND_OPERATOR)
            .map(|t| t.op)
            .collect();
        assert_eq!(ops_a, ops_b);
    }
}
