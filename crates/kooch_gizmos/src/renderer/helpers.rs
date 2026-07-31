use glam::Mat4;
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

    // The four logical corners of the quad in screen space:
    //   A = p1 + perp           B = p1 - perp
    //   C = p2 + perp           D = p2 - perp
    //
    // The shader computes `perp` from `(other_position - position)`. At
    // p2 that direction is reversed, so to keep C / D on the same world
    // sides as A / B we flip the `side` sign at p2.
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

pub(super) fn active_camera_view_proj(resources: &Resources, aspect: f32) -> Option<Mat4> {
    let query = Query::<(&PerspectiveCamera, &GlobalTransform)>::new(resources);
    let mut best: Option<(i32, PerspectiveCamera, Mat4)> = None;
    query.for_each(|(cam, gt)| {
        if !cam.active {
            return;
        }
        let better = match &best {
            Some((p, _, _)) => cam.priority > *p,
            None => true,
        };
        if better {
            best = Some((cam.priority, *cam, gt.matrix));
        }
    });
    drop(query);

    let (_, cam, world) = best?;
    let view = world.inverse();
    let projection = kooch_render::perspective_rh_reverse_z(
        cam.fov.to_radians(),
        aspect.max(0.001),
        cam.near.max(0.001),
        cam.far.max(cam.near + 0.001),
    );
    Some(projection * view)
}
