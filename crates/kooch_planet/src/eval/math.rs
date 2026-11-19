//! WGSL's built-in functions over intervals. In [`Mode::Range`] each one is widened by the error
//! WGSL allows the device, so a bound holds on any GPU; where WGSL leaves the precision undefined
//! the bound is the function's whole codomain.

use std::f64::consts::{FRAC_PI_2, PI};

use naga::MathFunction as F;

use super::Fault;
use super::num::{Mode, Num};
use super::ops::fit;
use super::value::{Kind, Lanes};

/// WGSL's absolute error for `sin` and `cos` inside [−π, π].
const TRIG_ERROR: f64 = 1.0 / 2048.0;
/// WGSL's absolute error for `log` and `log2` near one.
const LOG_ERROR: f64 = 1.0 / 2_097_152.0;
/// `atan2` and what inherits from it may be 4096 ULP out.
const ATAN_ULPS: u32 = 4096;

/// `fun(args)`. `same` says the first two arguments are one expression, so `dot(v, v)` is a sum
/// of squares.
pub fn call(fun: F, args: &[Lanes], same: bool, mode: Mode) -> Result<Lanes, Fault> {
    let a = *args
        .first()
        .ok_or(Fault::Unsupported("a call without arguments"))?;
    let arg = |i: usize| {
        args.get(i)
            .copied()
            .ok_or(Fault::Unsupported("a call short of arguments"))
    };
    let kind = a.kind;
    Ok(match fun {
        F::Dot => Lanes::scalar(kind, dot(a, arg(1)?, same, mode)),
        F::Length => Lanes::scalar(kind, length(a, mode)),
        F::Distance => Lanes::scalar(kind, length(minus(a, arg(1)?, mode), mode)),
        F::Normalize => normalize(a, mode),
        F::Cross => cross(a, arg(1)?, mode),
        _ => lane_wise(fun, args, mode)?,
    })
}

fn lane_wise(fun: F, args: &[Lanes], mode: Mode) -> Result<Lanes, Fault> {
    let len = args.iter().map(|a| a.len).max().unwrap_or(1);
    let kind = args[0].kind;
    let mut out = Lanes::splat(kind, len, Num::ANY);
    for i in 0..usize::from(len) {
        let x: Vec<Num> = args.iter().map(|a| a.lane(i)).collect();
        out.n[i] = one(fun, kind, &x, mode)?;
    }
    Ok(out)
}

fn one(fun: F, kind: Kind, x: &[Num], mode: Mode) -> Result<Num, Fault> {
    let a = x[0];
    let b = || {
        x.get(1)
            .copied()
            .ok_or(Fault::Unsupported("a call short of arguments"))
    };
    let c = || {
        x.get(2)
            .copied()
            .ok_or(Fault::Unsupported("a call short of arguments"))
    };
    let exact = |n| Ok(fit(kind, n, 0, mode));
    let rounded = |n, ulps| Ok(fit(kind, n, ulps, mode));
    match fun {
        F::Abs => exact(abs(a)),
        F::Min => exact(a.min(b()?)),
        F::Max => exact(a.max(b()?)),
        F::Clamp => exact(a.max(b()?).min(c()?)),
        F::Saturate => exact(a.max(Num::point(0.0)).min(Num::point(1.0))),
        F::Sign => exact(a.rising(sign)),
        F::Floor => exact(a.rising(f64::floor)),
        F::Ceil => exact(a.rising(f64::ceil)),
        F::Trunc => exact(a.rising(f64::trunc)),
        F::Round => exact(a.rising(f64::round_ties_even)),
        F::Fract => exact(fract(a, mode)),
        F::Step => exact(step(a, b()?)),
        F::Radians => rounded(a.mul(Num::point(PI / 180.0)), 1),
        F::Degrees => rounded(a.mul(Num::point(180.0 / PI)), 1),
        F::Fma => rounded(a.mul(b()?).add(c()?), 1),
        F::Mix => rounded(mix(a, b()?, c()?), 2),
        F::SmoothStep => rounded(smoothstep(a, b()?, c()?), 4),
        F::Sin | F::Cos => Ok(trig(fun, a, mode)),
        F::Sqrt if a.lo >= 0.0 => rounded(a.rising(f64::sqrt), 5),
        F::InverseSqrt if a.lo > 0.0 => {
            rounded(Num::new(a.lo.sqrt().recip(), a.hi.sqrt().recip()), 2)
        }
        F::Exp => Ok(exp(a, f64::exp, mode)),
        F::Exp2 => Ok(exp(a, f64::exp2, mode)),
        F::Log if a.lo >= 0.0 => Ok(log(a, f64::ln, mode)),
        F::Log2 if a.lo >= 0.0 => Ok(log(a, f64::log2, mode)),
        F::Pow => Ok(pow(a, b()?, mode)),
        F::Atan => Ok(inexact(a.rising(f64::atan), ATAN_ULPS, mode)),
        F::Asin if a.lo >= -1.0 && a.hi <= 1.0 => Ok(inexact(a.rising(f64::asin), ATAN_ULPS, mode)),
        F::Acos if a.lo >= -1.0 && a.hi <= 1.0 => {
            Ok(inexact(Num::new(a.hi.acos(), a.lo.acos()), ATAN_ULPS, mode))
        }
        F::Atan2 => Ok(atan2(a, b()?, mode)),
        F::Tan | F::Sinh | F::Cosh | F::Tanh | F::Asinh | F::Acosh | F::Atanh => {
            Ok(by_point(fun, a, mode))
        }
        F::Sqrt | F::InverseSqrt | F::Log | F::Log2 | F::Asin | F::Acos => Ok(Num::ANY),
        _ => Err(Fault::Unsupported("a built-in the evaluator does not know")),
    }
}

fn abs(a: Num) -> Num {
    match (a.lo >= 0.0, a.hi <= 0.0) {
        (true, _) => a,
        (_, true) => a.neg(),
        _ => Num::new(0.0, a.hi.max(-a.lo)),
    }
}

fn sign(x: f64) -> f64 {
    match x {
        x if x > 0.0 => 1.0,
        x if x < 0.0 => -1.0,
        _ => 0.0,
    }
}

/// `x - floor(x)`, which in f32 can round up to one for a tiny negative `x`.
fn fract(a: Num, mode: Mode) -> Num {
    if mode == Mode::Point && a.is_point() {
        let x = a.lo as f32;
        return Num::point(f64::from(x - x.floor()));
    }
    match a.lo.floor() == a.hi.floor() {
        true => Num::new(a.lo - a.lo.floor(), a.hi - a.hi.floor()),
        false => Num::new(0.0, 1.0),
    }
}

fn step(edge: Num, x: Num) -> Num {
    match (x.lo >= edge.hi, x.hi < edge.lo) {
        (true, _) => Num::point(1.0),
        (_, true) => Num::point(0.0),
        _ => Num::new(0.0, 1.0),
    }
}

/// `a + (b − a)·t` is linear in each argument, so its extremes sit on the corners.
fn mix(a: Num, b: Num, t: Num) -> Num {
    let mut corners = [0.0; 8];
    for (i, corner) in corners.iter_mut().enumerate() {
        let x = if i & 1 == 0 { a.lo } else { a.hi };
        let y = if i & 2 == 0 { b.lo } else { b.hi };
        let s = if i & 4 == 0 { t.lo } else { t.hi };
        *corner = x * (1.0 - s) + y * s;
    }
    Num::of(&corners)
}

fn smoothstep(low: Num, high: Num, x: Num) -> Num {
    if !(low.is_point() && high.is_point() && high.lo > low.lo) {
        return Num::new(0.0, 1.0);
    }
    let curve = |v: f64| {
        let t = ((v - low.lo) / (high.lo - low.lo)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    x.rising(curve)
}

fn trig(fun: F, a: Num, mode: Mode) -> Num {
    let f = match fun {
        F::Sin => f64::sin,
        _ => f64::cos,
    };
    if mode == Mode::Point && a.is_point() {
        return Num::point(f64::from(f(a.lo) as f32));
    }
    // Outside [−π, π] WGSL does not say how close the GPU gets.
    if a.lo < -PI || a.hi > PI {
        return Num::new(-1.0, 1.0).pad(TRIG_ERROR);
    }
    let mut n = Num::new(f(a.lo), f(a.hi));
    let peaks: &[(f64, f64)] = match fun {
        F::Sin => &[(FRAC_PI_2, 1.0), (-FRAC_PI_2, -1.0)],
        _ => &[(0.0, 1.0)],
    };
    for &(at, value) in peaks {
        if a.lo <= at && at <= a.hi {
            n = n.hull(Num::point(value));
        }
    }
    n.pad(TRIG_ERROR)
}

/// A rising function the GPU computes within `ulps`.
fn inexact(n: Num, ulps: u32, mode: Mode) -> Num {
    fit(Kind::F32, n, ulps, mode)
}

/// WGSL allows `exp` 3 + 2·|x| ULP.
fn exp(a: Num, f: fn(f64) -> f64, mode: Mode) -> Num {
    let reach = a.lo.abs().max(a.hi.abs()).min(1e6);
    inexact(a.rising(f), 3 + 2 * reach.ceil() as u32, mode)
}

fn log(a: Num, f: fn(f64) -> f64, mode: Mode) -> Num {
    let n = a.rising(f);
    match mode {
        Mode::Point => fit(Kind::F32, n, 0, mode),
        Mode::Range => inexact(n.pad(LOG_ERROR), 3, mode),
    }
}

/// `exp2(y · log2(x))`, as WGSL defines it and bounds its error.
fn pow(x: Num, y: Num, mode: Mode) -> Num {
    if mode == Mode::Point && x.is_point() && y.is_point() {
        return Num::point(f64::from((x.lo as f32).powf(y.lo as f32)));
    }
    if !(x.lo > 0.0) {
        return Num::ANY;
    }
    let power = log(x, f64::log2, mode).mul(y);
    exp(power, f64::exp2, mode)
}

fn atan2(y: Num, x: Num, mode: Mode) -> Num {
    match mode == Mode::Point && y.is_point() && x.is_point() {
        true => Num::point(f64::from((y.lo as f32).atan2(x.lo as f32))),
        false => inexact(Num::new(-PI, PI), ATAN_ULPS, mode),
    }
}

/// The functions only a point can answer: anywhere else they are unbounded.
fn by_point(fun: F, a: Num, mode: Mode) -> Num {
    if !(mode == Mode::Point && a.is_point()) {
        return Num::ANY;
    }
    let x = a.lo as f32;
    let value = match fun {
        F::Tan => x.tan(),
        F::Sinh => x.sinh(),
        F::Cosh => x.cosh(),
        F::Tanh => x.tanh(),
        F::Asinh => x.asinh(),
        F::Acosh => x.acosh(),
        _ => x.atanh(),
    };
    match value.is_nan() {
        true => Num::ANY,
        false => Num::point(f64::from(value)),
    }
}

fn dot(a: Lanes, b: Lanes, same: bool, mode: Mode) -> Num {
    let len = a.len.max(b.len);
    let sum = (0..usize::from(len))
        .map(|i| match same {
            true => a.lane(i).sqr(),
            false => a.lane(i).mul(b.lane(i)),
        })
        .fold(Num::point(0.0), Num::add);
    fit(a.kind, sum, 2 * u32::from(len), mode)
}

fn minus(a: Lanes, b: Lanes, mode: Mode) -> Lanes {
    let mut out = a;
    for i in 0..usize::from(a.len) {
        out.n[i] = fit(a.kind, a.lane(i).sub(b.lane(i)), 1, mode);
    }
    out
}

fn length(a: Lanes, mode: Mode) -> Num {
    let squares = a
        .lanes()
        .iter()
        .fold(Num::point(0.0), |s, n| s.add(n.sqr()));
    fit(
        a.kind,
        squares.rising(f64::sqrt),
        4 + 2 * u32::from(a.len),
        mode,
    )
}

fn normalize(a: Lanes, mode: Mode) -> Lanes {
    let size = length(a, mode);
    // The zero vector normalises to NaN.
    if !(size.lo > 0.0) {
        return a.map(a.kind, |_| Num::ANY);
    }
    a.map(a.kind, |n| {
        let unit = n.div(size);
        fit(
            a.kind,
            unit.max(Num::point(-1.0)).min(Num::point(1.0)),
            4,
            mode,
        )
    })
}

fn cross(a: Lanes, b: Lanes, mode: Mode) -> Lanes {
    let term = |i: usize, j: usize| a.lane(i).mul(b.lane(j)).sub(a.lane(j).mul(b.lane(i)));
    let mut out = Lanes::splat(a.kind, 3, Num::ANY);
    out.n[0] = fit(a.kind, term(1, 2), 2, mode);
    out.n[1] = fit(a.kind, term(2, 0), 2, mode);
    out.n[2] = fit(a.kind, term(0, 1), 2, mode);
    out
}
