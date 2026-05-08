//! Karras 2012 parallel construction primitives: the longest-common-
//! prefix [`delta`] function and the per-internal-node range / split
//! computation [`karras_range_and_split`]. Used by the CPU builder in
//! the parent module.

/// Karras' delta function: longest common prefix length between
/// `morton[i]` and `morton[j]`, treating equal codes as resolved by
/// appending the 32-bit index (the index tie-break makes the
/// algorithm well-defined when multiple items share a Morton code).
///
/// Returns `-1` when `j` is out of range, signalling "no neighbour
/// in this direction" to the caller.
pub(super) fn delta(morton: &[u32], i: usize, j: i64) -> i32 {
    let n = morton.len() as i64;
    if j < 0 || j >= n {
        return -1;
    }
    let xi = morton[i];
    let xj = morton[j as usize];
    if xi == xj {
        // All 32 morton bits equal — tie-break with index.
        return 32 + (i as u32 ^ j as u32).leading_zeros() as i32;
    }
    (xi ^ xj).leading_zeros() as i32
}

/// Run Karras' construction algorithm for one internal node `i`.
/// Returns `(first, last, gamma)` where `[first, last]` is the range
/// of leaves covered by the subtree and `gamma` is the split position
/// (left covers `[first, gamma]`, right covers `[gamma+1, last]`).
pub(super) fn karras_range_and_split(morton: &[u32], i: usize) -> (usize, usize, usize) {
    let i_s = i as i64;

    // Direction d ∈ {-1, +1}: which side of i extends the range.
    let delta_plus = delta(morton, i, i_s + 1);
    let delta_minus = delta(morton, i, i_s - 1);
    let d: i64 = if delta_plus > delta_minus { 1 } else { -1 };

    // Lower bound on the common prefix shared by every leaf in this
    // node's range — the "other end" must have a strictly longer
    // common prefix than the leaf one step in the opposite direction.
    let delta_min = delta(morton, i, i_s - d);

    // Exponential search to find an upper bound l_max on the range
    // length. Doubles until the leaf at i + l_max*d falls below
    // delta_min (or out of range).
    let mut l_max: i64 = 2;
    while delta(morton, i, i_s + l_max * d) > delta_min {
        l_max *= 2;
    }

    // Binary search inside [0, l_max) for the exact length l such
    // that delta(i, i + l*d) > delta_min and delta(i, i + (l+1)*d) ≤ delta_min.
    let mut l: i64 = 0;
    let mut t = l_max / 2;
    while t > 0 {
        if delta(morton, i, i_s + (l + t) * d) > delta_min {
            l += t;
        }
        t /= 2;
    }
    let j = i_s + l * d;

    // Split position γ: largest s ∈ [0, l) such that
    // delta(i, i + s*d) > delta_node, where delta_node = delta(i, j).
    let delta_node = delta(morton, i, j);
    let mut s: i64 = 0;
    let mut div: i64 = 2;
    loop {
        let t = ((l as f64) / (div as f64)).ceil() as i64;
        if delta(morton, i, i_s + (s + t) * d) > delta_node {
            s += t;
        }
        if t <= 1 {
            break;
        }
        div *= 2;
    }

    // For d=-1 the split lies one step further "left" so that the
    // returned γ is the inclusive end of the left child's range.
    let gamma = (i_s + s * d + d.min(0)) as usize;
    let first = i_s.min(j) as usize;
    let last = i_s.max(j) as usize;
    (first, last, gamma)
}
