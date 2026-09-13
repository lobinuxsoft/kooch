//! A dense, type-erased array of one component type.

use std::alloc::{self, Layout};
use std::ptr::NonNull;

/// Every value of one component type, packed end to end.
pub struct Column {
    /// Dangling **but aligned for the item type** while nothing is allocated. `NonNull::dangling()`
    /// would not do: it is aligned for `u8`, and a zero-sized item with a larger alignment would
    /// then be read from a misaligned address.
    data: NonNull<u8>,
    len: usize,
    /// Always 0 for a zero-sized item, which never allocates.
    capacity: usize,
    /// Bytes from one item to the next.
    stride: usize,
    align: usize,
    /// `None` when the item type has no destructor, so the common case
    /// costs no indirect call per row.
    drop_one: Option<unsafe fn(*mut u8)>,
}

// SAFETY: a column only ever holds values of one component type, and this
// engine's components are `Send + Sync` — `Column::of` requires it. The raw
// pointer is an implementation detail of an owned allocation.
unsafe impl Send for Column {}
unsafe impl Sync for Column {}

impl Column {
    /// An empty column for values of type `T`. Allocates nothing.
    pub fn of<T: Send + Sync + 'static>() -> Self {
        let layout = Layout::new::<T>();
        let align = layout.align();
        Self {
            // A pointer with no provenance, at the item's own alignment.
            data: NonNull::new(std::ptr::without_provenance_mut(align))
                .expect("an alignment is never zero"),
            len: 0,
            capacity: 0,
            stride: layout.size(),
            align,
            drop_one: std::mem::needs_drop::<T>().then_some(drop_in_place::<T>),
        }
    }

    /// How many items the column holds.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the column holds nothing.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// How many items fit before the next reallocation.
    ///
    /// Stays 0 forever for a zero-sized item: there is nothing to hold.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Appends `value`, returning the row it landed in.
    pub unsafe fn push<T>(&mut self, value: T) -> usize {
        debug_assert_eq!(Layout::new::<T>().align(), self.align, "wrong item type");
        debug_assert_eq!(Layout::new::<T>().size(), self.stride, "wrong item type");
        self.reserve_one();
        let row = self.len;
        // SAFETY: `reserve_one` guarantees room at `row`, and the caller
        // guarantees `T` is the item type.
        unsafe { std::ptr::write(self.row_ptr(row).cast::<T>(), value) };
        self.len += 1;
        row
    }

    /// The value at `row`, or `None` if the row is past the end.
    pub unsafe fn get<T>(&self, row: usize) -> Option<&T> {
        if row >= self.len {
            return None;
        }
        // SAFETY: `row` is in bounds and the caller guarantees the type.
        Some(unsafe { &*self.row_ptr(row).cast::<T>() })
    }

    /// The value at `row`, mutably.
    pub unsafe fn get_mut<T>(&mut self, row: usize) -> Option<&mut T> {
        if row >= self.len {
            return None;
        }
        // SAFETY: `row` is in bounds, we hold `&mut self`, and the caller
        // guarantees the type.
        Some(unsafe { &mut *self.row_ptr(row).cast::<T>() })
    }

    /// The value at `row` as a raw pointer, **without forming a reference to the column**.
    pub unsafe fn value_ptr<T>(&self, row: usize) -> Option<*mut T> {
        if row >= self.len {
            return None;
        }
        Some(self.row_ptr(row).cast::<T>())
    }

    /// Drops the item at `row` and moves the last item into its place.
    pub fn swap_remove(&mut self, row: usize) {
        self.vacate(row, true);
    }

    /// Moves the value at `row` into `dst`, appending it there, and vacates the row here **without
    /// running its destructor**.
    pub unsafe fn move_row_to(&mut self, row: usize, dst: &mut Column) -> usize {
        assert!(row < self.len, "row {row} is past the end ({})", self.len);
        debug_assert_eq!(self.stride, dst.stride, "columns hold different types");
        debug_assert_eq!(self.align, dst.align, "columns hold different types");

        dst.reserve_one();
        let landed = dst.len;
        // SAFETY: `reserve_one` made room at `landed`, the two columns are
        // distinct allocations, and the caller guarantees the shared type.
        unsafe {
            std::ptr::copy_nonoverlapping(self.row_ptr(row), dst.row_ptr(landed), self.stride)
        };
        dst.len += 1;

        self.vacate(row, false);
        landed
    }

    /// Frees `row`, pulling the last row into it.
    fn vacate(&mut self, row: usize, run_drop: bool) {
        assert!(row < self.len, "row {row} is past the end ({})", self.len);
        let last = self.len - 1;
        // SAFETY: both rows are in bounds, and the two regions cannot
        // overlap because they are distinct rows of the same stride.
        unsafe {
            if run_drop && let Some(drop_one) = self.drop_one {
                drop_one(self.row_ptr(row));
            }
            if row != last {
                std::ptr::copy_nonoverlapping(self.row_ptr(last), self.row_ptr(row), self.stride);
            }
        }
        self.len = last;
    }

    /// Drops every item, keeping the allocation.
    pub fn clear(&mut self) {
        if let Some(drop_one) = self.drop_one {
            for row in 0..self.len {
                // SAFETY: every row below `len` holds a live item.
                unsafe { drop_one(self.row_ptr(row)) };
            }
        }
        self.len = 0;
    }

    #[inline]
    fn row_ptr(&self, row: usize) -> *mut u8 {
        // SAFETY: for a sized item the caller has checked the bound; for a
        // zero-sized one the stride is 0 and this is the aligned dangling
        // pointer, which is what a ZST read wants.
        unsafe { self.data.as_ptr().add(row * self.stride) }
    }

    fn reserve_one(&mut self) {
        if self.stride == 0 || self.len < self.capacity {
            return;
        }
        let new_capacity = if self.capacity == 0 {
            4
        } else {
            self.capacity * 2
        };
        let new_layout = self.layout_for(new_capacity);
        let ptr = if self.capacity == 0 {
            // SAFETY: the layout has a non-zero size — `stride != 0` here.
            unsafe { alloc::alloc(new_layout) }
        } else {
            // SAFETY: `data` came from this allocator with the old layout.
            unsafe {
                alloc::realloc(
                    self.data.as_ptr(),
                    self.layout_for(self.capacity),
                    new_layout.size(),
                )
            }
        };
        self.data = NonNull::new(ptr).unwrap_or_else(|| alloc::handle_alloc_error(new_layout));
        self.capacity = new_capacity;
    }

    fn layout_for(&self, capacity: usize) -> Layout {
        Layout::from_size_align(
            self.stride
                .checked_mul(capacity)
                .expect("column capacity overflows a layout"),
            self.align,
        )
        .expect("column layout is valid by construction")
    }
}

impl Drop for Column {
    fn drop(&mut self) {
        self.clear();
        if self.stride != 0 && self.capacity != 0 {
            // SAFETY: allocated by this type with exactly this layout.
            unsafe { alloc::dealloc(self.data.as_ptr(), self.layout_for(self.capacity)) };
        }
    }
}

/// The monomorphised destructor a column stores when its item needs one.
unsafe fn drop_in_place<T>(ptr: *mut u8) {
    // SAFETY: guaranteed by the caller.
    unsafe { std::ptr::drop_in_place(ptr.cast::<T>()) };
}

#[cfg(test)]
mod tests;
