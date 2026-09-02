//! Reading every source out of the world, and answering with their sum.

use glam::{Mat4, Quat, Vec3};

use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::transform::Transform;

use crate::sources::{
    AreaGravity, BoxGravity, GlobalGravity, GravityPriority, PlaneGravity, PointGravity,
};

/// One source, resolved into world space.
pub(crate) struct Source {
    pub position: Vec3,
    /// From [`GravityPriority`], or 0 for the sources that carry none.
    pub level: i32,
    pub kind: Kind,
}

pub(crate) enum Kind {
    Global(GlobalGravity),
    Point(PointGravity),
    /// Carried with both halves of its transform: the inverse puts a
    /// world point into the field's space, and the rotation brings the
    /// resulting direction back out.
    Area {
        settings: AreaGravity,
        local: LocalSpace,
    },
    /// A solid whose direction differs at every point around it, so its
    /// answer needs the same round trip an area's does.
    Solid {
        settings: BoxGravity,
        local: LocalSpace,
    },
    /// A floor: bounded along its normal, unbounded across it.
    Plane {
        settings: PlaneGravity,
        local: LocalSpace,
    },
}

/// The two halves of a transform a local-space field needs: the inverse to
/// put a world point into the field's space, and the rotation to bring the
/// resulting direction back out.
///
/// Both, because only converting the point is a bug that looks like the
/// field working — the shape is tested in the right place and then pulls
/// along an *unrotated* direction, so a rotated zone drops things straight
/// down.
pub(crate) struct LocalSpace {
    pub to_local: Mat4,
    rotation: Quat,
}

impl LocalSpace {
    fn direction_from(&self, point: Vec3, local: impl Fn(Vec3) -> Vec3) -> Vec3 {
        self.rotation * local(self.to_local.transform_point3(point))
    }

    fn local_point(&self, point: Vec3) -> Vec3 {
        self.to_local.transform_point3(point)
    }
}

impl Source {
    pub(crate) fn acceleration_at(&self, point: Vec3) -> Vec3 {
        match &self.kind {
            Kind::Global(global) => global.acceleration,
            Kind::Point(source) => source.acceleration_at(self.position, point),
            Kind::Area { settings, local } => {
                local.direction_from(point, |p| settings.acceleration_at_local(p))
            }
            Kind::Solid { settings, local } => {
                local.direction_from(point, |p| settings.acceleration_at_local(p))
            }
            Kind::Plane { settings, local } => {
                local.direction_from(point, |p| settings.acceleration_at_local(p))
            }
        }
    }

    /// How strongly this source claims a point, in 0..=1.
    ///
    /// Not the magnitude of its pull: a weak field that fully covers a room
    /// still owns that room, and a strong one at the edge of its fade does
    /// not. This is the shape's own reach, which is why it is what
    /// [`GravityPriority`] suppresses with.
    fn claim_at(&self, point: Vec3) -> f32 {
        match &self.kind {
            // No bounds to be outside of.
            Kind::Global(_) => 1.0,
            // A hard edge, because a point source has no fade band — noted
            // on `GravityPriority` so an author picks a shape that does.
            Kind::Point(source) => {
                let outside = source.range > 0.0 && self.position.distance(point) > source.range;
                f32::from(!outside)
            }
            Kind::Area { settings, local } => settings.influence_at_local(local.local_point(point)),
            // Inside the solid there is no surface to fall towards, and the
            // body is as claimed by this planet as it can be.
            Kind::Solid { settings, local } => {
                match settings.pull_at_local(local.local_point(point)) {
                    Some((_, distance)) => settings.influence(distance),
                    None => 1.0,
                }
            }
            Kind::Plane { settings, local } => {
                settings.influence_at_local(local.local_point(point))
            }
        }
    }
}

/// Every source in the world, grouped by the level it overrules from.
pub(crate) struct Field {
    pub sources: Vec<Source>,
    /// The distinct levels present, descending. One entry (or none) is the
    /// scene that never asked for a priority, and it takes a plain sum.
    levels: Vec<i32>,
}

impl Field {
    pub(crate) fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// The acceleration every source together applies at a world point.
    pub(crate) fn acceleration_at(&self, point: Vec3) -> Vec3 {
        self.evaluate(point).0
    }

    /// The single strongest contribution at a world point, after
    /// suppression.
    pub(crate) fn dominant_at(&self, point: Vec3) -> Vec3 {
        self.evaluate(point).1
    }

    /// The summed field and its strongest single contributor.
    ///
    /// Both at once because they walk the same sources through the same
    /// suppression, and computing them apart is how the two come to
    /// disagree about which source won.
    fn evaluate(&self, point: Vec3) -> (Vec3, Vec3) {
        let mut total = Vec3::ZERO;
        let mut dominant = Vec3::ZERO;
        // One level is every scene that never authored a priority, and
        // there is nothing for a claim to suppress. Worth the branch: the
        // claim is a second pass over the shape, per source, per body.
        let overruled = self.levels.len() > 1;
        // The share of a level's pull that survives the levels above it.
        // One while nothing has claimed the point yet.
        let mut surviving = 1.0f32;

        for &level in &self.levels {
            let mut claimed = 0.0f32;
            for source in self.sources.iter().filter(|source| source.level == level) {
                let pull = source.acceleration_at(point) * surviving;
                total += pull;
                if pull.length_squared() > dominant.length_squared() {
                    dominant = pull;
                }
                // The strongest single claim, not their sum: two rooms that
                // each half-cover a point do not together overrule the
                // planet under it.
                if overruled {
                    claimed = claimed.max(source.claim_at(point));
                }
            }
            surviving *= 1.0 - claimed.clamp(0.0, 1.0);
            if surviving <= 0.0 {
                break;
            }
        }
        (total, dominant)
    }
}

/// Reads every source in the world, in world space.
pub(crate) fn collect_sources(resources: &Resources) -> Field {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Field {
            sources: Vec::new(),
            levels: Vec::new(),
        };
    };
    let globals = registry.get_cpu::<GlobalTransform>();
    let transforms = registry.get_cpu::<Transform>();
    let priorities = registry.get_cpu::<GravityPriority>();

    // A source without a `GlobalTransform` falls back to its `Transform`,
    // and then to the origin: a global field does not care where it is, and
    // refusing to work until the hierarchy has propagated would make a
    // freshly spawned planet silently inert for a frame.
    let position_of = |entity: Entity| -> Vec3 {
        globals
            .and_then(|storage| storage.get(entity))
            .map(|global| global.matrix.to_scale_rotation_translation().2)
            .or_else(|| transforms.and_then(|s| s.get(entity)).map(|t| t.position))
            .unwrap_or(Vec3::ZERO)
    };
    let matrix_of = |entity: Entity| -> Mat4 {
        globals
            .and_then(|storage| storage.get(entity))
            .map(|global| global.matrix)
            .or_else(|| {
                transforms
                    .and_then(|s| s.get(entity))
                    .map(|t| Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position))
            })
            .unwrap_or(Mat4::IDENTITY)
    };
    let level_of = |entity: Entity| -> i32 {
        priorities
            .and_then(|storage| storage.get(entity))
            .map(|priority| priority.level)
            .unwrap_or(0)
    };

    let mut sources = Vec::new();
    if let Some(storage) = registry.get_cpu::<GlobalGravity>() {
        for (&entity, global) in storage.iter() {
            sources.push(Source {
                position: position_of(entity),
                level: level_of(entity),
                kind: Kind::Global(*global),
            });
        }
    }
    if let Some(storage) = registry.get_cpu::<PointGravity>() {
        for (&entity, point) in storage.iter() {
            sources.push(Source {
                position: position_of(entity),
                level: level_of(entity),
                kind: Kind::Point(*point),
            });
        }
    }
    if let Some(storage) = registry.get_cpu::<AreaGravity>() {
        for (&entity, area) in storage.iter() {
            sources.push(Source {
                position: position_of(entity),
                level: level_of(entity),
                kind: Kind::Area {
                    settings: *area,
                    local: local_space(matrix_of(entity)),
                },
            });
        }
    }
    if let Some(storage) = registry.get_cpu::<BoxGravity>() {
        for (&entity, solid) in storage.iter() {
            sources.push(Source {
                position: position_of(entity),
                level: level_of(entity),
                kind: Kind::Solid {
                    settings: *solid,
                    local: local_space(matrix_of(entity)),
                },
            });
        }
    }
    if let Some(storage) = registry.get_cpu::<PlaneGravity>() {
        for (&entity, plane) in storage.iter() {
            sources.push(Source {
                position: position_of(entity),
                level: level_of(entity),
                kind: Kind::Plane {
                    settings: *plane,
                    local: local_space(matrix_of(entity)),
                },
            });
        }
    }

    let levels = descending_levels(&sources);
    Field { sources, levels }
}

/// The distinct levels present, highest first — the order suppression
/// cascades in.
fn descending_levels(sources: &[Source]) -> Vec<i32> {
    let mut levels: Vec<i32> = sources.iter().map(|source| source.level).collect();
    levels.sort_unstable_by(|a, b| b.cmp(a));
    levels.dedup();
    levels
}

/// Splits a transform into what a local-space field needs.
///
/// The rotation alone, not the whole matrix: rotating a direction by a
/// scaled matrix would scale the acceleration, and a stretched zone is not
/// a stronger one.
fn local_space(matrix: Mat4) -> LocalSpace {
    LocalSpace {
        to_local: matrix.inverse(),
        rotation: matrix.to_scale_rotation_translation().1,
    }
}
