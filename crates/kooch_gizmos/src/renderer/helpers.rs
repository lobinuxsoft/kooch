use glam::{Mat4, Vec3, Vec4};
use kooch_core::resource::Resources;
use kooch_ecs::PerspectiveCamera;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::query::Query;

use super::types::{GizmoVertex, LineSegment};

/// Generates the 6 vertices (2 triangles) forming the screen-space
/// quad for one [`LineSegment`].
pub(super) fn push_quad(seg: &LineSegment, vertices: &mut Vec<GizmoVertex>) {
    let p1 = seg.start.to_array();
    let p2 = seg.end.to_array();
    let color = seg.color.to_array();
    let thickness = seg.thickness;

    // Quad corners: A/B = p1 ± perp, C/D = p2 ± perp. The shader derives `perp` from the other
    // endpoint, which reverses at p2, so `side` flips there.
    let a = GizmoVertex {
        position: p1,
        color,
        other_position: p2,
        side: 1.0,
        thickness,
    };
    let b = GizmoVertex {
        position: p1,
        color,
        other_position: p2,
        side: -1.0,
        thickness,
    };
    let c = GizmoVertex {
        position: p2,
        color,
        other_position: p1,
        side: -1.0,
        thickness,
    };
    let d = GizmoVertex {
        position: p2,
        color,
        other_position: p1,
        side: 1.0,
        thickness,
    };

    // Triangle 1: A, B, C.  Triangle 2: B, D, C.
    vertices.push(a);
    vertices.push(b);
    vertices.push(c);
    vertices.push(b);
    vertices.push(d);
    vertices.push(c);
}

/// The camera the gizmos belong to: the active one of highest priority. An overlay is not that view
/// (#1221): it draws over the base, and a high-priority one would otherwise hand them its lens.
fn active_camera(resources: &Resources) -> Option<(PerspectiveCamera, Mat4)> {
    let query = Query::<(&PerspectiveCamera, &GlobalTransform)>::new(resources);
    let mut best: Option<(PerspectiveCamera, Mat4)> = None;
    query.for_each(|(cam, gt)| {
        if !cam.active || cam.overlay {
            return;
        }
        if best.is_none_or(|(held, _)| cam.priority > held.priority) {
            best = Some((*cam, gt.matrix));
        }
    });
    drop(query);
    best
}

/// The active camera's position, and its view-projection with that position taken out. The grid
/// unprojects through the second: with the eye in the matrix, two points 0.1 m apart a kilometre
/// from the origin differ by a few f32 ulps, and the ray built from them shook (#1992).
pub(super) fn camera_eye(resources: &Resources, aspect: f32) -> Option<(Vec3, Mat4)> {
    let (cam, mut world) = active_camera(resources)?;
    let eye = world.w_axis.truncate();
    world.w_axis = Vec4::W;
    Some((eye, projection(&cam, aspect) * world.inverse()))
}

fn projection(cam: &PerspectiveCamera, aspect: f32) -> Mat4 {
    kooch_render::projection::perspective_infinite_rh_reverse_z(
        cam.fov.to_radians(),
        aspect.max(0.001),
        cam.near.max(0.001),
    )
}

/// 🔴 Infinite, like the scene's (#1992): cut at `camera.far`, everything past it vanished, and the
/// depth the grid writes disagreed with the scene's it is tested against.
pub(crate) fn active_camera_view_proj(resources: &Resources, aspect: f32) -> Option<Mat4> {
    let (cam, world) = active_camera(resources)?;
    Some(projection(&cam, aspect) * world.inverse())
}
