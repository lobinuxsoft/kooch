//! Names the call site of every collection that grows past 8 MiB (#1592).
//!
//! 🔴 Temporary. The census showed one buffer doubling 7 → 15 → 31 → 62 MiB while every counter
//! it knew of stayed flat; a growing `Vec` reaches that size only through `realloc`, which is what
//! this watches. Comes out when #1592 closes.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// Below this a growth is ordinary; a leak that doubles reaches it within minutes.
const FLOOR: usize = 8 * 1024 * 1024;

pub struct BigRealloc;

thread_local! {
    /// Set while a report is being written: capturing and printing a backtrace allocates, and those
    /// allocations must go straight through.
    static INSIDE: Cell<bool> = const { Cell::new(false) };
}

// SAFETY: every call is forwarded unchanged to `System`; the report only reads sizes.
unsafe impl GlobalAlloc for BigRealloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size >= FLOOR && new_size > layout.size() {
            report(layout.size(), new_size);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

fn report(from: usize, to: usize) {
    // `try_with`: a thread being torn down has no thread-locals left.
    let _ = INSIDE.try_with(|inside| {
        if inside.replace(true) {
            return;
        }
        let trace = std::backtrace::Backtrace::force_capture();
        tracing::warn!(
            target: "kooch_editor::big_realloc",
            from_mb = from >> 20,
            to_mb = to >> 20,
            "a collection grew past 8 MiB\n{trace}",
        );
        inside.set(false);
    });
}
