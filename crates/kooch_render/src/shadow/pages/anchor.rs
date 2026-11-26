//! The sun clipmap's anchor (#2009): a point fixed in absolute space on the coarsest page grid, so
//! page keys and depth origins do not move when the floating origin rebases. Mirrors `sun_frame` in
//! `page_table.wgsl`.

use glam::{DVec3, Vec3};

use super::ClipmapConfig;

/// The sun's frame: right, up, and the direction it shines along.
pub(crate) fn sun_frame(sun: Vec3) -> (Vec3, Vec3, Vec3) {
    let f = sun.normalize_or(Vec3::NEG_Y);
    let up = if f.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let right = f.cross(up).normalize();
    (right, right.cross(f), f)
}

/// Where the anchor sits this frame.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(crate) struct SunAnchor {
    /// In local metres, near the eye.
    pub local: Vec3,
    /// In coarsest-page units along right, up and the sun; `w` the coarsest level.
    pub whole: [i32; 4],
}

impl SunAnchor {
    /// The coarsest grid point nearest the eye. Worked in f64 from the absolute eye, so the same
    /// world point gets the same page index whatever cell the origin is in.
    pub fn near(origin: DVec3, eye: Vec3, sun: Vec3, clipmap: ClipmapConfig, side: u32) -> Self {
        let top = clipmap.levels.saturating_sub(1);
        let unit = f64::from(clipmap.extent(top)) / f64::from(side.max(1));
        let (right, up, f) = sun_frame(sun);
        let axes = [right, up, f].map(|v| v.as_dvec3());
        let at = origin + eye.as_dvec3();
        let whole = axes.map(|v| (at.dot(v) / unit).round());
        let point =
            axes[0] * whole[0] * unit + axes[1] * whole[1] * unit + axes[2] * whole[2] * unit;
        // Wrapping, as the shader adds it: an identity modulo 2^32, never a position.
        let wrap = |w: f64| (w as i64) as i32;
        Self {
            local: (point - origin).as_vec3(),
            whole: [wrap(whole[0]), wrap(whole[1]), wrap(whole[2]), top as i32],
        }
    }

    /// The uniform's two words.
    pub fn words(&self) -> ([f32; 4], [i32; 4]) {
        let a = self.local;
        ([a.x, a.y, a.z, 0.0], self.whole)
    }

    /// The anchor's absolute page index along the sun at `level`, wrapping like the shader's.
    pub fn along(&self, level: u32) -> i32 {
        let top = self.whole[3].max(0) as u32;
        let shift = top - level.min(top);
        ((self.whole[2] as u32).wrapping_shl(shift)) as i32
    }
}
