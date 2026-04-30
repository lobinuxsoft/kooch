//! `FreeListPool` — `u32`-indexed free list of `[start, len)` ranges.
//!
//! CPU-only coordination: never crosses to GPU. Backs the three byte
//! pools owned by `OmeAccel` (nodes / leaves / primitives) plus the
//! singleton chunk-slot allocator. Lazy coalescing — the streaming
//! layer calls `coalesce` at quiescent moments (chunk eviction batch
//! boundaries) to amortise the sort across many frees.
//!
//! # Algorithm
//!
//! - `alloc(len)`: first-fit scan of `free_ranges`. The chosen range
//!   is shrunk in place (or removed if exact) — `O(F)` where `F` is
//!   the live free-range count. Returns `None` if no range fits.
//! - `free(start, len)`: push at the tail. `O(1)` amortised. The
//!   range is *not* immediately merged with neighbours — that work is
//!   deferred to `coalesce`.
//! - `coalesce`: sort by `start`, fold adjacent ranges. `O(F log F)`.
//!   Idempotent. The streaming layer calls this at the tail of every
//!   eviction batch and before any `fragmentation_metrics` query.
//!
//! `high_watermark` tracks the largest `start + len` ever allocated,
//! independent of subsequent frees — AC7's fragmentation metric
//! (`node_count_used / node_count_high_watermark`) reads it directly.

/// One free range in the pool, `[start, start + len)`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FreeRange {
    pub start: u32,
    pub len: u32,
}

/// Fragmentation metrics surfaced for AC7 and profiling.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FragmentationMetrics {
    /// Number of disjoint free ranges after coalescing.
    pub free_range_count: u32,
    /// Largest single contiguous free range, in elements.
    pub largest_free_range: u32,
    /// `start + len` of the largest position ever allocated.
    pub high_watermark: u32,
    /// Total elements currently in use (`capacity - sum(free.len)`).
    pub used: u32,
}

/// Single-pool allocator for the BLAS byte pools and the chunk-slot
/// table. Indices are `u32` — every consumer of the pool is GPU-bound.
#[derive(Debug)]
pub struct FreeListPool {
    capacity: u32,
    free_ranges: Vec<FreeRange>,
    high_watermark: u32,
}

impl FreeListPool {
    /// Build a pool with one free range covering `[0, capacity)`.
    pub fn new(capacity: u32) -> Self {
        let free_ranges = if capacity == 0 {
            Vec::new()
        } else {
            vec![FreeRange {
                start: 0,
                len: capacity,
            }]
        };
        Self {
            capacity,
            free_ranges,
            high_watermark: 0,
        }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn high_watermark(&self) -> u32 {
        self.high_watermark
    }

    /// First-fit allocation. Returns the start index of a range of
    /// length `len`, or `None` if no contiguous range fits.
    ///
    /// Allocating zero elements is a no-op and returns `Some(0)` — the
    /// caller has nothing to write but a valid offset for an empty
    /// slice.
    ///
    /// **Sort-order preserved.** Removes the picked range with
    /// `Vec::remove` (not `swap_remove`) so subsequent first-fit
    /// allocations keep walking left-to-right through the free list.
    /// `swap_remove` would shuffle the trailing range to position 0
    /// and steer the next alloc onto fresh capacity even when smaller
    /// holes are still available, blowing up `high_watermark` and
    /// regressing AC7 utilisation by ~33%.
    pub fn alloc(&mut self, len: u32) -> Option<u32> {
        if len == 0 {
            return Some(0);
        }
        let pick = self
            .free_ranges
            .iter()
            .position(|r| r.len >= len)?;
        let range = &mut self.free_ranges[pick];
        let start = range.start;
        if range.len == len {
            self.free_ranges.remove(pick);
        } else {
            range.start += len;
            range.len -= len;
        }
        let end = start + len;
        if end > self.high_watermark {
            self.high_watermark = end;
        }
        Some(start)
    }

    /// Return a range to the pool. Does NOT immediately merge with
    /// neighbours — call [`coalesce`](Self::coalesce) to consolidate.
    ///
    /// Freeing a zero-length range is a no-op.
    pub fn free(&mut self, start: u32, len: u32) {
        if len == 0 {
            return;
        }
        debug_assert!(
            start + len <= self.capacity,
            "free range out of bounds: [{}..{}) vs capacity {}",
            start,
            start + len,
            self.capacity
        );
        self.free_ranges.push(FreeRange { start, len });
    }

    /// Sort + fold adjacent free ranges. Idempotent. `O(F log F)`.
    pub fn coalesce(&mut self) {
        if self.free_ranges.len() < 2 {
            return;
        }
        self.free_ranges.sort_by_key(|r| r.start);
        let mut write = 0usize;
        for read in 1..self.free_ranges.len() {
            let cur = self.free_ranges[read];
            let prev = &mut self.free_ranges[write];
            if prev.start + prev.len == cur.start {
                prev.len += cur.len;
            } else {
                write += 1;
                self.free_ranges[write] = cur;
            }
        }
        self.free_ranges.truncate(write + 1);
    }

    /// Collapse fragmentation, then return metrics.
    pub fn fragmentation_metrics(&mut self) -> FragmentationMetrics {
        self.coalesce();
        let free_total: u32 = self.free_ranges.iter().map(|r| r.len).sum();
        let largest = self
            .free_ranges
            .iter()
            .map(|r| r.len)
            .max()
            .unwrap_or(0);
        FragmentationMetrics {
            free_range_count: self.free_ranges.len() as u32,
            largest_free_range: largest,
            high_watermark: self.high_watermark,
            used: self.capacity - free_total,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_pool_one_range() {
        let p = FreeListPool::new(1024);
        assert_eq!(p.free_ranges.len(), 1);
        assert_eq!(p.free_ranges[0], FreeRange { start: 0, len: 1024 });
        assert_eq!(p.high_watermark(), 0);
    }

    #[test]
    fn alloc_returns_first_fit() {
        let mut p = FreeListPool::new(1024);
        assert_eq!(p.alloc(64), Some(0));
        assert_eq!(p.alloc(64), Some(64));
        assert_eq!(p.high_watermark(), 128);
    }

    #[test]
    fn alloc_shrinks_or_removes_range() {
        let mut p = FreeListPool::new(100);
        let _ = p.alloc(40);
        // Single range remains: [40..100), len 60
        assert_eq!(p.free_ranges, vec![FreeRange { start: 40, len: 60 }]);
        let _ = p.alloc(60);
        // Exact fit removes the range entirely.
        assert!(p.free_ranges.is_empty());
        assert_eq!(p.alloc(1), None);
    }

    #[test]
    fn alloc_zero_len_no_op() {
        let mut p = FreeListPool::new(10);
        assert_eq!(p.alloc(0), Some(0));
        assert_eq!(p.high_watermark(), 0);
        assert_eq!(p.free_ranges, vec![FreeRange { start: 0, len: 10 }]);
    }

    #[test]
    fn free_does_not_coalesce_eagerly() {
        let mut p = FreeListPool::new(100);
        let a = p.alloc(20).unwrap();
        let b = p.alloc(20).unwrap();
        p.free(a, 20);
        p.free(b, 20);
        // Two free ranges sit at the head, plus the residual tail.
        assert_eq!(p.free_ranges.len(), 3);
    }

    #[test]
    fn coalesce_merges_adjacent_ranges() {
        let mut p = FreeListPool::new(100);
        let a = p.alloc(20).unwrap();
        let b = p.alloc(20).unwrap();
        let c = p.alloc(20).unwrap();
        p.free(b, 20);
        p.free(a, 20);
        p.free(c, 20);
        p.coalesce();
        // Single range covers [0..100).
        assert_eq!(p.free_ranges, vec![FreeRange { start: 0, len: 100 }]);
    }

    #[test]
    fn coalesce_keeps_disjoint_ranges() {
        let mut p = FreeListPool::new(100);
        let _a = p.alloc(20).unwrap(); // 0..20 in use
        let b = p.alloc(20).unwrap(); //  20..40 in use
        let _c = p.alloc(20).unwrap(); // 40..60 in use
        p.free(b, 20);
        p.coalesce();
        // [20..40) free + [60..100) tail. Two disjoint ranges.
        assert_eq!(p.free_ranges.len(), 2);
    }

    #[test]
    fn fragmentation_metrics_after_churn() {
        let mut p = FreeListPool::new(100);
        for _ in 0..5 {
            let _ = p.alloc(20);
        }
        // Free every other slot.
        p.free(0, 20);
        p.free(40, 20);
        p.free(80, 20);
        let m = p.fragmentation_metrics();
        assert_eq!(m.high_watermark, 100);
        assert_eq!(m.used, 40);
        assert_eq!(m.free_range_count, 3);
        assert_eq!(m.largest_free_range, 20);
    }

    #[test]
    fn alloc_after_partial_free_uses_first_fit() {
        let mut p = FreeListPool::new(100);
        let a = p.alloc(40).unwrap();
        let _b = p.alloc(40).unwrap();
        p.free(a, 40);
        p.coalesce();
        // First-fit should reuse [0..40), not extend the tail.
        assert_eq!(p.alloc(20), Some(0));
        assert_eq!(p.high_watermark(), 80);
    }

    #[test]
    fn alloc_fails_when_no_range_fits() {
        let mut p = FreeListPool::new(100);
        let _a = p.alloc(40).unwrap();
        let _b = p.alloc(40).unwrap();
        // Remaining: [80..100) = 20 elements.
        assert_eq!(p.alloc(30), None);
        assert_eq!(p.alloc(20), Some(80));
    }
}
