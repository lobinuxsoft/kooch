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
        /// Degrees the flight turns: 0 is straight, a full turn or more is a spiral.
        turn: f32,
        /// Inner radius of a turning flight.
        core: f32,
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
    /// A rectangular door frame: the opening is `width` by `height`, bordered by `frame` on the
    /// sides and top, so resizing the opening leaves the border as it is.
    Door {
        width: f32,
        height: f32,
        frame: f32,
        depth: f32,
    },
}

impl Shape {
    /// Every shape with the parameters it spawns with unless changed.
    pub const DEFAULTS: [Shape; 8] = [
        Shape::Cube { size: Vec3::ONE },
        Shape::Stairs {
            steps: 4,
            width: 1.0,
            rise: 1.0,
            run: 2.0,
            turn: 0.0,
            core: 0.5,
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
        Shape::Door {
            width: 1.0,
            height: 2.1,
            frame: 0.2,
            depth: 0.5,
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
            Shape::Door { .. } => "Door",
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
                turn,
                core,
            } => match turn.abs() < 1.0e-3 {
                true => stairs(&mut out, steps.max(1), width, rise, run),
                false => turning_stairs(&mut out, steps.max(1), width, rise, turn, core),
            },
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
            Shape::Door {
                width,
                height,
                frame,
                depth,
            } => door(&mut out, width, height, frame, depth),
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

/// Steps around a vertical axis. Below a full turn each step is a column down to the floor; from a
/// full turn up the columns would pass through the steps beneath, so each step floats as a wedge.
fn turning_stairs(out: &mut Builder, steps: u32, width: f32, rise: f32, turn: f32, core: f32) {
    let sweep = turn.to_radians();
    // Under 45 degrees a step, so every column stays convex and its chords stay near the arc.
    let steps = steps.max((sweep.abs() / (PI / 4.0)).ceil() as u32);
    let inner = core.max(MIN_SIZE);
    let outer = inner + width.max(MIN_SIZE);
    let lift = rise.max(MIN_SIZE) / steps as f32;
    let floating = sweep.abs() >= 2.0 * PI - 1.0e-3;
    let at = |angle: f32, r: f32, y: f32| Vec3::new(r * angle.cos(), y, r * angle.sin());
    for i in 0..steps {
        let a0 = sweep * i as f32 / steps as f32;
        let a1 = sweep * (i + 1) as f32 / steps as f32;
        let (lo, hi) = (lift * i as f32, lift * (i + 1) as f32);
        let base = if floating { lo } else { 0.0 };
        let inside = at((a0 + a1) / 2.0, (inner + outer) / 2.0, (base + hi) / 2.0);
        // Floating wedges are closed on their own and welded to nothing.
        let mut wedge = Builder::default();
        let target = if floating { &mut wedge } else { &mut *out };
        target.face_away(
            &[
                at(a0, inner, hi),
                at(a0, outer, hi),
                at(a1, outer, hi),
                at(a1, inner, hi),
            ],
            inside,
        );
        target.face_away(
            &[
                at(a0, inner, lo),
                at(a0, outer, lo),
                at(a0, outer, hi),
                at(a0, inner, hi),
            ],
            inside,
        );
        target.face_away(
            &[
                at(a0, inner, base),
                at(a1, inner, base),
                at(a1, outer, base),
                at(a0, outer, base),
            ],
            inside,
        );
        for r in [inner, outer] {
            // 🔴 The previous column's top corner on this one's front edge, or they meet in a T.
            let mut wall = vec![
                at(a0, r, base),
                at(a1, r, base),
                at(a1, r, hi),
                at(a0, r, hi),
            ];
            if !floating && i > 0 {
                wall.push(at(a0, r, lo));
            }
            target.face_away(&wall, inside);
        }
        if floating || i + 1 == steps {
            target.face_away(
                &[
                    at(a1, inner, base),
                    at(a1, outer, base),
                    at(a1, outer, hi),
                    at(a1, inner, hi),
                ],
                inside,
            );
        }
        if floating {
            out.append(wedge);
        }
    }
}

fn door(out: &mut Builder, width: f32, height: f32, frame: f32, depth: f32) {
    let w = width.max(MIN_SIZE) / 2.0;
    let h = height.max(MIN_SIZE);
    let t = frame.max(MIN_SIZE);
    let d = depth.max(MIN_SIZE) / 2.0;
    let (x, top) = (w + t, h + t);
    let p = Vec3::new;
    let left = p(-(w + t / 2.0), top / 2.0, 0.0);
    let right = p(w + t / 2.0, top / 2.0, 0.0);
    let lintel = p(0.0, h + t / 2.0, 0.0);
    for z in [d, -d] {
        out.face_away(
            &[
                p(-x, 0.0, z),
                p(-w, 0.0, z),
                p(-w, h, z),
                p(-w, top, z),
                p(-x, top, z),
            ],
            left,
        );
        out.face_away(
            &[
                p(w, 0.0, z),
                p(x, 0.0, z),
                p(x, top, z),
                p(w, top, z),
                p(w, h, z),
            ],
            right,
        );
        out.face_away(
            &[p(-w, h, z), p(w, h, z), p(w, top, z), p(-w, top, z)],
            lintel,
        );
    }
    out.face_away(
        &[p(-x, 0.0, d), p(-x, 0.0, -d), p(-x, top, -d), p(-x, top, d)],
        left,
    );
    out.face_away(
        &[p(x, 0.0, d), p(x, 0.0, -d), p(x, top, -d), p(x, top, d)],
        right,
    );
    out.face_away(
        &[
            p(-x, top, d),
            p(-w, top, d),
            p(w, top, d),
            p(x, top, d),
            p(x, top, -d),
            p(w, top, -d),
            p(-w, top, -d),
            p(-x, top, -d),
        ],
        lintel,
    );
    out.face_away(
        &[p(-x, 0.0, d), p(-w, 0.0, d), p(-w, 0.0, -d), p(-x, 0.0, -d)],
        left,
    );
    out.face_away(
        &[p(w, 0.0, d), p(x, 0.0, d), p(x, 0.0, -d), p(w, 0.0, -d)],
        right,
    );
    out.face_away(
        &[p(-w, 0.0, d), p(-w, h, d), p(-w, h, -d), p(-w, 0.0, -d)],
        left,
    );
    out.face_away(
        &[p(w, 0.0, d), p(w, h, d), p(w, h, -d), p(w, 0.0, -d)],
        right,
    );
    out.face_away(
        &[p(-w, h, d), p(w, h, d), p(w, h, -d), p(-w, h, -d)],
        lintel,
    );
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

    /// Adds a face wound so its normal points away from `inside`, a point within the convex piece
    /// the face bounds — for shapes whose winding is easier to derive than to write out.
    fn face_away(&mut self, points: &[Vec3], inside: Vec3) {
        let mut normal = Vec3::ZERO;
        for (index, current) in points.iter().enumerate() {
            let next = points[(index + 1) % points.len()];
            normal += (*current - next).cross(*current + next);
        }
        let centre = points.iter().copied().sum::<Vec3>() / points.len() as f32;
        match normal.dot(centre - inside) < 0.0 {
            true => {
                let reversed: Vec<Vec3> = points.iter().rev().copied().collect();
                self.face(&reversed);
            }
            false => self.face(points),
        }
    }

    /// Adds another builder's faces without welding them to these.
    fn append(&mut self, other: Builder) {
        let offset = self.positions.len() as u32;
        self.positions.extend(other.positions);
        self.faces.extend(
            other
                .faces
                .into_iter()
                .map(|face| face.into_iter().map(|corner| corner + offset).collect()),
        );
    }

    fn finish(self) -> BlockMesh {
        BlockMesh::from_faces(self.positions, &self.faces).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
