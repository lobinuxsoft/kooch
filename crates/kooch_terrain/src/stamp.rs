//! The stamp stack (#1954): shapes a terrain lists, applied in order over its generator.
//!
//! Instances are data, never code: the generate pass walks one buffer of them after `fn planet`,
//! and the CPU twin applies the same stack here, so moving a stamp recompiles nothing and only the
//! blocks inside its old and new bounds generate again. Godot Voxel's `VoxelModifier` is the model.

mod buffer;
#[cfg(test)]
mod tests;

use bytemuck::{Pod, Zeroable};
use glam::{DMat3, DVec3, Vec3};
use kooch_ecs::Reflect;
use kooch_ecs::reflect::{FieldChoice, FieldCondition, FieldRange};

use crate::lod::{Base, Shape};

pub use buffer::StampBuffer;

pub const SHAPE_SPHERE: u32 = 0;
pub const SHAPE_BOX: u32 = 1;

pub const OPERATION_ADD: u32 = 0;
pub const OPERATION_SUBTRACT: u32 = 1;
pub const OPERATION_INTERSECT: u32 = 2;

/// One entry of [`Terrain::stamps`](crate::Terrain::stamps), placed on the ground's tangent frame:
/// its up is away from a sphere's centre, or +Y on a plane.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct Stamp {
    /// Off leaves the ground as the stamps before it made it.
    pub enabled: bool,
    #[reflect(choices = SHAPE_CHOICES)]
    pub shape: u32,
    /// Add raises rock, subtract digs, intersect keeps only what is inside the shape.
    #[reflect(choices = OPERATION_CHOICES)]
    pub operation: u32,
    /// The shape's centre in the terrain's space, in metres.
    pub position: Vec3,
    /// Degrees the shape turns about the ground's up at `position`.
    #[reflect(range = TURN_RANGE)]
    pub turn: f32,
    #[reflect(range = LENGTH_RANGE, shown_when = IS_SPHERE)]
    pub radius: f32,
    /// Half the box's side on each axis: x and z along the ground, y up.
    #[reflect(shown_when = IS_BOX)]
    pub size: Vec3,
    /// Metres over which the shape melts into the ground; 0 is a sharp edge.
    #[reflect(range = LENGTH_RANGE)]
    pub smooth: f32,
    /// How much of the stamp shows: 0 is the ground without it, 1 all of it.
    #[reflect(range = UNIT_RANGE)]
    pub opacity: f32,
    /// The material layer (#1951) its surface is painted with.
    #[reflect(range = LAYER_RANGE)]
    pub layer: u32,
    /// How strongly `layer` covers its surface: 0 leaves the generator's layers.
    #[reflect(range = UNIT_RANGE)]
    pub paint: f32,
}

impl Default for Stamp {
    fn default() -> Self {
        Self {
            enabled: true,
            shape: SHAPE_SPHERE,
            operation: OPERATION_ADD,
            position: Vec3::ZERO,
            turn: 0.0,
            radius: 5.0,
            size: Vec3::splat(5.0),
            smooth: 1.0,
            opacity: 1.0,
            layer: 1,
            paint: 0.0,
        }
    }
}

pub static SHAPE_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Sphere",
        value: SHAPE_SPHERE as i64,
    },
    FieldChoice {
        label: "Box",
        value: SHAPE_BOX as i64,
    },
];

pub static OPERATION_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Add",
        value: OPERATION_ADD as i64,
    },
    FieldChoice {
        label: "Subtract",
        value: OPERATION_SUBTRACT as i64,
    },
    FieldChoice {
        label: "Intersect",
        value: OPERATION_INTERSECT as i64,
    },
];

static IS_SPHERE: FieldCondition = FieldCondition {
    field: "shape",
    values: &[SHAPE_SPHERE as i64],
};

static IS_BOX: FieldCondition = FieldCondition {
    field: "shape",
    values: &[SHAPE_BOX as i64],
};

const TURN_RANGE: FieldRange = FieldRange {
    min: -180.0,
    max: 180.0,
    step: 1.0,
};

const LENGTH_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 1.0e6,
    step: 0.1,
};

const UNIT_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 1.0,
    step: 0.01,
};

const LAYER_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: (crate::LAYERS - 1) as f64,
    step: 1.0,
};

/// A stamp in the generate pass. Mirrors `Stamp` in `generate.wgsl`; 96 bytes.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub struct GpuStamp {
    /// The centre in finest voxels, rounded down: a sample's offset from it is an exact integer
    /// before any float, so a stamp is as sharp at Earth radius as at the origin.
    pub centre: [i32; 3],
    pub shape: u32,
    /// What `centre` dropped, in metres.
    pub rest: [f32; 3],
    pub operation: u32,
    /// Rows of the turn from the terrain's space into the stamp's.
    pub x: [f32; 3],
    pub smooth: f32,
    pub y: [f32; 3],
    pub opacity: f32,
    pub z: [f32; 3],
    pub layer: u32,
    /// Half extents; a sphere's radius in `x`.
    pub size: [f32; 3],
    pub paint: f32,
}

/// One enabled stamp, measured for the CPU.
#[derive(Clone, Copy, Debug)]
struct Placed {
    centre: DVec3,
    /// Rows: the terrain's space into the stamp's.
    turn: DMat3,
    shape: u32,
    operation: u32,
    size: DVec3,
    smooth: f64,
    opacity: f64,
}

/// A terrain's stamps as both sides evaluate them.
#[derive(Clone, Debug, Default)]
pub struct Stack {
    /// What it was built from, to tell what an edit changed.
    pub(crate) stamps: Vec<Stamp>,
    placed: Vec<Placed>,
    pub(crate) gpu: std::sync::Arc<[GpuStamp]>,
    /// Bumped on every change, so a collider built before it is rebuilt.
    pub(crate) revision: u32,
}

impl Stack {
    /// `stamps` on a ground of `base` whose finest voxel is `fine` metres.
    pub(crate) fn new(stamps: &[Stamp], base: Base, fine: f64, revision: u32) -> Self {
        let placed: Vec<Placed> = stamps
            .iter()
            .filter(|stamp| stamp.enabled && stamp.opacity > 0.0)
            .map(|stamp| place(stamp, base))
            .collect();
        let gpu = stamps
            .iter()
            .filter(|stamp| stamp.enabled && stamp.opacity > 0.0)
            .zip(&placed)
            .map(|(stamp, placed)| record(stamp, placed, fine))
            .collect();
        Self {
            stamps: stamps.to_vec(),
            placed,
            gpu,
            revision,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.placed.is_empty()
    }

    /// `d`, the generator's SDF at `point`, with every stamp applied.
    pub(crate) fn apply(&self, point: DVec3, d: f64) -> f64 {
        self.placed.iter().fold(d, |d, stamp| {
            let s = distance(stamp, stamp.turn * (point - stamp.centre));
            let k = stamp.smooth;
            let applied = match stamp.operation {
                OPERATION_SUBTRACT => smooth_max(d, -s, k),
                OPERATION_INTERSECT => smooth_max(d, s, k),
                _ => smooth_min(d, s, k),
            };
            d + (applied - d) * stamp.opacity
        })
    }

    /// `(lo, hi)`, the generator's range over the box, with every stamp applied: each shape is
    /// bounded from the box's centre, an SDF changing by at most its distance.
    pub(crate) fn bound(&self, min: DVec3, max: DVec3, range: (f64, f64)) -> (f64, f64) {
        let centre = (min + max) * 0.5;
        let reach = (max - min).length() * 0.5;
        self.placed.iter().fold(range, |(lo, hi), stamp| {
            let s = distance(stamp, stamp.turn * (centre - stamp.centre));
            let (slo, shi) = (s - reach, s + reach);
            // A smooth blend moves the hard result by at most a quarter of its radius.
            let k = stamp.smooth * 0.25;
            let (alo, ahi) = match stamp.operation {
                OPERATION_SUBTRACT => (lo.max(-shi), hi.max(-slo) + k),
                OPERATION_INTERSECT => (lo.max(slo), hi.max(shi) + k),
                _ => (lo.min(slo) - k, hi.min(shi)),
            };
            let t = stamp.opacity;
            (lo + (alo - lo) * t, hi + (ahi - hi) * t)
        })
    }
}

/// Where an edit from `old` to `new` can change the ground: the boxes of every stamp not the same
/// at the same place in both. `None` when an intersect changed, which reaches the whole terrain.
pub(crate) fn changed(old: &[Stamp], new: &[Stamp], base: Base) -> Option<Vec<(DVec3, DVec3)>> {
    let mut boxes = Vec::new();
    for i in 0..old.len().max(new.len()) {
        let (a, b) = (old.get(i), new.get(i));
        if a == b {
            continue;
        }
        for stamp in [a, b].into_iter().flatten() {
            if !stamp.enabled {
                continue;
            }
            if stamp.operation == OPERATION_INTERSECT {
                return None;
            }
            boxes.push(reach(stamp, base));
        }
    }
    Some(boxes)
}

/// Whether box `min..max` meets box `lo..hi`.
pub(crate) fn overlaps(min: DVec3, max: DVec3, lo: DVec3, hi: DVec3) -> bool {
    min.cmple(hi).all() && max.cmpge(lo).all()
}

/// The box a stamp's shape and its blend stay inside, in the terrain's space.
fn reach(stamp: &Stamp, base: Base) -> (DVec3, DVec3) {
    let placed = place(stamp, base);
    let r = match placed.shape {
        SHAPE_BOX => placed.size.length(),
        _ => placed.size.x,
    } + placed.smooth;
    (placed.centre - r, placed.centre + r)
}

impl Shape {
    /// Grown so every stamp fits: the octree has no blocks past relief and depth, and a plane
    /// none past its extent.
    pub(crate) fn hold(mut self, stamps: &[Stamp]) -> Self {
        for stamp in stamps.iter().filter(|stamp| stamp.enabled) {
            let (min, max) = reach(stamp, self.base);
            match self.base {
                Base::Sphere => {
                    let centre = (min + max) * 0.5;
                    let r = (max.x - min.x) * 0.5;
                    let at = centre.length();
                    self.relief = self.relief.max(at + r - self.radius);
                    self.depth = self.depth.max(self.radius - (at - r));
                }
                Base::Plane => {
                    self.relief = self.relief.max(max.y);
                    self.depth = self.depth.max(-min.y);
                    let side = max.x.max(max.z).max(-min.x).max(-min.z);
                    self.extent = self.extent.max(side);
                }
            }
        }
        self
    }
}

fn place(stamp: &Stamp, base: Base) -> Placed {
    let centre = stamp.position.as_dvec3();
    let up = match base {
        Base::Plane => DVec3::Y,
        Base::Sphere => centre.try_normalize().unwrap_or(DVec3::Y),
    };
    let reference = match up.y.abs() > 0.999 {
        true => DVec3::Z,
        false => DVec3::Y,
    };
    let east = reference.cross(up).normalize();
    let north = east.cross(up);
    let (sin, cos) = f64::from(stamp.turn).to_radians().sin_cos();
    let x = east * cos - north * sin;
    let z = east * sin + north * cos;
    let size = match stamp.shape {
        SHAPE_BOX => stamp.size.as_dvec3().max(DVec3::ZERO),
        _ => DVec3::splat(f64::from(stamp.radius.max(0.0))),
    };
    Placed {
        centre,
        turn: DMat3::from_cols(x, up, z).transpose(),
        shape: stamp.shape,
        operation: stamp.operation,
        size,
        smooth: f64::from(stamp.smooth.max(0.0)),
        opacity: f64::from(stamp.opacity.clamp(0.0, 1.0)),
    }
}

fn record(stamp: &Stamp, placed: &Placed, fine: f64) -> GpuStamp {
    let cell = (placed.centre / fine).floor();
    let rest = placed.centre - cell * fine;
    let row = |r: DVec3| r.as_vec3().to_array();
    GpuStamp {
        centre: cell.as_ivec3().to_array(),
        shape: placed.shape,
        rest: rest.as_vec3().to_array(),
        operation: placed.operation,
        x: row(placed.turn.row(0)),
        smooth: placed.smooth as f32,
        y: row(placed.turn.row(1)),
        opacity: placed.opacity as f32,
        z: row(placed.turn.row(2)),
        layer: stamp.layer.min(crate::LAYERS as u32 - 1),
        size: placed.size.as_vec3().to_array(),
        paint: stamp.paint.clamp(0.0, 1.0),
    }
}

/// The shape's SDF at `p`, in its own space.
fn distance(stamp: &Placed, p: DVec3) -> f64 {
    match stamp.shape {
        SHAPE_BOX => {
            let q = p.abs() - stamp.size;
            q.max(DVec3::ZERO).length() + q.max_element().min(0.0)
        }
        _ => p.length() - stamp.size.x,
    }
}

/// Íñigo Quílez's polynomial smooth minimum; `k` 0 is the hard one.
fn smooth_min(a: f64, b: f64, k: f64) -> f64 {
    if !(k > 0.0) {
        return a.min(b);
    }
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k * 0.25
}

fn smooth_max(a: f64, b: f64, k: f64) -> f64 {
    -smooth_min(-a, -b, k)
}
