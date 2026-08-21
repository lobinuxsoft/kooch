//! What a page pool would actually hold — the census (#866).
//!
//! # Why this is the first thing in the issue
//!
//! #866 refuses to pick a page count, a page size or an atlas format
//! from a whiteboard, and says so: *"the first task in this issue is a
//! measurement, not an allocation"*. This is that measurement. It
//! answers one question — **how many pages would a real frame make
//! resident** — and it answers it for a sweep of configurations, so the
//! allocation that follows is read off a table instead of guessed.
//!
//! The number it is measured against is today's **152 MiB**: 128 for the
//! cascade + spot array ([`ShadowAtlas`](super::atlas::ShadowAtlas)) and
//! 24 for the point cubes ([`PointShadowCubes`](super::cube)), standing
//! whether or not the frame contains a shadow-casting light.
//!
//! # Why it runs on the CPU
//!
//! The marking pass this previews belongs on the GPU, over the froxel
//! grid that already runs — and that is #477's page marking, where it
//! *allocates*. Here it only counts, so it costs no device, no shader
//! and no handheld cycles, and it can be a test. What it buys twice:
//! the same walk becomes the **oracle** the GPU pass is checked against,
//! which is the position `ClusterGrid::z_slice` already holds against
//! `cluster_z_slice` in WGSL.
//!
//! ⚠️ It is a preview, so it inherits the froxel grid's conservatism and
//! adds its own. Both are named at the site that introduces them.
//!
//! # The structure it previews
//!
//! A page is the unit of residency: a fixed square of shadow texels out
//! of one atlas. A local light's virtual map is a mip chain — Epic's
//! spot is one 16k map with mips and its point is six of them, and only
//! the **directional** light gets a clipmap, because it is the one with
//! no position to bound it.
//!
//! Which level a cell reads is decided the same way UE5 decides it: by
//! the size of a shadow texel against the size of a screen pixel, at
//! that cell's distance from the light. That is what makes the memory a
//! function of the screen rather than of the sum of every light's worst
//! case.

use glam::{Mat4, UVec3, Vec2, Vec3, Vec4};

use kooch_lighting::ClusterGrid;

use super::point::{CUBE_FACES, POINT_SHADOW_NEAR_Z, face_view_proj};

/// How a virtual shadow map is diced into pages.
///
/// 🔴 Both numbers are the ones #866 declines to fix, which is why they
/// are a parameter of the census rather than a constant of the engine.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PageConfig {
    /// A page's side, in texels. UE5 uses 128.
    pub page: u32,
    /// A local light's virtual map at level 0, in texels. UE5 uses
    /// 16384, of which a frame residents a few thousand pages.
    pub virtual_size: u32,
}

impl Default for PageConfig {
    /// Epic's, verified against the UE 5.8 documentation rather than
    /// quoted second-hand: *"they have a virtual resolution of 16k x 16k
    /// pixels"* and *"VSMs split the shadow map into tiles (or Pages)
    /// that are 128x128 each"*.
    fn default() -> Self {
        Self {
            page: 128,
            virtual_size: 16384,
        }
    }
}

/// Pages in Unreal's physical pool by default —
/// `r.Shadow.Virtual.MaxPhysicalPages`.
///
/// 🔴 **The budget line, and it is one number for the whole scene**:
/// every light, the sun included, allocates out of this. Epic's own
/// tuning advice puts 4096 at *"too tight for a real open world"*, 6144
/// as the open-world recommendation and 8192 as the point where it
/// thrashes — so a census landing in that band is landing where a
/// shipped engine's pool sits, and one landing far above it is
/// describing an allocation nobody ships.
///
/// Overflow is not graceful: Epic's page-pool overflow shows up as
/// checkerboard corruption or missing shadows.
pub const POOL_PAGES: u32 = 4096;

/// The open-world pool Epic recommends over the default.
pub const POOL_PAGES_WIDE: u32 = 6144;

impl PageConfig {
    /// Levels in the chain, level 0 being the finest.
    ///
    /// The chain stops where a whole level is one page: past that a
    /// level cannot be paged, only allocated whole.
    pub fn levels(&self) -> u32 {
        let side = self.side(0).max(1);
        side.ilog2() + 1
    }

    /// Pages along one side of `level`.
    pub fn side(&self, level: u32) -> u32 {
        (self.virtual_size >> level).div_ceil(self.page).max(1)
    }

    /// Texels along one side of `level`.
    pub fn texels(&self, level: u32) -> u32 {
        (self.virtual_size >> level).max(self.page)
    }

    /// Pages in one face's whole chain — the stride between faces in the
    /// census bitmap.
    pub fn face_pages(&self) -> u32 {
        (0..self.levels()).map(|l| self.side(l).pow(2)).sum()
    }

    /// Where `level` starts inside one face's chain.
    fn level_base(&self, level: u32) -> u32 {
        (0..level).map(|l| self.side(l).pow(2)).sum()
    }

    /// What one page costs, at `Depth32Float`.
    pub fn page_bytes(&self) -> u64 {
        self.page as u64 * self.page as u64 * 4
    }
}

/// What kind of chain a light addresses.
///
/// 🔴 The three are not variations of one shape, and Epic's design is
/// explicit about it: a spot is **one** map with a mip chain, a point is
/// **six** of them, and only the directional light gets a **clipmap** —
/// because it is the one with no position, so nothing else bounds how
/// far its shadow has to reach.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum CensusKind {
    Point,
    Spot,
    /// The direction the light travels, normalised.
    Sun(Vec3),
}

/// A shadow-casting light, as the census needs it.
///
/// Deliberately not [`GpuLight`](kooch_lighting::GpuLight): the census
/// runs headless, over a scene document or over a test's own list, and
/// the fields below are all of a light that decides a page.
#[derive(Copy, Clone, Debug)]
pub struct CensusLight {
    pub position: Vec3,
    /// Metres. A sun has none, and stores [`f32::INFINITY`].
    pub range: f32,
    pub kind: CensusKind,
}

impl CensusLight {
    pub fn point(position: Vec3, range: f32) -> Self {
        Self {
            position,
            range,
            kind: CensusKind::Point,
        }
    }

    pub fn spot(position: Vec3, range: f32) -> Self {
        Self {
            position,
            range,
            kind: CensusKind::Spot,
        }
    }

    pub fn sun(direction: Vec3) -> Self {
        Self {
            position: Vec3::ZERO,
            range: f32::INFINITY,
            kind: CensusKind::Sun(direction.normalize_or_zero()),
        }
    }

    /// Faces this light's chain has.
    fn faces(&self) -> u32 {
        match self.kind {
            CensusKind::Point => CUBE_FACES as u32,
            CensusKind::Spot | CensusKind::Sun(_) => 1,
        }
    }
}

/// How a directional light's clipmap is nested.
///
/// Levels are centred on the viewer, each twice the extent of the last,
/// which is what keeps a page's on-screen size roughly constant with
/// distance. The reduction that makes it non-optional is in #866: a
/// 2.5 km draw distance is 200 trillion cells at the near resolution and
/// 20 million with a clipmap.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ClipmapConfig {
    /// Level 0's extent, in metres.
    pub base: f32,
    pub levels: u32,
}

impl Default for ClipmapConfig {
    /// Unreal's, read off the UE 5.8 documentation: *"by default,
    /// clipmap levels 6 through 22 are allocated"*, the finest
    /// *"covering 64 cm (2^6 cm) from the camera position"* and the
    /// broadest *"about 40 kilometers (2^22 cm)"*.
    ///
    /// Those are **radii**, so level 6's extent is 1.28 m and the
    /// seventeenth level's is 83.9 km. 🔴 And every level keeps the full
    /// 16k resolution — a clipmap level is not half of the last one, the
    /// way a mip is.
    fn default() -> Self {
        Self {
            base: 1.28,
            levels: 22 - 6 + 1,
        }
    }
}

impl ClipmapConfig {
    /// Level `level`'s extent, in metres.
    pub fn extent(&self, level: u32) -> f32 {
        self.base * (1u64 << level.min(40)) as f32
    }
}

/// The camera the frame is being censused for.
#[derive(Copy, Clone, Debug)]
pub struct CensusCamera {
    pub world_from_view: Mat4,
    pub clip_from_view: Mat4,
    pub viewport: Vec2,
}

impl CensusCamera {
    /// Clip back to world, which is what a cell's corners are found
    /// with.
    fn world_from_clip(&self) -> Mat4 {
        self.world_from_view * self.clip_from_view.inverse()
    }

    /// World metres one screen pixel covers at `depth`.
    ///
    /// 🔴 The density every level choice is made against, and it is
    /// read off the frustum rather than off the cell: a froxel's largest
    /// world extent is its **depth** — the slices are logarithmic, so a
    /// far cell is tens of metres deep and a fraction of that wide — and
    /// measuring a pixel against it asks for a shadow coarser than the
    /// screen by the froxel's own aspect. `2 * d * tan(fov/2) / height`
    /// is the same number with nothing to be wrong about.
    fn pixel_at(&self, depth: f32) -> f32 {
        let focal = self.clip_from_view.y_axis.y;
        if focal.abs() < f32::EPSILON {
            return 0.0;
        }
        2.0 * depth.abs() / (focal * self.viewport.y.max(1.0))
    }

    /// Where the camera is, which is what a clipmap is centred on.
    fn eye(&self) -> Vec3 {
        self.world_from_view.w_axis.truncate()
    }

    /// Where a view-space depth lands in NDC.
    ///
    /// Read off the projection rather than assumed, because Kóoch's is
    /// infinite and reversed (ADR 0002) and every closed form for it is
    /// one sign away from being wrong.
    fn ndc_z(&self, view_z: f32) -> f32 {
        let clip = self.clip_from_view * Vec4::new(0.0, 0.0, view_z, 1.0);
        if clip.w.abs() < f32::EPSILON {
            return 0.0;
        }
        clip.z / clip.w
    }
}

/// What one frame would make resident.
#[derive(Clone, Debug)]
pub struct PageCensus {
    config: PageConfig,
    clipmap: ClipmapConfig,
    /// One bit per page of every light's chain, which is the structure
    /// the GPU pass marks with `atomicOr` — the census is the same walk
    /// with the atomics taken out.
    marks: Vec<u64>,
    per_light: u32,
    resident: u32,
    /// Cell/light pairs the walk visited, so a page count can be read
    /// against the work that produced it.
    pairs: u32,
    /// Cells the walk marked from, which is every cell of the grid
    /// unless [`CensusFrame::surfaces`] narrowed it.
    cells: u32,
}

impl PageCensus {
    pub fn new(config: PageConfig, clipmap: ClipmapConfig, lights: usize) -> Self {
        // One stride for every light, sized for whichever chain is
        // longest. A per-kind stride would save bits and cost a prefix
        // sum to find a light's base — and this buffer is under two MiB
        // at Epic's configuration with a hundred lights.
        let local = config.face_pages() * CUBE_FACES as u32;
        let sun = clipmap.levels * config.side(0).pow(2);
        let per_light = local.max(sun);
        let bits = per_light as usize * lights.max(1);
        Self {
            config,
            clipmap,
            marks: vec![0; bits.div_ceil(64)],
            per_light,
            resident: 0,
            pairs: 0,
            cells: 0,
        }
    }

    /// Distinct pages the frame touched.
    pub fn resident(&self) -> u32 {
        self.resident
    }

    /// What those pages cost, in bytes.
    pub fn bytes(&self) -> u64 {
        self.resident as u64 * self.config.page_bytes()
    }

    pub fn pairs(&self) -> u32 {
        self.pairs
    }

    pub fn cells(&self) -> u32 {
        self.cells
    }

    /// Marks one page of a local light's mip chain.
    fn mark(&mut self, light: u32, face: u32, level: u32, x: u32, y: u32) -> bool {
        let side = self.config.side(level);
        let offset = face * self.config.face_pages()
            + self.config.level_base(level)
            + y.min(side - 1) * side
            + x.min(side - 1);
        self.set(light * self.per_light + offset)
    }

    /// Marks one page of the sun's clipmap.
    ///
    /// Every level is a full grid rather than half of the last, which is
    /// what a clipmap is and what a mip chain is not — so the offset is
    /// a multiply where [`Self::mark`]'s is a running sum.
    fn mark_sun(&mut self, light: u32, level: u32, x: u32, y: u32) -> bool {
        let side = self.config.side(0);
        let offset = level.min(self.clipmap.levels - 1) * side.pow(2)
            + y.min(side - 1) * side
            + x.min(side - 1);
        self.set(light * self.per_light + offset)
    }

    fn set(&mut self, index: u32) -> bool {
        let (word, bit) = (index as usize / 64, index % 64);
        let Some(slot) = self.marks.get_mut(word) else {
            return false;
        };
        let mask = 1u64 << bit;
        if *slot & mask != 0 {
            return false;
        }
        *slot |= mask;
        self.resident += 1;
        true
    }
}

/// One frame's inputs to the census.
///
/// A struct rather than three arguments because `surfaces` only makes
/// sense alongside the camera that decides which cells exist, and
/// because a walk with it and a walk without it are the two halves of
/// the same comparison.
#[derive(Copy, Clone, Debug)]
pub struct CensusFrame<'a> {
    pub camera: CensusCamera,
    pub lights: &'a [CensusLight],
    /// Where the scene's geometry is.
    ///
    /// 🔴 **Empty means every cell is marked**, which is the walk over
    /// the frustum's whole *volume* — and measuring that against this
    /// one is the point. A froxel is a box of mostly empty air; a page
    /// allocated for air is a page no shadow ever reads. UE5 marks from
    /// the depth buffer and the Chalmers papers from the cluster's view
    /// samples, and both are this filter taken to its limit: not "a cell
    /// with geometry in it" but "a cell with *visible* geometry in it".
    ///
    /// So this is an upper bound on a depth-driven pass and a lower
    /// bound on the volume walk, which is exactly what brackets the
    /// question.
    pub surfaces: &'a [WorldBox],
}

/// Walks the froxel grid and marks every page the frame would need.
///
/// The walk is the one #866 describes — *read off the froxel grid that
/// already runs* — with the grid's cell/light assignment recomputed here
/// rather than read back from the device.
pub fn census(
    config: PageConfig,
    clipmap: ClipmapConfig,
    grid: &ClusterGrid,
    frame: &CensusFrame<'_>,
) -> PageCensus {
    let camera = &frame.camera;
    let mut out = PageCensus::new(config, clipmap, frame.lights.len());
    let world_from_clip = camera.world_from_clip();
    let dims = grid.dimensions;

    for z in 0..dims.z {
        let (near, far) = slice_bounds(grid, z);
        // The slice's near edge, which is where its pixels are smallest
        // and so where its shadow is asked for most.
        let wanted = camera.pixel_at(near);
        for y in 0..dims.y {
            for x in 0..dims.x {
                let cell = cell_aabb(
                    camera,
                    &world_from_clip,
                    dims,
                    UVec3::new(x, y, z),
                    near,
                    far,
                );
                // An empty list is the volume walk: every cell counts.
                if !frame.surfaces.is_empty() && !frame.surfaces.iter().any(|s| cell.overlaps(s)) {
                    continue;
                }
                out.cells += 1;
                for (index, light) in frame.lights.iter().enumerate() {
                    // A sun reaches every cell, which is why it is not
                    // in the froxel grid at all.
                    if !matches!(light.kind, CensusKind::Sun(_))
                        && !cell.reaches(light.position, light.range)
                    {
                        continue;
                    }
                    out.pairs += 1;
                    mark_cell(&mut out, index as u32, light, &cell, wanted, camera.eye());
                }
            }
        }
    }
    out
}

/// A box in world space — a froxel, or a piece of the scene's geometry.
///
/// One type for both because the census asks the same two questions of
/// each: does a light reach it, and does it overlap that other one.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct WorldBox {
    pub min: Vec3,
    pub max: Vec3,
}

impl WorldBox {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self {
            min: min.min(max),
            max: min.max(max),
        }
    }

    /// Whether two boxes share any volume, touching included.
    pub fn overlaps(&self, other: &Self) -> bool {
        self.min.cmple(other.max).all() && other.min.cmple(self.max).all()
    }

    /// Whether a sphere reaches this box.
    fn reaches(&self, position: Vec3, radius: f32) -> bool {
        let nearest = position.clamp(self.min, self.max);
        nearest.distance_squared(position) <= radius * radius
    }

    /// The eight corners, in the order the projection walks them.
    fn corners(&self) -> [Vec3; 8] {
        let (a, b) = (self.min, self.max);
        [
            Vec3::new(a.x, a.y, a.z),
            Vec3::new(b.x, a.y, a.z),
            Vec3::new(a.x, b.y, a.z),
            Vec3::new(b.x, b.y, a.z),
            Vec3::new(a.x, a.y, b.z),
            Vec3::new(b.x, a.y, b.z),
            Vec3::new(a.x, b.y, b.z),
            Vec3::new(b.x, b.y, b.z),
        ]
    }
}

/// The view-space depths a slice spans, both negative.
///
/// 🔴 The first and last slices are not what the mapping says, and the
/// census inherits the correction `ClusterGrid::slice_depth` already
/// documents: slice 0 holds everything nearer than the grid's near, and
/// the last one everything behind its far. Both ends are widened to the
/// range they really hold, which makes those cells *larger* and so the
/// page count they produce *coarser* — conservative in the direction
/// that does not understate the budget.
fn slice_bounds(grid: &ClusterGrid, slice: u32) -> (f32, f32) {
    let edge = |s: f32| ((s + grid.z_factors.y - 1.0) / grid.z_factors.x).exp();
    let near = if slice == 0 { 0.01 } else { edge(slice as f32) };
    let far = if slice + 1 >= grid.dimensions.z {
        grid.far
    } else {
        edge(slice as f32 + 1.0)
    };
    (-near, -far.max(near * 1.0001))
}

/// One cell's world-space bounds.
fn cell_aabb(
    camera: &CensusCamera,
    world_from_clip: &Mat4,
    dims: UVec3,
    cell: UVec3,
    near: f32,
    far: f32,
) -> WorldBox {
    // The NDC rectangle this cell covers. `y` is flipped because the
    // grid indexes rows from the top, the way `cluster_of_ndc` does.
    let xs = [
        cell.x as f32 / dims.x as f32 * 2.0 - 1.0,
        (cell.x + 1) as f32 / dims.x as f32 * 2.0 - 1.0,
    ];
    let ys = [
        1.0 - 2.0 * (cell.y as f32 / dims.y as f32),
        1.0 - 2.0 * ((cell.y + 1) as f32 / dims.y as f32),
    ];
    let zs = [camera.ndc_z(near), camera.ndc_z(far)];

    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for &z in &zs {
        for &y in &ys {
            for &x in &xs {
                let p = *world_from_clip * Vec4::new(x, y, z, 1.0);
                if p.w.abs() < 1e-9 {
                    continue;
                }
                let world = p.truncate() / p.w;
                min = min.min(world);
                max = max.max(world);
            }
        }
    }
    WorldBox { min, max }
}

/// Marks every page one cell needs from one light.
fn mark_cell(
    out: &mut PageCensus,
    light: u32,
    source: &CensusLight,
    cell: &WorldBox,
    wanted: f32,
    eye: Vec3,
) {
    if let CensusKind::Sun(direction) = source.kind {
        mark_sun_cell(out, light, direction, cell, wanted, eye);
        return;
    }

    let nearest = source.position.clamp(cell.min, cell.max);
    let distance = nearest.distance(source.position).max(POINT_SHADOW_NEAR_Z);
    let level = level_for(out.config, distance, wanted);
    let side = out.config.side(level);
    let corners = cell.corners();

    for face in 0..source.faces() {
        let clip_from_world = face_view_proj(source.position, face as usize, POINT_SHADOW_NEAR_Z);
        let mut lo = Vec2::splat(f32::MAX);
        let mut hi = Vec2::splat(f32::MIN);
        let mut behind = false;
        for corner in corners {
            let p = clip_from_world * corner.extend(1.0);
            // `w` is the distance in front of this face's plane: a
            // corner at or behind it has no projection, and clamping one
            // is what silently mirrors a cell onto the wrong face.
            if p.w <= 1e-4 {
                behind = true;
                continue;
            }
            let ndc = p.truncate() / p.w;
            let uv = Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
            lo = lo.min(uv);
            hi = hi.max(uv);
        }
        if lo.x > hi.x {
            continue;
        }
        // ⚠️ A cell the light sits inside straddles the face plane, so
        // its projection is unbounded rather than the rectangle the
        // in-front corners describe. Such a cell really is lit in every
        // direction, so the whole face is the honest answer — and it is
        // self-limiting: a cell that close is a metre from the light, so
        // `level` is already coarse and the face is a handful of pages.
        let (lo, hi) = if behind {
            (Vec2::ZERO, Vec2::ONE)
        } else {
            (
                lo.clamp(Vec2::ZERO, Vec2::ONE),
                hi.clamp(Vec2::ZERO, Vec2::ONE),
            )
        };

        for (x, y) in page_rect(lo, hi, side) {
            out.mark(light, face, level, x, y);
        }
    }
}

/// Marks every page one cell needs from the sun.
///
/// The clipmap is centred on the camera, so a cell's page is its
/// position in the light's plane relative to the viewer — and the level
/// is whichever is both dense enough for the screen and wide enough to
/// contain the cell. When those two disagree, containment wins and the
/// cell gets a coarser shadow than the screen asked for, which is the
/// trade a clipmap exists to make.
fn mark_sun_cell(
    out: &mut PageCensus,
    light: u32,
    direction: Vec3,
    cell: &WorldBox,
    wanted: f32,
    eye: Vec3,
) {
    let light_from_world = Mat4::look_to_rh(eye, direction, sun_up(direction));
    let mut lo = Vec2::splat(f32::MAX);
    let mut hi = Vec2::splat(f32::MIN);
    for corner in cell.corners() {
        let p = (light_from_world * corner.extend(1.0))
            .truncate()
            .truncate();
        lo = lo.min(p);
        hi = hi.max(p);
    }

    let texels = out.config.texels(0) as f32;
    let reach = lo.abs().max(hi.abs()).max_element() * 2.0;
    let clipmap = out.clipmap;
    // Containment is a floor and density is a ceiling: the level must
    // be wide enough to hold the cell, and no wider than the screen's
    // pixels justify. Rounding goes opposite ways for the same reason —
    // `ceil` on containment because a level that *nearly* holds the cell
    // does not hold it, `floor` on density because a level one step
    // coarser than the screen asked for is one step too coarse.
    let contain = level_above(reach / clipmap.base);
    let density = level_below(wanted * texels / clipmap.base);
    let level = contain.max(density).min(clipmap.levels - 1);

    let extent = clipmap.extent(level);
    let half = extent * 0.5;
    let side = out.config.side(0);
    let uv = |p: Vec2| ((p + Vec2::splat(half)) / extent).clamp(Vec2::ZERO, Vec2::ONE);
    for (x, y) in page_rect(uv(lo), uv(hi), side) {
        out.mark_sun(light, level, x, y);
    }
}

/// An up vector the sun's basis will not be degenerate about.
fn sun_up(direction: Vec3) -> Vec3 {
    if direction.y.abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    }
}

/// The smallest level whose doubling reaches `ratio`.
fn level_above(ratio: f32) -> u32 {
    if ratio.is_nan() || ratio <= 1.0 {
        return 0;
    }
    ratio.log2().ceil() as u32
}

/// The largest level whose doubling still fits inside `ratio`.
fn level_below(ratio: f32) -> u32 {
    if ratio.is_nan() || ratio <= 1.0 {
        return 0;
    }
    ratio.log2().floor() as u32
}

/// The pages a normalised rectangle covers.
fn page_rect(lo: Vec2, hi: Vec2, side: u32) -> impl Iterator<Item = (u32, u32)> {
    let last = side.saturating_sub(1);
    let x0 = ((lo.x * side as f32) as u32).min(last);
    let x1 = ((hi.x * side as f32) as u32).min(last);
    let y0 = ((lo.y * side as f32) as u32).min(last);
    let y1 = ((hi.y * side as f32) as u32).min(last);
    (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| (x, y)))
}

/// The coarsest level whose texels are still at least as dense as the
/// screen's pixels.
///
/// A cube face spans 90°, so at `distance` it covers `2 * distance`
/// world units across its texels — the same collapse of
/// `2 * tan(half_fov)` that `face_texel_size` documents. The level is
/// then the largest whose texel is no bigger than `wanted`.
fn level_for(config: PageConfig, distance: f32, wanted: f32) -> u32 {
    if wanted <= 0.0 {
        return 0;
    }
    let texels = 2.0 * distance / wanted;
    if texels <= 0.0 {
        return config.levels() - 1;
    }
    let level = (config.virtual_size as f32 / texels).log2().floor();
    (level.max(0.0) as u32).min(config.levels() - 1)
}

pub mod mark;
pub mod pool;
pub mod raster;

/// Whether the marking pass runs, and how coarsely.
///
/// A `Resources` value with a panel, **not** a field of
/// `.rendersettings`: #477 is explicit that nothing on the shadow side
/// should grow a public setting that becomes a compatibility promise
/// before the pool's shape is decided. This is a diagnostic the editor
/// drives, the way [`ClusterSettings`](kooch_lighting::ClusterSettings)
/// is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PageMarkingSettings {
    /// 🔴 Off by default. Nothing reads what the pass writes yet, and a
    /// measurement that runs whether or not anyone asked is a cost
    /// nobody attributed.
    ///
    /// `KOOCH_PAGE_MARKING=1` turns it on from outside a build — an
    /// environment variable *as well as* a setting for the same reason
    /// `KOOCH_CLUSTERING` is one: the comparison it exists for is made
    /// on a handheld, over SSH, against a build nobody wants to make
    /// twice.
    pub enabled: bool,
    /// Pixels one thread stands for, per axis.
    ///
    /// ⚠️ Not free accuracy in either direction. Coarser is fewer
    /// threads **and** a wider pixel footprint, so the level chosen
    /// comes out coarser and the count lower. 1 is the honest reading
    /// and the expensive one.
    pub rate: u32,
    /// Paint the page each pixel reads over the scene.
    ///
    /// 🔴 Forces `rate` to 1 while it is on: at any coarser rate the
    /// view is a grid of dots over an unpainted frame, which reads as
    /// "the pass is broken" rather than as "you asked for one sample in
    /// sixteen".
    pub paint: bool,
}

impl Default for PageMarkingSettings {
    fn default() -> Self {
        Self {
            enabled: mark::enabled_by_environment(),
            rate: mark::rate_from_environment(),
            paint: false,
        }
    }
}

#[cfg(test)]
mod tests;
