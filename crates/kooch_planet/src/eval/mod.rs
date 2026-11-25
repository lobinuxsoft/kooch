//! The generator on the CPU (#313 step 3b): naga's IR of a `planet` shader, walked over intervals.
//!
//! [`Generator::range`] bounds the SDF over a box, so `choose_lods` can give no slot to a block the
//! generator proves all rock or all air (Zylann's range analysis). [`Generator::sample`] is the
//! same walk at one point, rounded as the GPU rounds, for the collision of step 6. Any construct
//! it cannot bound makes the answer unknown, never wrong.

mod math;
mod num;
mod ops;
mod run;
#[cfg(test)]
mod tests;
mod value;

use glam::DVec3;
use naga::{Expression as E, Function, Handle, Module};

use crate::PLANET_INPUT_WGSL;
use crate::lod::Shape;
use num::{Mode, Num, to_f32};
use run::Machine;
use value::{Kind, Lanes, Value, zero};

/// Why an evaluation gave up; every one of them leaves the block a candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fault {
    Unsupported(&'static str),
    /// A branch or loop whose way depends on where in the box the sample is.
    Unbounded,
    Budget,
}

/// A planet's generator, parsed once and evaluated on the CPU.
pub struct Generator {
    module: Module,
    planet: Handle<Function>,
    constants: Vec<Option<Value>>,
}

impl Generator {
    /// `generator` as a `planet` shader holds it: the fragment that defines `fn planet`.
    pub fn parse(generator: &str) -> Result<Self, String> {
        let source = format!("{PLANET_INPUT_WGSL}{generator}");
        let module =
            naga::front::wgsl::parse_str(&source).map_err(|e| e.emit_to_string(&source))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .map_err(|e| e.emit_to_string(&source))?;
        let planet = module
            .functions
            .iter()
            .find(|(_, f)| f.name.as_deref() == Some("planet"))
            .map(|(h, _)| h)
            .ok_or("the generator has no `fn planet`")?;
        let constants = constants(&module);
        Ok(Self {
            module,
            planet,
            constants,
        })
    }

    /// The SDF anywhere in the box from `min` to `max` (planet space, metres), or `None` when
    /// the generator cannot be bounded there.
    pub fn range(&self, shape: &Shape, min: DVec3, max: DVec3, planet: u32) -> Option<(f64, f64)> {
        let input = range_input(&self.module, shape, min, max, planet).ok()?;
        let out = self.run(input, Mode::Range).ok()?;
        (out.lo.is_finite() && out.hi.is_finite()).then_some((out.lo, out.hi))
    }

    /// The SDF at `point`, as the GPU computes it to f32 rounding.
    pub fn sample(&self, shape: &Shape, point: DVec3, planet: u32) -> Option<f32> {
        let distance = point.length();
        let input = PlanetValue {
            position: point.to_array().map(Num::point),
            altitude: Num::point(distance - shape.radius),
            direction: (point / distance.max(1e-6)).to_array().map(Num::point),
            radius: shape.radius,
            planet,
        };
        let value = input.value(&self.module, Mode::Point).ok()?;
        let out = self.run(value, Mode::Point).ok()?;
        out.is_point().then_some(out.lo as f32)
    }

    fn run(&self, input: Value, mode: Mode) -> Result<Num, Fault> {
        let mut machine = Machine::new(&self.module, &self.constants, mode);
        machine.call(self.planet, vec![input])?.num()
    }
}

/// Whether a block whose samples the generator bounds by `range` can hold surface. The GPU stores
/// `sdf / voxel` in f16, so a value within a hair of zero may land on either side.
pub(crate) fn may_cross(range: Option<(f64, f64)>, voxel: f64) -> bool {
    let margin = voxel * 1e-4;
    range.is_none_or(|(lo, hi)| lo <= margin && hi >= -margin)
}

/// `PlanetInput` as intervals, by member name.
struct PlanetValue {
    position: [Num; 3],
    altitude: Num,
    direction: [Num; 3],
    radius: f64,
    planet: u32,
}

impl PlanetValue {
    fn value(&self, module: &Module, mode: Mode) -> Result<Value, Fault> {
        let ty = module
            .types
            .iter()
            .find(|(_, t)| t.name.as_deref() == Some("PlanetInput"))
            .map(|(h, _)| h)
            .ok_or(Fault::Unsupported("no PlanetInput"))?;
        let naga::TypeInner::Struct { members, .. } = &module.types[ty].inner else {
            return Err(Fault::Unsupported("PlanetInput is not a struct"));
        };
        let vector = |n: [Num; 3], ulps| {
            let mut lanes = Lanes::splat(Kind::F32, 3, Num::ANY);
            for (lane, n) in lanes.n.iter_mut().zip(n) {
                *lane = to_f32(n, ulps, mode);
            }
            Value::Lanes(lanes)
        };
        let scalar = |kind, n| Value::Lanes(Lanes::scalar(kind, n));
        let parts = members
            .iter()
            .map(|member| match member.name.as_deref() {
                // The GPU adds the tile's f32 origin to an f32 offset: two roundings.
                Some("position") => Ok(vector(self.position, 2)),
                Some("direction") => Ok(vector(self.direction, 4)),
                Some("altitude") => Ok(scalar(Kind::F32, to_f32(self.altitude, 0, mode))),
                Some("radius") => Ok(scalar(Kind::F32, to_f32(Num::point(self.radius), 0, mode))),
                Some("planet") => Ok(scalar(Kind::U32, Num::point(f64::from(self.planet)))),
                _ => zero(module, member.ty),
            })
            .collect::<Result<_, _>>()?;
        Ok(Value::Parts(parts))
    }
}

/// Every sample of a box, as intervals: where it is, how high, which way is up.
fn range_input(
    module: &Module,
    shape: &Shape,
    min: DVec3,
    max: DVec3,
    planet: u32,
) -> Result<Value, Fault> {
    let near = DVec3::ZERO.clamp(min, max).length();
    let far = min.abs().max(max.abs()).length();
    // The GPU's altitude is exact to a few f32 steps of the excess it starts from.
    let slack = 1e-5 * (near - shape.radius).abs().max((far - shape.radius).abs()) + 1e-3;
    let altitude = Num::new(near - shape.radius, far - shape.radius).pad(slack);
    let distance = Num::new(near.max(1e-6), far.max(1e-6));
    let position = [0, 1, 2].map(|i| Num::new(min[i], max[i]));
    let direction = position.map(|p| {
        let n = p.div(distance);
        n.max(Num::point(-1.0)).min(Num::point(1.0))
    });
    PlanetValue {
        position,
        altitude,
        direction,
        radius: shape.radius,
        planet,
    }
    .value(module, Mode::Range)
}

/// The module's constants, which naga has already folded into literals and compositions.
fn constants(module: &Module) -> Vec<Option<Value>> {
    let mut out: Vec<Option<Value>> = Vec::with_capacity(module.constants.len());
    for (_, constant) in module.constants.iter() {
        let value = global(module, &out, constant.init).ok();
        out.push(value);
    }
    out
}

fn global(module: &Module, known: &[Option<Value>], h: Handle<E>) -> Result<Value, Fault> {
    match &module.global_expressions[h] {
        E::Literal(literal) => Ok(Value::Lanes(run::literal_lanes(*literal)?)),
        E::Constant(c) => known
            .get(c.index())
            .cloned()
            .flatten()
            .ok_or(Fault::Unsupported("a constant declared after its use")),
        E::ZeroValue(ty) => zero(module, *ty),
        E::Compose { ty, components } => {
            let parts = components
                .iter()
                .map(|&c| global(module, known, c))
                .collect::<Result<_, _>>()?;
            run::compose(module, *ty, parts)
        }
        E::Splat { size, value } => {
            let lane = global(module, known, *value)?.lanes()?;
            Ok(Value::Lanes(Lanes::splat(
                lane.kind,
                *size as u8,
                lane.n[0],
            )))
        }
        _ => Err(Fault::Unsupported("a constant naga did not fold")),
    }
}
