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

/// Words per table entry — the slot, its age, and its place in the
/// compacted page list. Mirrors `PAGE_CELL` in
/// `page_table.wgsl`, which is where the reason lives.
pub const PAGE_CELL: u32 = 3;

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
}

/// How long a page outlives the frame that asked for it.
///
/// # 🔴 Zero is the DANGEROUS value, which is the opposite of the guess
///
/// The instinct is that keeping a page longer is the risky half — a
/// resident page holds the depth of the last frame that drew it, so
/// surely a short age is the safe default. It is exactly backwards here,
/// and it took a measurement to see why.
///
/// The raster redraws every resident page every frame, so a page's
/// CONTENT is never stale no matter how long it lives. What a short age
/// costs is the page's ADDRESS: at zero, every page is evicted and
/// re-taken each frame, its slot comes back off a free list in whatever
/// order the GPU's threads got there, and the same page lands somewhere
/// new. `a_resident_page_keeps_its_slot` measures it — page 450494
/// moved from slot 2 to slot 1 with the camera and the sun standing
/// still.
///
/// That matters because `vbuf64.render` rasterises and shades in ONE
/// pass: the shading samples an atlas a frame old while reading this
/// frame's table. The two agree only while a page's slot holds. Before
/// persistence they agreed by accident — the allocator was a bump from
/// zero and handed the same page the same slot. A free list has no such
/// order, and the symptom is what the user saw: artefacts that flash and
/// vanish whenever the camera or the lights move.
///
/// So the default is long. A page nothing stops asking for is never
/// freed, never moves, and the fused pass keeps its guarantee.
/// `KOOCH_SHADOW_PAGE_AGE` moves it; zero is reachable and is what the
/// eviction tests use, but it is not a setting to ship.
///
/// ⚠️ The thing this genuinely still waits on is the NEXT step, not this
/// one: rasterising only the pages that changed. That needs something to
/// mark the pages a moving caster passed through, and until it exists
/// the raster's answer is to redraw them all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolLife {
    /// This frame's index, counting up for the process's lifetime.
    pub frame: u32,
    /// Frames a page may go unrequested before it is evicted.
    pub max_age: u32,
    /// Set when the pool was just rebuilt, which evicts everything: a
    /// slot recorded against the old atlas names a different page in the
    /// new one.
    pub rebuilt: bool,
}

impl Default for PoolLife {
    fn default() -> Self {
        Self {
            frame: 0,
            max_age: age_from_environment(),
            rebuilt: false,
        }
    }
}

impl PoolLife {
    /// The uniform's `life` field.
    pub fn words(&self) -> [u32; 4] {
        [self.frame, self.max_age, u32::from(self.rebuilt), 0]
    }
}

/// Frames a page survives unrequested when nobody says otherwise.
///
/// A second at 60 Hz. Long enough that a camera sweeping across a scene
/// and back finds its pages still there; short enough that the pool is
/// not holding a minute of somewhere else.
pub const DEFAULT_MAX_AGE: u32 = 60;

/// `KOOCH_SHADOW_PAGE_AGE`, read once. See [`PoolLife`] for why the
/// default is long rather than short.
pub fn age_from_environment() -> u32 {
    static AGE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *AGE.get_or_init(|| {
        std::env::var("KOOCH_SHADOW_PAGE_AGE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MAX_AGE)
            .min(1024)
    })
}

/// The page table and the allocator's state, rebuilt when the pool is
/// resized.
///
/// 🔴 Aged **one view at a time, by a pass** — `age_view` in
/// `page_mark.wgsl` — and not by `clear_buffer`. The table is shared
/// between the cameras and the raster is fused with the shading, so a
/// view samples an atlas a frame old: wiping the whole table at the top
/// of a frame leaves whichever view marks second reading what the first
/// one just erased. [`Self::clear`] remains for the one caller that
/// really does own the whole table — a test, or a rebuild.
///
/// # 🔴 It PERSISTS, and that is what the third buffer is for
///
/// A page is not freed because a frame ended. It is freed because
/// `max_age` frames passed with nothing asking for it — Epic's
/// `MaxPageAgeSinceLastRequest` — so a shadow that does not change is
/// rasterised once and then read for as long as the camera keeps
/// wanting it. `ages` records when each entry was last requested and
/// `alloc` holds the per-view free list the eviction pushes onto.
///
/// ⚠️ What that buys has a matching cost: a resident page's depth is as
/// old as the last time it was drawn. Nothing here invalidates a page
/// because a caster inside it MOVED, so with `max_age` above zero a
/// moving object leaves its shadow behind. That is the next machine and
/// [`PoolLife`] is the knob that keeps it off until it exists.
pub struct PagePool {
    /// The FLAT page table: `PAGE_CELL` words per VIRTUAL page, indexed
    /// by the page id itself. See `page_table.wgsl` for the entry
    /// layout and for why the hash this replaced is gone.
    slots: wgpu::Buffer,

    /// `[high, free_count, free_slots...]` per view — see `alloc_base`
    /// in the shader.
    alloc: wgpu::Buffer,
    config: PoolConfig,
    /// Virtual pages the table holds entries for, across every view.
    ///
    /// 🔴 A function of the LIGHT COUNT, not of the pool: the table is
    /// one entry per addressable page. [`Self::ensure_entries`] grows
    /// it, and the marker owns the number because the marker owns the
    /// address space.
    entries: u32,
}

impl PagePool {
    pub fn new(device: &wgpu::Device, config: PoolConfig) -> Self {
        Self {
            slots: table_buffer(device, "shadow_page_cells", PAGE_CELL),
            alloc: table_buffer(
                device,
                "shadow_page_alloc",
                config.slices() * (config.slice() + 2),
            ),
            config,
            entries: 1,
        }
    }

    pub fn config(&self) -> PoolConfig {
        self.config
    }

    /// Virtual pages the table holds entries for.
    pub fn entries(&self) -> u32 {
        self.entries
    }

    /// What the table costs: [`PAGE_CELL`] words per virtual page. The
    /// number that used to make a flat table impossible — 108 MiB over
    /// the full chain — and that the floored local stride brought to a
    /// few MiB. See `page_table.wgsl`.
    pub fn table_bytes(&self) -> u64 {
        self.entries as u64 * PAGE_CELL as u64 * 4
    }

    /// Resizes if the pool changed, and reports whether it did.
    pub fn resize(&mut self, device: &wgpu::Device, config: PoolConfig) -> bool {
        if config == self.config {
            return false;
        }
        let entries = self.entries;
        *self = Self::new(device, config);
        self.ensure_entries(device, entries);
        true
    }

    /// Grows the table to `entries` virtual pages, and reports whether
    /// the buffers were replaced — in which case every entry is gone
    /// and the caller flags a rebuild.
    ///
    /// 🔴 The allocator goes with the table. A free list that outlives
    /// the table it was built for hands out slots two entries both
    /// believe they own, and the second one wins silently.
    pub fn ensure_entries(&mut self, device: &wgpu::Device, entries: u32) -> bool {
        if entries <= self.entries {
            return false;
        }
        self.slots = table_buffer(device, "shadow_page_cells", entries * PAGE_CELL);
        self.alloc = table_buffer(
            device,
            "shadow_page_alloc",
            self.config.slices() * (self.config.slice() + 2),
        );
        self.entries = entries;
        true
    }

    pub fn slots(&self) -> &wgpu::Buffer {
        &self.slots
    }

    pub fn alloc(&self) -> &wgpu::Buffer {
        &self.alloc
    }

    /// Empties the WHOLE table and resets the allocator, every view's
    /// entries included.
    ///
    /// ⚠️ Only correct where there is exactly one view — a test — or
    /// where the pool has just been rebuilt and no entry in it names a
    /// slot that still exists. The per-frame reset is `age_view`.
    ///
    /// 🔴 `alloc` has to go with the table. A free list that outlives
    /// the table it was built for hands out slots two entries both
    /// believe they own, and the second one wins silently.
    pub fn clear(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.clear_buffer(&self.slots, 0, None);
        encoder.clear_buffer(&self.alloc, 0, None);
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
    /// Requests that found their page ALREADY RESIDENT, so nothing was
    /// allocated and nothing has to be rasterised again.
    ///
    /// 🔴 The number persistence exists to produce. `claims` against
    /// this is the whole reading: a frame where almost every request is
    /// a reuse is a frame whose shadow atlas is mostly last frame's,
    /// which is what makes a hundred casting lights affordable at all.
    pub reused: u32,
    /// Resident pages this view carried over — requested recently enough
    /// not to be evicted.
    pub alive: u32,
    /// Pages freed this frame for going unrequested past `max_age`.
    pub evicted: u32,
    /// Slots the free list could not hold, which is a double free.
    /// 🔴 Always zero, or the allocator is wrong.
    pub leaked: u32,
    /// Claims past the end of the pool. Non-zero means pages went
    /// unshadowed this frame; Epic's own overflow shows up as
    /// checkerboard corruption, so the counter exists to name it before
    /// anyone has to recognise it by sight.
    pub overflow: u32,
    /// Physical pages THIS VIEW owns, so the two numbers above are
    /// readable without knowing how the build was configured.
    ///
    /// 🔴 The slice, not the whole pool. A view cannot spend another
    /// view's pages, so measuring it against the total would report a
    /// budget half spent at the moment it ran out.
    pub capacity: u32,
}

impl PoolCounts {
    /// Slots the pool is holding after this frame: what survived the
    /// ageing, plus what was allocated on top of it.
    ///
    /// 🔴 NOT `claims`. Since the pool persists, `claims` is what was
    /// NEW this frame — a number that falls to zero on a still camera
    /// while the pool stays exactly as full as it was.
    pub fn allocated(&self) -> u32 {
        (self.alive + self.claims).min(self.capacity)
    }

    /// Pages the frame marked and could not spend a slot on, because
    /// nothing draws them yet.
    pub fn unspent(&self, resident: u32) -> u32 {
        resident.saturating_sub(self.claims + self.reused)
    }

    /// Requests that reached the pool at all — a reuse or an allocation.
    pub fn requests(&self) -> u32 {
        self.claims + self.reused
    }

    /// How much of this frame's work the pool answered from what it
    /// already had, as a percentage.
    ///
    /// 🔴 The one number that says whether persistence is doing
    /// anything. 100 % is a frame that rasterised no shadow at all
    /// because every page it wanted was already drawn.
    pub fn hit_rate(&self) -> f32 {
        let requests = self.requests();
        if requests == 0 {
            return 0.0;
        }
        self.reused as f32 / requests as f32 * 100.0
    }

    /// How full the pool ran, as a percentage.
    pub fn load(&self) -> f32 {
        if self.capacity == 0 {
            return 0.0;
        }
        self.allocated() as f32 / self.capacity as f32 * 100.0
    }
}
