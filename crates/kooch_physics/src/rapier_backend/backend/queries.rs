//! The scene queries, translated both ways.
//!
//! Kept apart from the trait impl because it is the one place that has to
//! speak both vocabularies at once — engine filters and glam on one side,
//! rapier's pipeline and parry's hit records on the other — and mixing it
//! into the contract would bury that seam.

use glam::Vec3;
use rapier3d::geometry::ColliderHandle as RapierColliderHandle;
use rapier3d::parry::query::{ShapeCastOptions, ShapeCastStatus};
use rapier3d::prelude::*;

use crate::backend::{
    BodyHandle, PointHit, QueryFilter as EngineFilter, RayHit, ShapeAt, ShapeHit,
};

use super::super::conv::groups;
use super::super::shapes::shape_builder;
use super::RapierBackend;

impl RapierBackend {
    /// A query pipeline narrowed by an engine filter.
    ///
    /// Since 0.34 this is a view borrowed from the broad-phase BVH rather
    /// than a mirror kept in sync by hand, so it always sees the current
    /// colliders with no `update` call to forget after a spawn or a
    /// teleport — which is what lets the editor query a world nobody is
    /// stepping.
    fn pipeline(&self, filter: EngineFilter) -> QueryPipeline<'_> {
        let mut rapier = QueryFilter::default().groups(groups(filter.groups));
        if let Some(body) = filter.exclude
            && let Some(handle) = self.handles.get(body)
        {
            rapier = rapier.exclude_rigid_body(*handle);
        }
        if filter.skip_sensors {
            rapier = rapier.exclude_sensors();
        }
        self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            rapier,
        )
    }

    /// Whose collider that was.
    fn body_of(&self, collider: RapierColliderHandle) -> Option<BodyHandle> {
        let parent = self.colliders.get(collider)?.parent()?;
        self.body_lookup.get(&parent).copied()
    }

    pub(super) fn ray(
        &self,
        origin: Vec3,
        dir: Vec3,
        max_t: f32,
        filter: EngineFilter,
    ) -> Option<RayHit> {
        let ray = Ray::new(origin, dir);
        let (collider, hit) = self
            .pipeline(filter)
            .cast_ray_and_get_normal(&ray, max_t, true)?;
        Some(RayHit {
            body: self.body_of(collider)?,
            t: hit.time_of_impact,
            point: ray.origin + ray.dir * hit.time_of_impact,
            normal: hit.normal,
        })
    }

    pub(super) fn rays(
        &self,
        origin: Vec3,
        dir: Vec3,
        max_t: f32,
        filter: EngineFilter,
        out: &mut dyn FnMut(RayHit) -> bool,
    ) {
        let ray = Ray::new(origin, dir);
        for (collider, _, hit) in self.pipeline(filter).intersect_ray(ray, max_t, true) {
            let Some(body) = self.body_of(collider) else {
                continue;
            };
            let carry = out(RayHit {
                body,
                t: hit.time_of_impact,
                point: ray.origin + ray.dir * hit.time_of_impact,
                normal: hit.normal,
            });
            if !carry {
                return;
            }
        }
    }

    pub(super) fn sweep(
        &self,
        shape: ShapeAt<'_>,
        dir: Vec3,
        max_t: f32,
        filter: EngineFilter,
    ) -> Option<ShapeHit> {
        let builder = shape_builder(shape.shape).ok()?;
        let pose = Pose::from_parts(shape.origin, shape.rotation);
        let options = ShapeCastOptions {
            max_time_of_impact: max_t,
            // A cast that begins already touching reports it rather than
            // saying nothing: a controller stuck in geometry needs to
            // know, and silence reads as open space.
            stop_at_penetration: true,
            compute_impact_geometry_on_penetration: true,
            ..Default::default()
        };
        let (collider, hit) =
            self.pipeline(filter)
                .cast_shape(&pose, dir, builder.shape.as_ref(), options)?;

        // Shape *one* is the world: the pipeline casts the composite of
        // every collider against the shape handed in, so `witness1` and
        // `normal1` describe the surface that was hit, and shape one's
        // frame is world space. `normal2` is its negation in the swept
        // shape's frame — pointing into the wall, which reads plausible
        // and is wrong. Rapier's own character controller compares
        // `normal1` against world up for the same reason.
        Some(ShapeHit {
            body: self.body_of(collider)?,
            t: hit.time_of_impact,
            point: hit.witness1,
            normal: hit.normal1,
            penetrating: matches!(hit.status, ShapeCastStatus::PenetratingOrWithinTargetDist),
        })
    }

    pub(super) fn point(
        &self,
        point: Vec3,
        max_distance: f32,
        filter: EngineFilter,
    ) -> Option<PointHit> {
        let (collider, projection) =
            self.pipeline(filter)
                .project_point(point, max_distance, true)?;
        Some(PointHit {
            body: self.body_of(collider)?,
            point: projection.point,
            inside: projection.is_inside,
        })
    }

    pub(super) fn overlaps(
        &self,
        shape: ShapeAt<'_>,
        filter: EngineFilter,
        out: &mut dyn FnMut(BodyHandle) -> bool,
    ) {
        let Ok(builder) = shape_builder(shape.shape) else {
            return;
        };
        let pose = Pose::from_parts(shape.origin, shape.rotation);
        // Collected first: the iterator borrows the pipeline, which
        // borrows `self`, and the callback may want to ask another
        // question. One `Vec` per call is the cost of that, and an
        // overlap query is not a per-frame-per-agent path the way a
        // sweep is.
        let hits: Vec<RapierColliderHandle> = self
            .pipeline(filter)
            .intersect_shape(pose, builder.shape.as_ref())
            .map(|(handle, _)| handle)
            .collect();
        // A body with several colliders overlaps once, not once per
        // shape: the caller asked which bodies, not which pieces.
        let mut seen = std::collections::HashSet::new();
        for collider in hits {
            let Some(body) = self.body_of(collider) else {
                continue;
            };
            if !seen.insert(body) {
                continue;
            }
            if !out(body) {
                return;
            }
        }
    }
}
