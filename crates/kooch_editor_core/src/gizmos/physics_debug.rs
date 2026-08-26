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

use kooch_core::resource::Resources;
use kooch_gizmos::GizmoBatch;
use kooch_physics::backend::{DebugCategories, DebugLine};
use kooch_physics::plugin::PhysicsWorld;

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
///
/// # Where the solver is
///
/// Usually not here. The editor registers physics components without a
/// solver, and a mirrored project's solver runs in its own process, so
/// asking the local `PhysicsWorld` finds nothing in either editor mode —
/// which is what made this overlay draw a blank until #634.
///
/// So: ask the host when mirroring one, and the local world otherwise. The
/// local branch is what an embedded host uses, and what the tests exercise.
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
///
/// Returns `false` when there is no session to ask, so the caller falls
/// back to a local world. A session that *is* connected and fails still
/// returns `true`: the answer is "the host has no physics", and falling
/// back to a local world that mirrors nothing would draw a different
/// world's state, which is worse than drawing none.
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
///
/// Silently skips anything malformed rather than failing the frame: a
/// newer host sending a shape this build does not understand should cost
/// the lines it sent, not the overlay.
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
