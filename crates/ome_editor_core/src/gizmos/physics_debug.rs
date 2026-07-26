//! The solver's own account of itself, drawn in the viewport.
//!
//! # Not a second collider outline
//!
//! [`ColliderVisualizer`](super::collider::ColliderVisualizer) already
//! draws colliders, from the ECS components, and it is the right tool for
//! "does this shape wrap my model". This is the other question.
//!
//! Drawing the components is the same arithmetic the sync layer does,
//! performed twice. If the body was never built, or was built from a spec
//! that has since gone stale, the component gizmo draws the shape that
//! *ought* to exist and cannot say a word about the one that does. **When
//! the two disagree, the disagreement is the bug** — so `collider_shapes`
//! is off by default here, and switching it on is an act of comparison,
//! not a second opinion.
//!
//! The categories that have no component equivalent at all are the reason
//! this exists: contacts, centres of mass, joint anchors, broad-phase
//! bounds, and which bodies the solver has stopped simulating.
//!
//! # Cost
//!
//! The walk is CPU work every frame, proportional to shape count times
//! tessellation. Nothing is on by default and nothing is asked for when
//! everything is off — the backend is not even called, so an unused
//! overlay costs one boolean check.

use ome_core::resource::Resources;
use ome_gizmos::GizmoBatch;
use ome_physics::backend::{DebugCategories, DebugLine};
use ome_physics::plugin::PhysicsWorld;

/// Which parts of the solver the viewport is currently drawing.
///
/// Editor state, deliberately not a component: it describes the tool, not
/// the scene, so it must not reach a scene file. Absent from `Resources`
/// reads as everything off, which is what a host that never inserted one
/// should get.
#[derive(Debug, Default)]
pub(crate) struct PhysicsDebugOverlay {
    pub(crate) categories: DebugCategories,
    /// Reused between frames so a live overlay is not sixty allocations a
    /// second. The batch copies what it needs.
    scratch: Vec<DebugLine>,
}

impl PhysicsDebugOverlay {
    /// An overlay drawing the given categories.
    pub(crate) fn new(categories: DebugCategories) -> Self {
        Self {
            categories,
            scratch: Vec::new(),
        }
    }

    /// Whether the overlay draws anything at all right now.
    pub(crate) fn is_active(&self) -> bool {
        self.categories.any()
    }
}

/// Appends the solver's description of the world to the line batch.
pub(super) fn draw(resources: &mut Resources, batch: &mut GizmoBatch) {
    let Some(mut overlay) = resources.remove::<PhysicsDebugOverlay>() else {
        return;
    };
    // The early out that makes "costs nothing when off" true: with every
    // switch down, the physics world is never walked.
    if overlay.is_active()
        && let Some(world) = resources.get::<PhysicsWorld>()
    {
        overlay.scratch.clear();
        let categories = overlay.categories;
        world
            .backend()
            .debug_lines(categories, &mut overlay.scratch);
        for line in &overlay.scratch {
            batch.line(line.start, line.end, line.color);
        }
    }
    resources.insert(overlay);
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use ome_physics::backend::{BodyDesc, CollisionShape, PhysicsBackend};
    use ome_physics::rapier_backend::RapierBackend;

    fn world_with_a_body() -> Resources {
        let mut backend = RapierBackend::new();
        backend.add_body(BodyDesc::dynamic(
            CollisionShape::Sphere { radius: 0.5 },
            1.0,
        ));
        let mut resources = Resources::new();
        resources.insert(PhysicsWorld::new(Box::new(backend)));
        resources
    }

    fn line_count(batch: &GizmoBatch) -> usize {
        batch.lines.len()
    }

    /// A host that never inserted the resource must not pay for the
    /// overlay, or every headless tool walks the physics world for nothing.
    #[test]
    fn without_the_resource_nothing_is_drawn() {
        let mut resources = world_with_a_body();
        let mut batch = GizmoBatch::default();

        draw(&mut resources, &mut batch);

        assert_eq!(line_count(&batch), 0);
    }

    /// The default is every switch down, and that has to mean the walk
    /// never happens rather than happening and drawing nothing.
    #[test]
    fn the_overlay_is_off_by_default() {
        let mut resources = world_with_a_body();
        resources.insert(PhysicsDebugOverlay::default());
        let mut batch = GizmoBatch::default();

        draw(&mut resources, &mut batch);

        assert_eq!(line_count(&batch), 0);
        assert!(!resources.get::<PhysicsDebugOverlay>().unwrap().is_active());
    }

    #[test]
    fn switching_a_category_on_draws_geometry() {
        let mut resources = world_with_a_body();
        resources.insert(PhysicsDebugOverlay {
            categories: DebugCategories {
                collider_shapes: true,
                ..Default::default()
            },
            ..Default::default()
        });
        let mut batch = GizmoBatch::default();

        draw(&mut resources, &mut batch);

        assert!(line_count(&batch) > 0, "the overlay drew nothing");
    }

    /// The resource has to go back, or the overlay works for exactly one
    /// frame and then silently turns itself off.
    #[test]
    fn the_overlay_survives_a_frame() {
        let mut resources = world_with_a_body();
        resources.insert(PhysicsDebugOverlay {
            categories: DebugCategories::all(),
            ..Default::default()
        });
        let mut batch = GizmoBatch::default();

        draw(&mut resources, &mut batch);
        let first = line_count(&batch);
        batch.clear();
        draw(&mut resources, &mut batch);

        assert!(resources.get::<PhysicsDebugOverlay>().is_some());
        assert_eq!(line_count(&batch), first, "the second frame differs");
    }

    /// The scratch buffer is reused, so it has to be cleared or the
    /// overlay grows without bound while it is on.
    #[test]
    fn the_scratch_buffer_does_not_accumulate() {
        let mut resources = world_with_a_body();
        resources.insert(PhysicsDebugOverlay {
            categories: DebugCategories::all(),
            ..Default::default()
        });
        let mut batch = GizmoBatch::default();

        draw(&mut resources, &mut batch);
        let first = resources
            .get::<PhysicsDebugOverlay>()
            .unwrap()
            .scratch
            .len();
        draw(&mut resources, &mut batch);
        let second = resources
            .get::<PhysicsDebugOverlay>()
            .unwrap()
            .scratch
            .len();

        assert_eq!(first, second, "the buffer grew between frames");
    }

    /// Sanity on the whole path: an empty world has nothing to say even
    /// with everything switched on.
    #[test]
    fn an_empty_world_draws_nothing() {
        let mut resources = Resources::new();
        resources.insert(PhysicsWorld::new(Box::new(RapierBackend::new())));
        resources.insert(PhysicsDebugOverlay {
            categories: DebugCategories::all(),
            ..Default::default()
        });
        let mut batch = GizmoBatch::default();

        draw(&mut resources, &mut batch);

        assert_eq!(line_count(&batch), 0);
        let _ = Vec3::ZERO;
    }
}
