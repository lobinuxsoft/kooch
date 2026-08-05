//! Editor camera controller state — the *navigation* state of the editor
//! camera, distinct from the camera entity's `Transform`.
//!
//! The `Transform` answers "where is the camera and where does it look".
//! This controller answers "what is the orbit pivot, how far is the
//! camera from it, and how sensitive are the controls". Orbit, pan, zoom
//! and fly each mutate `Transform` *and* this controller in lockstep so
//! state stays consistent across mode switches.

use glam::Vec3;

/// Persistent state of the editor camera controller.
///
/// Kept as a `Resource`, not on the camera entity, because it represents
/// **editor input intent** (which can outlive any specific camera entity
/// or be retargeted at one) rather than transform data.
#[derive(Debug, Clone)]
pub struct EditorCameraController {
    /// World-space point the camera orbits around. Moves with the camera
    /// in fly mode so that subsequent orbits feel natural rather than
    /// snapping back to the origin.
    pub focus_point: Vec3,
    /// Distance from `focus_point` to the camera position along the
    /// camera's forward axis. Clamped to `[min_distance, max_distance]`.
    pub distance: f32,
    /// Lower bound on `distance` to prevent the camera from passing
    /// through the focus point.
    pub min_distance: f32,
    /// Upper bound on `distance` to keep the scene framed.
    pub max_distance: f32,

    /// Orbit rotation per pixel of mouse drag (radians).
    pub orbit_sensitivity: f32,
    /// Pan world-units per pixel of mouse drag, scaled by `distance` so
    /// pan feels constant across zoom levels.
    pub pan_sensitivity_factor: f32,
    /// Zoom multiplier per scroll line. `1.1` = zoom in 10% per tick.
    pub zoom_sensitivity: f32,
    /// Fly translation speed in world-units per second.
    pub fly_speed: f32,
    /// Fly mode look rotation per pixel of mouse drag (radians).
    pub fly_look_sensitivity: f32,
}

impl Default for EditorCameraController {
    /// Defaults tuned for a Unity/Godot-familiar feel:
    /// - 0.005 rad/px ≈ 0.29°/px orbit
    /// - 0.0015 × distance pan factor — at distance 10 that's ~1.5%/px
    /// - 1.1× zoom per scroll tick
    /// - 5 units/sec fly base speed
    fn default() -> Self {
        Self {
            focus_point: Vec3::ZERO,
            distance: 10.0,
            min_distance: 0.1,
            max_distance: 10_000.0,
            orbit_sensitivity: 0.005,
            pan_sensitivity_factor: 0.0015,
            zoom_sensitivity: 1.1,
            fly_speed: 5.0,
            fly_look_sensitivity: 0.003,
        }
    }
}

impl EditorCameraController {
    /// Clamps `distance` into `[min_distance, max_distance]`.
    pub fn clamp_distance(&mut self) {
        self.distance = self.distance.clamp(self.min_distance, self.max_distance);
    }

    /// Returns the effective pan sensitivity in world-units per pixel
    /// at the current distance.
    pub fn effective_pan_sensitivity(&self) -> f32 {
        self.pan_sensitivity_factor * self.distance
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let c = EditorCameraController::default();
        assert_eq!(c.focus_point, Vec3::ZERO);
        assert!(c.min_distance < c.distance);
        assert!(c.distance < c.max_distance);
    }

    #[test]
    fn clamp_distance_enforces_bounds() {
        let mut c = EditorCameraController::default();
        c.distance = 50_000.0;
        c.clamp_distance();
        assert_eq!(c.distance, c.max_distance);

        c.distance = -1.0;
        c.clamp_distance();
        assert_eq!(c.distance, c.min_distance);
    }

    #[test]
    fn pan_sensitivity_scales_with_distance() {
        let mut c = EditorCameraController::default();
        c.distance = 1.0;
        let near = c.effective_pan_sensitivity();
        c.distance = 100.0;
        let far = c.effective_pan_sensitivity();
        assert!(far > near, "pan should be slower up-close, faster far-away");
    }
}
