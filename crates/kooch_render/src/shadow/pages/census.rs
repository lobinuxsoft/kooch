//! The CPU census of the pages a frame needs: which lights, faces, levels and cells, and what they cost.

use super::*;

/// What kind of chain a light addresses.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum CensusKind {
    Point,
    Spot,
    /// The direction the light travels, normalised.
    Sun(Vec3),
}

/// A shadow-casting light, as the census needs it.
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
    pub(super) fn faces(&self) -> u32 {
        match self.kind {
            CensusKind::Point => CUBE_FACES as u32,
            CensusKind::Spot | CensusKind::Sun(_) => 1,
        }
    }
}

/// How a directional light's clipmap is nested.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ClipmapConfig {
    /// Level 0's extent, in metres.
    pub base: f32,
    pub levels: u32,
}

impl Default for ClipmapConfig {
    /// Unreal's, read off the UE 5.8 documentation: *"by default, clipmap levels 6 through 22 are
    /// allocated"*, the finest *"covering 64 cm (2^6 cm) from the camera position"* and the
    /// broadest *"about 40 kilometers (2^22 cm)"*.
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
    pub(super) fn world_from_clip(&self) -> Mat4 {
        self.world_from_view * self.clip_from_view.inverse()
    }

    /// World metres one screen pixel covers at `depth`.
    pub(super) fn pixel_at(&self, depth: f32) -> f32 {
        let focal = self.clip_from_view.y_axis.y;
        if focal.abs() < f32::EPSILON {
            return 0.0;
        }
        2.0 * depth.abs() / (focal * self.viewport.y.max(1.0))
    }

    /// Where the camera is, which is what a clipmap is centred on.
    pub(super) fn eye(&self) -> Vec3 {
        self.world_from_view.w_axis.truncate()
    }

    /// Where a view-space depth lands in NDC.
    pub(super) fn ndc_z(&self, view_z: f32) -> f32 {
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
        // One stride for every light, sized for whichever chain is longest. A per-kind stride would
        // save bits and cost a prefix sum to find a light's base — and this buffer is under two MiB
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
    pub(super) fn mark(&mut self, light: u32, face: u32, level: u32, x: u32, y: u32) -> bool {
        let side = self.config.side(level);
        let offset = face * self.config.face_pages()
            + self.config.level_base(level)
            + y.min(side - 1) * side
            + x.min(side - 1);
        self.set(light * self.per_light + offset)
    }

    /// Marks one page of the sun's clipmap.
    pub(super) fn mark_sun(&mut self, light: u32, level: u32, x: u32, y: u32) -> bool {
        let side = self.config.side(0);
        let offset = level.min(self.clipmap.levels - 1) * side.pow(2)
            + y.min(side - 1) * side
            + x.min(side - 1);
        self.set(light * self.per_light + offset)
    }

    pub(super) fn set(&mut self, index: u32) -> bool {
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
#[derive(Copy, Clone, Debug)]
pub struct CensusFrame<'a> {
    pub camera: CensusCamera,
    pub lights: &'a [CensusLight],
    /// Where the scene's geometry is.
    pub surfaces: &'a [WorldBox],
}

/// Walks the froxel grid and marks every page the frame would need.
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
    pub(super) fn reaches(&self, position: Vec3, radius: f32) -> bool {
        let nearest = position.clamp(self.min, self.max);
        nearest.distance_squared(position) <= radius * radius
    }

    /// The eight corners, in the order the projection walks them.
    pub(super) fn corners(&self) -> [Vec3; 8] {
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
pub(super) fn slice_bounds(grid: &ClusterGrid, slice: u32) -> (f32, f32) {
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
pub(super) fn cell_aabb(
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
pub(super) fn mark_cell(
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
        // ⚠️ A cell the light sits inside straddles the face plane, so its projection is unbounded
        // rather than the rectangle the in-front corners describe.
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
pub(super) fn mark_sun_cell(
    out: &mut PageCensus,
    light: u32,
    direction: Vec3,
    cell: &WorldBox,
    wanted: f32,
    eye: Vec3,
) {
    let light_from_world = glam::camera::rh::view::look_to_mat4(eye, direction, sun_up(direction));
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
    // Containment is a floor and density is a ceiling: the level must be wide enough to hold the
    // cell, and no wider than the screen's pixels justify.
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
pub(super) fn sun_up(direction: Vec3) -> Vec3 {
    if direction.y.abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    }
}

/// The smallest level whose doubling reaches `ratio`.
pub(super) fn level_above(ratio: f32) -> u32 {
    if ratio.is_nan() || ratio <= 1.0 {
        return 0;
    }
    ratio.log2().ceil() as u32
}

/// The largest level whose doubling still fits inside `ratio`.
pub fn level_below(ratio: f32) -> u32 {
    if ratio.is_nan() || ratio <= 1.0 {
        return 0;
    }
    ratio.log2().floor() as u32
}

/// The pages a normalised rectangle covers.
pub(super) fn page_rect(lo: Vec2, hi: Vec2, side: u32) -> impl Iterator<Item = (u32, u32)> {
    let last = side.saturating_sub(1);
    let x0 = ((lo.x * side as f32) as u32).min(last);
    let x1 = ((hi.x * side as f32) as u32).min(last);
    let y0 = ((lo.y * side as f32) as u32).min(last);
    let y1 = ((hi.y * side as f32) as u32).min(last);
    (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| (x, y)))
}

/// The coarsest level whose texels are still at least as dense as the screen's pixels.
pub(super) fn level_for(config: PageConfig, distance: f32, wanted: f32) -> u32 {
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
