//! The physical page pool and its table (#866).
//!
//! Marking answers *which pages does this frame need*. This answers
//! *where does each of them live*, and the two run in the **same
//! dispatch**: `mark_bit` already reports the thread that flipped a
//! page's bit from 0 to 1, so claiming a physical slot there is one
//! `atomicAdd` on a rare branch. No second pass, and nothing walks the
//! virtual space.
//!
//! # What the table is, and why the flat answer is dead
//!
//! The arithmetic is in `page_table.wgsl` next to the code that depends
//! on it, in full. The short version: 101 lights and a sun address
//! **28 409 856** virtual pages. One bit each is 3.4 MiB and that is the
//! mark bitmap; one `u32` each is **108 MiB — 42 % of the 256 MiB pool
//! it would index**. So the table is sized to what is RESIDENT rather
//! than to what is addressable: `2 x pages` entries of open addressing,
//! **64 KiB** at Epic's 4096-page pool.
//!
//! # The pool is SLICED between the cameras
//!
//! One editor frame draws the same world twice. A clipmap is centred on
//! ITS camera, so the two need different pages — and sharing one bump
//! allocator means the camera that runs first can take every slot.
//!
//! So each camera owns [`PoolConfig::slice`] pages, and the atlas is an
//! **array with a layer each**: a layer is an attachment a camera clears
//! on its own, which is what lets one refill its pages while the other
//! is still sampling last frame's. The budget does not multiply — a
//! layer is `pages / views` rounded up to a square.

use super::PageConfig;

/// `KOOCH_SHADOW_POOL_PAGES`, read once.
///
/// An environment variable and **not** a `.rendersettings` field, on
/// purpose: #477 asks that nothing on the shadow side grow a public
/// setting before the pool's shape is decided, and a knob that sizes an
/// atlas nobody allocates yet would promise memory nothing spends. It
/// becomes a setting when the raster does.
pub fn pages_from_environment() -> u32 {
    static PAGES: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *PAGES.get_or_init(|| {
        std::env::var("KOOCH_SHADOW_POOL_PAGES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_PAGES)
            .clamp(PAGES_RANGE.0, PAGES_RANGE.1)
    })
}

/// What the pool is sized to when nobody says otherwise.
///
/// 🔴 Half of Epic's 4096, and the reason is that the atlas is now
/// REAL: 4096 pages at `Depth32Float` is 256 MiB, and this engine's
/// existing fixed shadow allocations are 152 MiB. 2048 pages is **128
/// MiB — less than what stands today for four casting lights**, which
/// is the comparison that matters on a handheld with shared memory.
///
/// The measurement it has to hold: 1681 pages for a hundred and one
/// lights at 400x400. `KOOCH_SHADOW_POOL_PAGES` raises it to Epic's
/// figure, or past it.
pub const DEFAULT_PAGES: u32 = 2048;

/// What the pool may be sized to.
///
/// The floor is absurdly small on purpose: overflow is the one failure
/// mode nobody recognises by sight — Epic's own shows up as
/// checkerboard corruption — so a test has to be able to fill the pool
/// deliberately. The ceiling is where Epic's tuning notes say the pool
/// starts thrashing rather than helping.
pub const PAGES_RANGE: (u32, u32) = (4, 8192);

/// Views a single pool may be sliced between.
///
/// The editor draws two and a game draws one. The ceiling is a guard
/// rather than a design: a slice thinner than a handful of pages is a
/// view with no shadows at all, and a silent one.
pub const VIEWS_RANGE: (u32, u32) = (1, 8);

/// How the physical pool is laid out.
///
/// 🔴 `views` is part of the layout and not a detail of the caller. The
/// atlas is one array texture with a layer per view, so the number of
/// views decides the layer size — change it and the whole thing is a
/// different texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolConfig {
    /// Physical pages the budget asks for, across every view. Epic's
    /// default is 4096; see [`POOL_PAGES`](super::POOL_PAGES).
    pub pages: u32,
    /// Cameras sharing it.
    pub views: u32,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            pages: pages_from_environment(),
            views: 1,
        }
    }
}

impl PoolConfig {
    /// Cameras the pool is really sliced between — `views`, clamped.
    ///
    /// 🔴 Read this and never the field. The atlas clamps its layer
    /// count and the mark bitmap sizes itself from the same number; two
    /// readings of it that disagree put a camera on a layer the texture
    /// does not have.
    pub fn slices(&self) -> u32 {
        self.clamped().1
    }

    /// The same pool, sliced between `views` cameras.
    pub fn with_views(self, views: u32) -> Self {
        Self {
            views: views.clamp(VIEWS_RANGE.0, VIEWS_RANGE.1),
            ..self
        }
    }

    fn clamped(&self) -> (u32, u32) {
        (
            self.pages.clamp(PAGES_RANGE.0, PAGES_RANGE.1),
            self.views.clamp(VIEWS_RANGE.0, VIEWS_RANGE.1),
        )
    }

    /// Entries in the hash table.
    ///
    /// Twice the pool, rounded up to a power of two, so the load factor
    /// never passes 0.5 and the mask is an `and` rather than a modulo.
    /// The expected probe count at that load is under two.
    ///
    /// Sized against [`Self::total`] rather than `pages`: every view's
    /// entries live in the SAME table, keyed by a page id that carries
    /// the view, and a table sized for one view's worth would run at a
    /// load factor of 1.0 the moment a second viewport opened.
    pub fn entries(&self) -> u32 {
        self.total().next_power_of_two() * 2
    }

    /// Pages across one view's layer, in both axes.
    ///
    /// Square, because a long strip wastes the second dimension of
    /// every texture limit there is: 8192 pages in a row is past
    /// `max_texture_dimension_2d` the moment a page is more than 8
    /// texels.
    pub fn per_row(&self) -> u32 {
        let (pages, views) = self.clamped();
        ((pages.div_ceil(views)) as f64).sqrt().ceil().max(1.0) as u32
    }

    /// Pages one view owns — its slice of the pool, and the capacity
    /// every per-view counter is read against.
    pub fn slice(&self) -> u32 {
        self.per_row().pow(2)
    }

    /// Pages the pool really holds, which is the slice times the views.
    ///
    /// 🔴 Not `pages`: a layer is square, so the budget is rounded UP to
    /// the next square rather than trimmed to fit. Asking for 2048 across
    /// one view buys 2116. The number the atlas costs is this one, and
    /// it is the one reported.
    pub fn total(&self) -> u32 {
        self.slice() * self.clamped().1
    }

    /// Where a view's slice starts, in global slot numbers.
    ///
    /// Slots are global so that a table entry is self-describing: the
    /// layer is `slot / slice` and the texel origin comes from the
    /// remainder. Nothing that samples a page has to be told which view
    /// filled it.
    pub fn base(&self, view: u32) -> u32 {
        view.min(self.clamped().1 - 1) * self.slice()
    }

    /// What the atlas costs, at `Depth32Float`.
    pub fn atlas_bytes(&self, config: PageConfig) -> u64 {
        self.total() as u64 * config.page_bytes()
    }

    /// What the table costs, which is the number the flat answer lost
    /// to: keys and slots, one `u32` each.
    pub fn table_bytes(&self) -> u64 {
        self.entries() as u64 * 8
    }
}

/// The page table's two buffers, rebuilt when the pool is resized.
///
/// 🔴 Emptied **one view at a time, by a pass** — `clear_view` in
/// `page_mark.wgsl` — and not by `clear_buffer`. The table is shared
/// between the cameras and the raster is fused with the shading, so a
/// view samples an atlas a frame old: wiping the whole table at the top
/// of a frame leaves whichever view marks second reading what the first
/// one just erased. [`Self::clear`] remains for the one caller that
/// really does own the whole table — a test.
///
/// ⚠️ Nothing is cached across frames, which is the optimisation real
/// VSM exists for: UE5 keeps a page alive until it goes unrequested for
/// `MaxPageAgeSinceLastRequest` frames and allocates in LRU order, so a
/// static shadow is rasterised once. That needs an eviction policy and
/// an invalidation rule, and it is the next machine.
pub struct PagePool {
    keys: wgpu::Buffer,
    slots: wgpu::Buffer,
    config: PoolConfig,
}

impl PagePool {
    pub fn new(device: &wgpu::Device, config: PoolConfig) -> Self {
        Self {
            keys: table_buffer(device, "shadow_page_keys", config.entries()),
            slots: table_buffer(device, "shadow_page_slots", config.entries()),
            config,
        }
    }

    pub fn config(&self) -> PoolConfig {
        self.config
    }

    /// Resizes if the pool changed, and reports whether it did.
    pub fn resize(&mut self, device: &wgpu::Device, config: PoolConfig) -> bool {
        if config == self.config {
            return false;
        }
        *self = Self::new(device, config);
        true
    }

    pub fn keys(&self) -> &wgpu::Buffer {
        &self.keys
    }

    pub fn slots(&self) -> &wgpu::Buffer {
        &self.slots
    }

    /// Empties the WHOLE table, every view's entries included.
    ///
    /// ⚠️ Only correct where there is exactly one view — a test, or a
    /// pool being rebuilt. The per-frame reset is `clear_view`. Only
    /// the keys need it: a slot is never read without its key matching
    /// first.
    pub fn clear(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.clear_buffer(&self.keys, 0, None);
    }
}

fn table_buffer(device: &wgpu::Device, label: &str, entries: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: entries as u64 * 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

/// What the allocator did, alongside what marking found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolCounts {
    /// Pages that asked for a physical slot.
    ///
    /// 🔴 **Fewer than `MarkCounts::resident`, on purpose.** Marking a
    /// local light's page is a measurement — this track exists to say
    /// what a hundred casting lights would cost — but only the sun's
    /// pages are rasterised, so only they claim. The difference is
    /// exactly the pages the frame would need if the local raster
    /// existed, which is the number the whole census was for.
    pub claims: u32,
    /// Claims past the end of the pool. Non-zero means pages went
    /// unshadowed this frame; Epic's own overflow shows up as
    /// checkerboard corruption, so the counter exists to name it before
    /// anyone has to recognise it by sight.
    pub overflow: u32,
    /// Inserts that walked `PAGE_PROBES` slots without finding room.
    ///
    /// ⚠️ Distinct from `overflow`: the pool had space and the TABLE
    /// did not. At a load factor of 0.5 this should be zero, and any
    /// other number is a statement about the hash rather than about the
    /// scene.
    pub probes: u32,
    /// Physical pages THIS VIEW owns, so the two numbers above are
    /// readable without knowing how the build was configured.
    ///
    /// 🔴 The slice, not the whole pool. A view cannot spend another
    /// view's pages, so measuring it against the total would report a
    /// budget half spent at the moment it ran out.
    pub capacity: u32,
}

impl PoolCounts {
    /// Slots actually handed out, which is `claims` capped by the pool.
    pub fn allocated(&self) -> u32 {
        self.claims.min(self.capacity)
    }

    /// Pages the frame marked and could not spend a slot on, because
    /// nothing draws them yet.
    pub fn unspent(&self, resident: u32) -> u32 {
        resident.saturating_sub(self.claims)
    }

    /// How full the pool ran, as a percentage.
    pub fn load(&self) -> f32 {
        if self.capacity == 0 {
            return 0.0;
        }
        self.allocated() as f32 / self.capacity as f32 * 100.0
    }
}
