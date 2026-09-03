//! Asking the world a question, from the vocabulary a system speaks.
//!
//! The backend answers in [`BodyHandle`]s and takes its own filter type;
//! a system holds a [`SolverBody`] and has no way to name either. These
//! wrappers are that translation, and [`PhysicsWorld::without`] is the
//! one that matters — "everything but me" is the most common filter
//! there is and could not otherwise be built by the code that needs it.

use glam::Vec3;

use crate::backend::{BodyHandle, PointHit, QueryFilter, RayHit, ShapeAt, ShapeHit};

use super::{PhysicsWorld, SolverBody};

impl PhysicsWorld {
    /// First thing a ray meets, or `None` for empty space.
    ///
    /// `direction` need not be normalised; `max_distance` is measured in
    /// its lengths. Not tied to a body — it is here so that asking the
    /// world a question does not require finding the backend first.
    pub fn raycast(&self, origin: Vec3, direction: Vec3, max_distance: f32) -> Option<RayHit> {
        self.raycast_where(origin, direction, max_distance, QueryFilter::ALL)
    }

    /// A filter blind to one body, named the way game code names bodies.
    ///
    /// [`QueryFilter::excluding`] takes a [`BodyHandle`], which is the
    /// backend's vocabulary and deliberately not reachable from a system.
    /// Without this the most common filter there is — "everything but
    /// me" — could not be built by the code that needs it most.
    ///
    /// A stale [`SolverBody`] gives an unfiltered query rather than a
    /// blind one: seeing too much is recoverable, and silently seeing
    /// nothing is a character standing on air.
    pub fn without(&self, body: SolverBody) -> QueryFilter {
        match self.handle(body.slot()) {
            Some(handle) => QueryFilter::excluding(handle),
            None => QueryFilter::ALL,
        }
    }

    /// The same, seeing only what a filter allows.
    ///
    /// A body probing its own surroundings wants
    /// [`QueryFilter::excluding`] itself: a downward ray from a
    /// character's centre finds the character first, every time.
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

    /// Every hit along a ray, unordered, for something that pierces.
    pub fn raycast_all(
        &self,
        origin: Vec3,
        direction: Vec3,
        max_distance: f32,
        filter: QueryFilter,
        out: &mut dyn FnMut(RayHit) -> bool,
    ) {
        self.backend()
            .query_ray_all(origin, direction, max_distance, filter, out);
    }

    /// Sweeps a shape and returns the first thing it meets.
    ///
    /// What a character controller tests a move with: a ray is a line of
    /// zero width and will slip between two crates a body cannot fit
    /// through.
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
