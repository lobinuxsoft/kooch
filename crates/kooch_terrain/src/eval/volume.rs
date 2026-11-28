//! A joined volume (#1957) on the CPU: its SDF in the stamp's space, at a point or over a box.

use glam::DVec3;

use super::num::{Mode, Num};
use super::{Generator, PlanetValue};

impl Generator {
    /// Volume `index`'s SDF at `p`, in the stamp's space; `radius` is what its input reads.
    pub(crate) fn volume(&self, index: u32, p: DVec3, radius: f64, planet: u32) -> Option<f64> {
        let value = local(p.to_array().map(Num::point), radius, planet);
        let out = self.run_volume(index, value, Mode::Point)?;
        out.is_point().then_some(out.lo)
    }

    /// Its SDF anywhere in the box `min..max` of the stamp's space, when it can be bounded.
    pub(crate) fn volume_range(
        &self,
        index: u32,
        (min, max): (DVec3, DVec3),
        radius: f64,
        planet: u32,
    ) -> Option<(f64, f64)> {
        let value = local([0, 1, 2].map(|i| Num::new(min[i], max[i])), radius, planet);
        let out = self.run_volume(index, value, Mode::Range)?;
        (out.lo.is_finite() && out.hi.is_finite()).then_some((out.lo, out.hi))
    }

    fn run_volume(&self, index: u32, value: PlanetValue, mode: Mode) -> Option<Num> {
        let function = *self.volumes.get(index as usize)?;
        let input = value.value(&self.module, mode).ok()?;
        let mut machine = super::Machine::new(&self.module, &self.constants, mode);
        machine.call(function, vec![input]).ok()?.num().ok()
    }
}

/// The input `generate.wgsl` gives a volume: a plane through the stamp's centre, up its `y`.
fn local(position: [Num; 3], radius: f64, planet: u32) -> PlanetValue {
    PlanetValue {
        altitude: position[1],
        position,
        direction: [0.0, 1.0, 0.0].map(Num::point),
        radius,
        planet,
        slope: 0.0,
    }
}
