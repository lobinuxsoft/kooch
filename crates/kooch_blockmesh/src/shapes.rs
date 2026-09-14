//! Parameterised starting shapes for the block tool (#1106), built from quads wherever the shape
//! allows: the tool edits faces, and a triangulated stair is forty faces nobody can select.

use std::collections::HashMap;
use std::f32::consts::PI;

use glam::Vec3;

use crate::BlockMesh;

/// Smallest length a shape is built with, so a parameter dragged to zero cannot collapse faces.
const MIN_SIZE: f32 = 0.01;

/// A shape to spawn a block from, with its parameters. Centred on the origin, Y up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// A box, `size` along each axis.
    Cube { size: Vec3 },
    /// Steps rising along +Z; `rise` is the total height and `run` the total depth.
    Stairs {
        steps: u32,
        width: f32,
        rise: f32,
        run: f32,
    },
    /// The stairs' footprint as one slope, a blockout stand-in for them.
    Ramp { width: f32, rise: f32, run: f32 },
    /// A half ring standing on its feet: `radius` is the opening, `thickness` the wall.
    Arch {
        segments: u32,
        radius: f32,
        thickness: f32,
        depth: f32,
    },
    /// A prism with `sides` faces around.
    Cylinder {
        sides: u32,
        radius: f32,
        height: f32,
    },
    /// A cylinder whose top ring collapses to a point.
    Cone {
        sides: u32,
        radius: f32,
        height: f32,
    },
    /// A floor slab split into quads along each side, so it can be shaped without cutting.
    Plane {
        subdivisions: u32,
        size: f32,
        thickness: f32,
    },
}

impl Shape {
    /// Every shape with the parameters it spawns with unless changed.
    pub const DEFAULTS: [Shape; 7] = [
        Shape::Cube { size: Vec3::ONE },
        Shape::Stairs {
            steps: 4,
            width: 1.0,
            rise: 1.0,
            run: 2.0,
        },
        Shape::Ramp {
            width: 1.0,
            rise: 1.0,
            run: 2.0,
        },
        Shape::Arch {
            segments: 8,
            radius: 1.0,
            thickness: 0.25,
            depth: 0.5,
        },
        Shape::Cylinder {
            sides: 16,
            radius: 0.5,
            height: 1.0,
        },
        Shape::Cone {
            sides: 16,
            radius: 0.5,
            height: 1.0,
        },
        Shape::Plane {
            subdivisions: 4,
            size: 4.0,
            thickness: 0.1,
        },
    ];

    /// The name shown in the menu and given to the asset file.
    pub fn label(&self) -> &'static str {
        match self {
            Shape::Cube { .. } => "Cube",
            Shape::Stairs { .. } => "Stairs",
            Shape::Ramp { .. } => "Ramp",
            Shape::Arch { .. } => "Arch",
            Shape::Cylinder { .. } => "Cylinder",
            Shape::Cone { .. } => "Cone",
            Shape::Plane { .. } => "Plane",
        }
    }

    /// The authoring mesh: closed, manifold, every face wound counter-clockwise from outside.
    pub fn build(&self) -> BlockMesh {
        let mut out = Builder::default();
        match *self {
            Shape::Cube { size } => {
                return BlockMesh::cuboid(size.max(Vec3::splat(MIN_SIZE)) / 2.0);
            }
            Shape::Stairs {
                steps,
                width,
                rise,
                run,
            } => stairs(&mut out, steps.max(1), width, rise, run),
            Shape::Ramp { width, rise, run } => ramp(&mut out, width, rise, run),
            Shape::Arch {
                segments,
                radius,
                thickness,
                depth,
            } => arch(&mut out, segments.max(1), radius, thickness, depth),
            Shape::Cylinder {
                sides,
                radius,
                height,
            } => round(&mut out, sides.max(3), radius, height, false),
            Shape::Cone {
                sides,
                radius,
                height,
            } => round(&mut out, sides.max(3), radius, height, true),
            Shape::Plane {
                subdivisions,
                size,
                thickness,
            } => plane(&mut out, subdivisions.max(1), size, thickness),
        }
        out.finish()
    }
}

/// A point from `(x, z, y)`, the order the profiles below are written in.
fn at(x: f32, z: f32, y: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

fn stairs(out: &mut Builder, steps: u32, width: f32, rise: f32, run: f32) {
    let w = width.max(MIN_SIZE) / 2.0;
    let (h, d) = (rise.max(MIN_SIZE), run.max(MIN_SIZE));
    let y = |i: u32| -h / 2.0 + h * i as f32 / steps as f32;
    let z = |i: u32| -d / 2.0 + d * i as f32 / steps as f32;
    let base = y(0);
    for i in 0..steps {
        let (z0, z1, lo, hi) = (z(i), z(i + 1), y(i), y(i + 1));
        out.face(&[at(-w, z0, hi), at(-w, z1, hi), at(w, z1, hi), at(w, z0, hi)]);
        out.face(&[at(-w, z0, lo), at(-w, z0, hi), at(w, z0, hi), at(w, z0, lo)]);
        out.face(&[
            at(-w, z0, base),
            at(w, z0, base),
            at(w, z1, base),
            at(-w, z1, base),
        ]);
        // 🔴 Each side column carries the riser's lower corner on its front edge, or the column
        // before it meets this one in a T-junction and the mesh leaks.
        let mut right = vec![at(w, z0, base)];
        let mut left = vec![
            at(-w, z0, base),
            at(-w, z1, base),
            at(-w, z1, hi),
            at(-w, z0, hi),
        ];
        if i > 0 {
            right.push(at(w, z0, lo));
            left.push(at(-w, z0, lo));
        }
        right.extend([at(w, z0, hi), at(w, z1, hi), at(w, z1, base)]);
        out.face(&right);
        out.face(&left);
    }
    let (back, top) = (z(steps), y(steps));
    out.face(&[
        at(-w, back, base),
        at(w, back, base),
        at(w, back, top),
        at(-w, back, top),
    ]);
}

fn ramp(out: &mut Builder, width: f32, rise: f32, run: f32) {
    let w = width.max(MIN_SIZE) / 2.0;
    let (b, t) = (-rise.max(MIN_SIZE) / 2.0, rise.max(MIN_SIZE) / 2.0);
    let (f, k) = (-run.max(MIN_SIZE) / 2.0, run.max(MIN_SIZE) / 2.0);
    out.face(&[at(-w, f, b), at(-w, k, t), at(w, k, t), at(w, f, b)]);
    out.face(&[at(-w, f, b), at(w, f, b), at(w, k, b), at(-w, k, b)]);
    out.face(&[at(-w, k, b), at(w, k, b), at(w, k, t), at(-w, k, t)]);
    out.face(&[at(w, f, b), at(w, k, t), at(w, k, b)]);
    out.face(&[at(-w, f, b), at(-w, k, b), at(-w, k, t)]);
}

fn arch(out: &mut Builder, segments: u32, radius: f32, thickness: f32, depth: f32) {
    let inner = radius.max(MIN_SIZE);
    let outer = inner + thickness.max(MIN_SIZE);
    let hz = depth.max(MIN_SIZE) / 2.0;
    let lift = -outer / 2.0;
    let ring = |j: u32, r: f32, z: f32| {
        let angle = PI * j as f32 / segments as f32;
        Vec3::new(r * angle.cos(), r * angle.sin() + lift, z)
    };
    for j in 0..segments {
        let n = j + 1;
        out.face(&[
            ring(j, inner, hz),
            ring(j, outer, hz),
            ring(n, outer, hz),
            ring(n, inner, hz),
        ]);
        out.face(&[
            ring(j, inner, -hz),
            ring(n, inner, -hz),
            ring(n, outer, -hz),
            ring(j, outer, -hz),
        ]);
        out.face(&[
            ring(j, outer, hz),
            ring(j, outer, -hz),
            ring(n, outer, -hz),
            ring(n, outer, hz),
        ]);
        out.face(&[
            ring(j, inner, hz),
            ring(n, inner, hz),
            ring(n, inner, -hz),
            ring(j, inner, -hz),
        ]);
    }
    let m = segments;
    out.face(&[
        ring(0, inner, hz),
        ring(0, inner, -hz),
        ring(0, outer, -hz),
        ring(0, outer, hz),
    ]);
    out.face(&[
        ring(m, inner, hz),
        ring(m, outer, hz),
        ring(m, outer, -hz),
        ring(m, inner, -hz),
    ]);
}

fn round(out: &mut Builder, sides: u32, radius: f32, height: f32, cone: bool) {
    let r = radius.max(MIN_SIZE);
    let hh = height.max(MIN_SIZE) / 2.0;
    let ring = |k: u32, y: f32| {
        let angle = 2.0 * PI * (k % sides) as f32 / sides as f32;
        Vec3::new(r * angle.cos(), y, r * angle.sin())
    };
    let bottom: Vec<Vec3> = (0..sides).map(|k| ring(k, -hh)).collect();
    out.face(&bottom);
    let apex = Vec3::new(0.0, hh, 0.0);
    for k in 0..sides {
        match cone {
            true => out.face(&[ring(k, -hh), apex, ring(k + 1, -hh)]),
            false => out.face(&[ring(k, -hh), ring(k, hh), ring(k + 1, hh), ring(k + 1, -hh)]),
        }
    }
    if !cone {
        let top: Vec<Vec3> = (0..sides).rev().map(|k| ring(k, hh)).collect();
        out.face(&top);
    }
}

fn plane(out: &mut Builder, subdivisions: u32, size: f32, thickness: f32) {
    let s = size.max(MIN_SIZE) / 2.0;
    let (b, t) = (
        -thickness.max(MIN_SIZE) / 2.0,
        thickness.max(MIN_SIZE) / 2.0,
    );
    let g = |i: u32| -s + 2.0 * s * i as f32 / subdivisions as f32;
    for i in 0..subdivisions {
        for j in 0..subdivisions {
            let (x0, x1, z0, z1) = (g(i), g(i + 1), g(j), g(j + 1));
            out.face(&[at(x0, z0, t), at(x0, z1, t), at(x1, z1, t), at(x1, z0, t)]);
            out.face(&[at(x0, z0, b), at(x1, z0, b), at(x1, z1, b), at(x0, z1, b)]);
        }
    }
    for k in 0..subdivisions {
        let (a, c) = (g(k), g(k + 1));
        out.face(&[at(-s, a, b), at(-s, c, b), at(-s, c, t), at(-s, a, t)]);
        out.face(&[at(s, a, b), at(s, a, t), at(s, c, t), at(s, c, b)]);
        out.face(&[at(a, -s, b), at(a, -s, t), at(c, -s, t), at(c, -s, b)]);
        out.face(&[at(a, s, b), at(c, s, b), at(c, s, t), at(a, s, t)]);
    }
}

/// Collects faces by position, welding corners that land on the same point so neighbouring faces
/// share them — an unwelded seam is an open edge.
#[derive(Default)]
struct Builder {
    positions: Vec<Vec3>,
    faces: Vec<Vec<u32>>,
    known: HashMap<[i64; 3], u32>,
}

impl Builder {
    fn corner(&mut self, point: Vec3) -> u32 {
        // Tenth of a millimetre: coarse enough that `sin(π)` lands on zero.
        let key = (point * 1.0e4).round().as_i64vec3().to_array();
        let positions = &mut self.positions;
        *self.known.entry(key).or_insert_with(|| {
            positions.push(point);
            positions.len() as u32 - 1
        })
    }

    fn face(&mut self, points: &[Vec3]) {
        let face = points.iter().map(|point| self.corner(*point)).collect();
        self.faces.push(face);
    }

    fn finish(self) -> BlockMesh {
        BlockMesh::from_faces(self.positions, &self.faces).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
