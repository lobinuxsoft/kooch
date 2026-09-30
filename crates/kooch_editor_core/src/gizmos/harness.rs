//! Reading back what a visualizer drew.

use glam::{Mat4, Vec3};

use kooch_gizmos::Gizmos;

use super::gravity::ARROW;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{GizmoBatch, MeshBatch, Visualizer};

/// Every segment drawn, as `(start, end)` in world space.
pub(crate) fn draw<C, V>(visualizer: &V, component: &C, matrix: Mat4) -> Vec<(Vec3, Vec3)>
where
    V: Visualizer<C>,
    C: kooch_ecs::component::Component,
{
    let mut lines = GizmoBatch::default();
    let mut meshes = MeshBatch::default();
    let mut gizmos = Gizmos::new(&mut lines, &mut meshes);
    visualizer.draw(component, &GlobalTransform { matrix }, &mut gizmos);
    lines.lines.iter().map(|s| (s.start, s.end)).collect()
}

/// The furthest any drawn point gets from the origin — how far the
/// gizmo claims the field reaches.
pub(crate) fn reach(segments: &[(Vec3, Vec3)]) -> f32 {
    segments
        .iter()
        .flat_map(|(a, b)| [a.length(), b.length()])
        .fold(0.0, f32::max)
}

/// The direction the longest segments run in, which for an arrow shaft
/// is the direction of the field.
pub(crate) fn shaft(segments: &[(Vec3, Vec3)]) -> Vec3 {
    segments
        .iter()
        .max_by(|x, y| (x.1 - x.0).length().total_cmp(&(y.1 - y.0).length()))
        .map(|(a, b)| (*b - *a).normalize())
        .expect("nothing was drawn")
}

/// Only the wire arrow shafts, as unit directions — the heads are short segments at the tip and
/// would drown the signal.
pub(crate) fn shafts(segments: &[(Vec3, Vec3)]) -> Vec<Vec3> {
    segments
        .iter()
        .filter(|(a, b)| ((*b - *a).length() - ARROW).abs() < 1e-3)
        .map(|(a, b)| (*b - *a).normalize())
        .collect()
}

/// Every arrow a visualizer drew, as a unit direction.
pub(crate) fn arrows<C, V>(visualizer: &V, component: &C, matrix: Mat4) -> Vec<Vec3>
where
    V: Visualizer<C>,
    C: kooch_ecs::component::Component,
{
    drawn_arrows(visualizer, component, matrix)
        .into_iter()
        .filter_map(|(base, tip)| (tip - base).try_normalize())
        .collect()
}

/// The same, as `(base, tip)` — for a test that cares where an arrow stands, not only where it
/// points.
pub(crate) fn drawn_arrows<C, V>(visualizer: &V, component: &C, matrix: Mat4) -> Vec<(Vec3, Vec3)>
where
    V: Visualizer<C>,
    C: kooch_ecs::component::Component,
{
    let mut lines = GizmoBatch::default();
    let mut meshes = MeshBatch::default();
    let mut gizmos = Gizmos::new(&mut lines, &mut meshes);
    visualizer.draw(component, &GlobalTransform { matrix }, &mut gizmos);
    meshes.arrows
}

/// Every segment a visualizer draws when it has the world to read, as `(start, end)`.
pub(crate) fn draw_with<C, V>(
    visualizer: &V,
    component: &C,
    matrix: Mat4,
    entity: kooch_ecs::entity::Entity,
    resources: &kooch_core::resource::Resources,
) -> Vec<(Vec3, Vec3)>
where
    V: Visualizer<C>,
    C: kooch_ecs::component::Component,
{
    let mut lines = GizmoBatch::default();
    let mut meshes = MeshBatch::default();
    let mut gizmos = Gizmos::new(&mut lines, &mut meshes);
    visualizer.draw_with(
        component,
        &GlobalTransform { matrix },
        entity,
        resources,
        &mut gizmos,
    );
    lines.lines.iter().map(|s| (s.start, s.end)).collect()
}
