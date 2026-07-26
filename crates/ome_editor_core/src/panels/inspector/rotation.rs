//! Gimbal-safe Quat editing with Local/World display toggle (#202, #205).
//!
//! Caches Euler state per `(entity, type_id, field, mode)` so crossing
//! ±90° on any axis does not snap the others — the failure mode of a
//! naive per-frame Quat→Euler→Quat round-trip.

use std::any::TypeId;
use std::collections::HashMap;

use glam::{EulerRot, Quat, Vec3};

use ome_ecs::entity::Entity;
use ome_ecs::reflect::ReflectValue;
use ome_ecs::transform::Transform;

use crate::state::{EulerCacheKey, RotationDisplayMode};

use super::{EULER_CACHE_EPS, RotationContext};

/// Identifies the specific `Transform.rotation` field, which is the
/// only Quat with a meaningful world-space interpretation.
pub(super) fn is_transform_rotation(type_id: TypeId, field_name: &str) -> bool {
    type_id == TypeId::of::<Transform>() && field_name == "rotation"
}

/// Renders a Quat field as XYZ Euler degrees with a persistent cache so
/// crossing ±90° on any axis does not snap the other two (gimbal lock
/// from a per-frame Quat→Euler→Quat round-trip). See #202.
///
/// When `context.mode == World` and `context.self_global` is available,
/// the field is displayed in world space; user edits are converted back
/// to local space via `parent_global.inverse()` before being returned.
/// The returned `ReflectValue::Quat` is always the local rotation so the
/// Transform storage never changes representation. See #205.
///
/// The cache is refreshed only when the displayed quaternion (local or
/// world depending on mode) differs from the reconstruction of the
/// cached Euler within [`EULER_CACHE_EPS`], i.e. when the rotation was
/// modified externally (scripting, undo, physics).
pub(super) fn draw_quat_with_cache(
    ui: &mut egui::Ui,
    entity: Entity,
    type_id: TypeId,
    field_name: &str,
    local_quat: Quat,
    context: RotationContext,
    euler_cache: &mut HashMap<EulerCacheKey, Vec3>,
) -> Option<ReflectValue> {
    let mode = context.mode;
    // In World mode, display the entity's world-space rotation; fall
    // back to the stored local if the GlobalTransform is not available.
    let display_quat = match mode {
        RotationDisplayMode::Local => local_quat,
        RotationDisplayMode::World => context.self_global.unwrap_or(local_quat),
    };

    let key: EulerCacheKey = (entity, type_id, field_name.to_owned(), mode);
    let euler = fresh_cached_euler(euler_cache, &key, display_quat);
    let mut dx = euler.x.to_degrees();
    let mut dy = euler.y.to_degrees();
    let mut dz = euler.z.to_degrees();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("x");
        changed |= ui
            .add(egui::DragValue::new(&mut dx).speed(0.5).suffix("\u{00b0}"))
            .changed();
        ui.label("y");
        changed |= ui
            .add(egui::DragValue::new(&mut dy).speed(0.5).suffix("\u{00b0}"))
            .changed();
        ui.label("z");
        changed |= ui
            .add(egui::DragValue::new(&mut dz).speed(0.5).suffix("\u{00b0}"))
            .changed();
    });
    if !changed {
        return None;
    }
    let new_euler = Vec3::new(dx.to_radians(), dy.to_radians(), dz.to_radians());
    euler_cache.insert(key, new_euler);
    let new_display_quat = Quat::from_euler(EulerRot::XYZ, new_euler.x, new_euler.y, new_euler.z);
    // Convert the edited display-space quaternion back to local space so
    // the Transform storage remains authoritative.
    let new_local = match mode {
        RotationDisplayMode::Local => new_display_quat,
        RotationDisplayMode::World => {
            let parent_inv = context
                .parent_global
                .map_or(Quat::IDENTITY, |p| p.inverse());
            parent_inv * new_display_quat
        }
    };
    Some(ReflectValue::Quat(new_local))
}

/// Returns the Euler angles the inspector should display for `actual`,
/// refreshing the cache when the underlying quaternion changed outside
/// the editor.
fn fresh_cached_euler(
    euler_cache: &mut HashMap<EulerCacheKey, Vec3>,
    key: &EulerCacheKey,
    actual: Quat,
) -> Vec3 {
    if let Some(cached) = euler_cache.get(key).copied() {
        let reconstructed = Quat::from_euler(EulerRot::XYZ, cached.x, cached.y, cached.z);
        if actual.dot(reconstructed).abs() > 1.0 - EULER_CACHE_EPS {
            return cached;
        }
    }
    let (x, y, z) = actual.to_euler(EulerRot::XYZ);
    let euler = Vec3::new(x, y, z);
    euler_cache.insert(key.clone(), euler);
    euler
}
