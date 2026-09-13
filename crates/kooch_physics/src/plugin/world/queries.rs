//! Queries in a system's vocabulary: [`SolverBody`] in, not [`BodyHandle`].
//! [`PhysicsWorld::without`] matters most — "everything but me".

use glam::Vec3;

use crate::backend::{BodyHandle, PointHit, QueryFilter, RayHit, ShapeAt, ShapeHit};

use super::{PhysicsWorld, SolverBody};

impl PhysicsWorld {
    /// First ray hit, `None` for empty space; `direction` unnormalised, `max_distance` in its
    /// lengths.
    pub fn raycast(&self, origin: Vec3, direction: Vec3, max_distance: f32) -> Option<RayHit> {
        self.raycast_where(origin, direction, max_distance, QueryFilter::ALL)
    }

    /// A filter blind to one body, since [`QueryFilter::excluding`] takes an unreachable
    /// [`BodyHandle`]. A stale body gives an unfiltered query: seeing too much is recoverable.
    pub fn without(&self, body: SolverBody) -> QueryFilter {
        match self.handle(body.slot()) {
            Some(handle) => QueryFilter::excluding(handle),
            None => QueryFilter::ALL,
        }
    }

    /// The same through a filter; a body probing its surroundings excludes itself, or a downward
    /// ray finds it first.
    pub fn raycast_where(
        &self,
        origin: Vec3,
        direction: Vec3,
        max_distance: f32,
        filter: QueryFilter,
    ) -> Option<RayHit> {
        self.backend()
            .query_ray(origin, direction, max_distance, filter)
    }

    /// Sweeps a shape to its first hit — a character controller's move test.
    pub fn sweep(
        &self,
        shape: ShapeAt<'_>,
        direction: Vec3,
        max_distance: f32,
        filter: QueryFilter,
    ) -> Option<ShapeHit> {
        self.backend()
            .query_sweep(shape, direction, max_distance, filter)
    }

    /// Nearest point on the nearest body, and whether `point` is inside
    /// it.
    pub fn project_point(
        &self,
        point: Vec3,
        max_distance: f32,
        filter: QueryFilter,
    ) -> Option<PointHit> {
        self.backend().query_point(point, max_distance, filter)
    }

    /// Every body a shape overlaps where it stands, moving nothing.
    pub fn overlaps(
        &self,
        shape: ShapeAt<'_>,
        filter: QueryFilter,
        out: &mut dyn FnMut(BodyHandle) -> bool,
    ) {
        self.backend().query_overlaps(shape, filter, out);
    }
}
