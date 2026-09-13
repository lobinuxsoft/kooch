//! The solver's own account of itself, drawn in the viewport.

use kooch_core::resource::Resources;
use kooch_gizmos::GizmoBatch;
use kooch_physics::backend::{DebugCategories, DebugLine};
use kooch_physics::plugin::PhysicsWorld;

/// Which parts of the solver the viewport is currently drawing.
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
    // switch down, nothing is walked and nothing crosses the wire.
    if overlay.is_active() {
        overlay.scratch.clear();
        let categories = overlay.categories;
        if !collect_remote(resources, categories, &mut overlay.scratch)
            && let Some(world) = resources.get::<PhysicsWorld>()
        {
            world
                .backend()
                .debug_lines(categories, &mut overlay.scratch);
        }
        for line in &overlay.scratch {
            batch.line(line.start, line.end, line.color);
        }
    }
    resources.insert(overlay);
}

/// Asks the mirrored project for its solver's segments.
fn collect_remote(
    resources: &Resources,
    categories: DebugCategories,
    out: &mut Vec<DebugLine>,
) -> bool {
    use kooch_remote::protocol::{Method, ResponseData};

    let Some(state) = resources.get::<crate::remote_session::RemoteState>() else {
        return false;
    };
    let Some(session) = state.session.as_ref().filter(|_| state.is_connected()) else {
        return false;
    };

    let payload = kooch_remote::serde_json::json!({
        "collider_shapes": categories.collider_shapes,
        "contacts": categories.contacts,
        "joints": categories.joints,
        "collider_aabbs": categories.collider_aabbs,
        "body_axes": categories.body_axes,
    });
    let response = session.client().call(Method::Extension {
        name: "physics.debug_lines".to_owned(),
        payload,
    });

    match response {
        Ok(ResponseData::Extension { result, .. }) => {
            out.extend(parse_lines(&result));
        }
        // A host built without physics has no such extension. Said once
        // would be better than never, but this runs every frame the
        // overlay is on, and a log line per frame is not a diagnostic.
        Err(_) | Ok(_) => {}
    }
    true
}

/// The segments out of an extension's reply.
fn parse_lines(result: &kooch_remote::serde_json::Value) -> Vec<DebugLine> {
    let Some(lines) = result.get("lines").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    lines
        .iter()
        .filter_map(|line| {
            Some(DebugLine {
                start: vec3(line.get("start")?)?,
                end: vec3(line.get("end")?)?,
                color: vec3(line.get("color")?)?,
            })
        })
        .collect()
}

fn vec3(value: &kooch_remote::serde_json::Value) -> Option<glam::Vec3> {
    let array = value.as_array()?;
    let [x, y, z] = array.as_slice() else {
        return None;
    };
    Some(glam::Vec3::new(
        x.as_f64()? as f32,
        y.as_f64()? as f32,
        z.as_f64()? as f32,
    ))
}

#[cfg(test)]
mod tests;
