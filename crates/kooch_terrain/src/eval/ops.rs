//! Operators and conversions over lanes, with WGSL's rules for integers and the GPU's rounding
//! allowance for f32.

use naga::{BinaryOperator as Bin, ScalarKind, UnaryOperator as Un};

use super::Fault;
use super::num::{Mode, Num, to_f32};
use super::value::{Kind, Lanes};

/// Rounds a float result as its kind holds it.
pub fn fit(kind: Kind, n: Num, ulps: u32, mode: Mode) -> Num {
    match kind {
        Kind::F32 => to_f32(n, ulps, mode),
        _ => n,
    }
}

/// `a op b`, lane by lane; a scalar side stands for every lane. `same` says both sides are one
/// expression, which `x * x` and `x - x` need to stay tight.
pub fn binary(op: Bin, a: Lanes, b: Lanes, same: bool, mode: Mode) -> Result<Lanes, Fault> {
    let len = a.len.max(b.len);
    let kind = match op {
        Bin::Equal
        | Bin::NotEqual
        | Bin::Less
        | Bin::LessEqual
        | Bin::Greater
        | Bin::GreaterEqual => Kind::Bool,
        _ if a.len >= b.len => a.kind,
        _ => b.kind,
    };
    let mut out = Lanes::splat(kind, len, Num::ANY);
    for i in 0..usize::from(len) {
        out.n[i] = lane(op, a.kind, a.lane(i), b.lane(i), same, mode)?;
    }
    Ok(out)
}

fn lane(op: Bin, kind: Kind, x: Num, y: Num, same: bool, mode: Mode) -> Result<Num, Fault> {
    if let Some(known) = compare(op, x, y) {
        return Ok(known);
    }
    match kind {
        Kind::F32 | Kind::Float => float(op, kind, x, y, same, mode),
        Kind::I32 | Kind::U32 | Kind::Int => Ok(integer(op, kind, x, y)),
        Kind::Bool => logic(op, x, y),
    }
}

/// The comparisons, which every kind answers the same way.
fn compare(op: Bin, x: Num, y: Num) -> Option<Num> {
    let decide = |yes: bool, no: bool| match (yes, no) {
        (true, _) => Num::truth(true),
        (_, true) => Num::truth(false),
        _ => Num::EITHER,
    };
    let equal = || decide(x.is_point() && x == y, x.hi < y.lo || y.hi < x.lo);
    Some(match op {
        Bin::Less => decide(x.hi < y.lo, x.lo >= y.hi),
        Bin::LessEqual => decide(x.hi <= y.lo, x.lo > y.hi),
        Bin::Greater => decide(x.lo > y.hi, x.hi <= y.lo),
        Bin::GreaterEqual => decide(x.lo >= y.hi, x.hi < y.lo),
        Bin::Equal => equal(),
        Bin::NotEqual => not(equal()),
        _ => return None,
    })
}

fn not(b: Num) -> Num {
    Num::new(1.0 - b.hi, 1.0 - b.lo)
}

fn float(op: Bin, kind: Kind, x: Num, y: Num, same: bool, mode: Mode) -> Result<Num, Fault> {
    let (n, ulps) = match op {
        Bin::Add => (x.add(y), 1),
        Bin::Subtract if same => (Num::point(0.0), 0),
        Bin::Subtract => (x.sub(y), 1),
        Bin::Multiply if same => (x.sqr(), 1),
        Bin::Multiply => (x.mul(y), 1),
        // WGSL allows 2.5 ULP for a division.
        Bin::Divide => (x.div(y), 3),
        Bin::Modulo => (modulo(x, y), 3),
        _ => return Err(Fault::Unsupported("a bit operation on a float")),
    };
    Ok(fit(kind, n, ulps, mode))
}

/// `x - y · trunc(x / y)`: the result takes `x`'s sign and stays inside `|y|`.
fn modulo(x: Num, y: Num) -> Num {
    if x.is_point() && y.is_point() && y.lo != 0.0 {
        return Num::point(x.lo % y.lo);
    }
    let reach = y.lo.abs().max(y.hi.abs());
    let lo = if x.lo < 0.0 { x.lo.max(-reach) } else { 0.0 };
    let hi = if x.hi > 0.0 { x.hi.min(reach) } else { 0.0 };
    Num::new(lo, hi)
}

fn integer(op: Bin, kind: Kind, x: Num, y: Num) -> Num {
    if x.is_point() && y.is_point() {
        return exact(op, kind, x.lo as i64, y.lo as i64);
    }
    let n = match op {
        Bin::Add => x.add(y),
        Bin::Subtract => x.sub(y),
        Bin::Multiply => x.mul(y),
        Bin::Divide if y.lo > 0.0 || y.hi < 0.0 => x.div(y).rising(f64::trunc),
        Bin::Modulo if x.lo >= 0.0 && y.lo > 0.0 => Num::new(0.0, x.hi.min(y.hi - 1.0)),
        Bin::And if kind == Kind::U32 => Num::new(0.0, x.hi.min(y.hi)),
        // Shifting a u32 right is monotonic: the noise hashes keep their top bits this way (#1930).
        Bin::ShiftRight if kind == Kind::U32 && y.is_point() => {
            let s = y.lo as u32 & 31;
            Num::new(((x.lo as u64) >> s) as f64, ((x.hi as u64) >> s) as f64)
        }
        _ => return kind.full(),
    };
    let full = kind.full();
    match n.lo >= full.lo && n.hi <= full.hi {
        true => n,
        false => full,
    }
}

/// Two known integers, wrapping as the GPU does.
fn exact(op: Bin, kind: Kind, x: i64, y: i64) -> Num {
    let value = match kind {
        Kind::I32 => i64::from(exact_i32(op, x as i32, y as i32)),
        Kind::U32 => i64::from(exact_u32(op, x as u32, y as u32)),
        _ => match op {
            Bin::Add => x.wrapping_add(y),
            Bin::Subtract => x.wrapping_sub(y),
            Bin::Multiply => x.wrapping_mul(y),
            Bin::Divide if y != 0 => x.wrapping_div(y),
            _ => return kind.full(),
        },
    };
    Num::point(value as f64)
}

fn exact_i32(op: Bin, x: i32, y: i32) -> i32 {
    // WGSL: a zero divisor, or MIN / -1, gives back `x`; the remainder is then zero.
    let unsafe_div = y == 0 || (x == i32::MIN && y == -1);
    match op {
        Bin::Add => x.wrapping_add(y),
        Bin::Subtract => x.wrapping_sub(y),
        Bin::Multiply => x.wrapping_mul(y),
        Bin::Divide if unsafe_div => x,
        Bin::Divide => x / y,
        Bin::Modulo if unsafe_div => 0,
        Bin::Modulo => x % y,
        Bin::And => x & y,
        Bin::InclusiveOr => x | y,
        Bin::ExclusiveOr => x ^ y,
        Bin::ShiftLeft => x.wrapping_shl(y as u32 & 31),
        Bin::ShiftRight => x.wrapping_shr(y as u32 & 31),
        _ => x,
    }
}

fn exact_u32(op: Bin, x: u32, y: u32) -> u32 {
    match op {
        Bin::Add => x.wrapping_add(y),
        Bin::Subtract => x.wrapping_sub(y),
        Bin::Multiply => x.wrapping_mul(y),
        Bin::Divide if y == 0 => x,
        Bin::Divide => x / y,
        Bin::Modulo if y == 0 => 0,
        Bin::Modulo => x % y,
        Bin::And => x & y,
        Bin::InclusiveOr => x | y,
        Bin::ExclusiveOr => x ^ y,
        Bin::ShiftLeft => x.wrapping_shl(y & 31),
        Bin::ShiftRight => x.wrapping_shr(y & 31),
        _ => x,
    }
}

fn logic(op: Bin, x: Num, y: Num) -> Result<Num, Fault> {
    let (a, b) = (x.known(), y.known());
    Ok(match op {
        Bin::LogicalAnd | Bin::And => match (a, b) {
            (Some(false), _) | (_, Some(false)) => Num::truth(false),
            (Some(true), Some(true)) => Num::truth(true),
            _ => Num::EITHER,
        },
        Bin::LogicalOr | Bin::InclusiveOr => match (a, b) {
            (Some(true), _) | (_, Some(true)) => Num::truth(true),
            (Some(false), Some(false)) => Num::truth(false),
            _ => Num::EITHER,
        },
        Bin::ExclusiveOr => match (a, b) {
            (Some(p), Some(q)) => Num::truth(p != q),
            _ => Num::EITHER,
        },
        _ => return Err(Fault::Unsupported("arithmetic on a boolean")),
    })
}

pub fn unary(op: Un, a: Lanes) -> Lanes {
    match op {
        Un::Negate => a.map(a.kind, |n| match (a.kind, n.is_point()) {
            (Kind::I32, true) => Num::point(f64::from((n.lo as i32).wrapping_neg())),
            (Kind::I32, false) if n.lo <= f64::from(i32::MIN) => Kind::I32.full(),
            _ => n.neg(),
        }),
        Un::LogicalNot => a.map(Kind::Bool, not),
        Un::BitwiseNot => a.map(a.kind, |n| match (a.kind, n.is_point()) {
            (Kind::I32, true) => Num::point(f64::from(!(n.lo as i32))),
            (Kind::U32, true) => Num::point(f64::from(!(n.lo as u32))),
            (Kind::Bool, _) => not(n),
            _ => a.kind.full(),
        }),
    }
}

/// `select(reject, accept, condition)`, lane by lane; an undecided lane can be either.
pub fn select(condition: Lanes, accept: Lanes, reject: Lanes) -> Lanes {
    let mut out = accept;
    for i in 0..usize::from(out.len) {
        out.n[i] = match condition.lane(i).known() {
            Some(true) => accept.lane(i),
            Some(false) => reject.lane(i),
            None => accept.lane(i).hull(reject.lane(i)),
        };
    }
    out
}

/// `T(x)` when `convert` is a width, `bitcast<T>(x)` when it is not.
pub fn convert(a: Lanes, to: ScalarKind, width: Option<u8>, mode: Mode) -> Result<Lanes, Fault> {
    let target = Kind::of(naga::Scalar {
        kind: to,
        width: width.unwrap_or(4),
    })?;
    let from = a.kind;
    Ok(a.map(target, |n| match width {
        Some(_) => numeric(from, target, n, mode),
        None => bitcast(from, target, n),
    }))
}

fn numeric(from: Kind, to: Kind, n: Num, mode: Mode) -> Num {
    let full = to.full();
    match (from, to) {
        (_, Kind::Bool) => not(compare(Bin::Equal, n, Num::point(0.0)).unwrap_or(Num::EITHER)),
        (Kind::Bool, _) => n,
        // WGSL truncates toward zero and saturates at the target's ends.
        (f, Kind::I32 | Kind::U32) if f.is_float() => {
            n.rising(|x| x.trunc().clamp(full.lo, full.hi))
        }
        (_, Kind::F32) => fit(Kind::F32, n, 0, mode),
        (Kind::I32 | Kind::U32 | Kind::Int, Kind::I32 | Kind::U32) => match n.is_point() {
            true if to == Kind::I32 => Num::point(f64::from(n.lo as i64 as u32 as i32)),
            true => Num::point(f64::from(n.lo as i64 as i32 as u32)),
            false if n.lo >= 0.0 && n.hi <= f64::from(i32::MAX) => n,
            false => full,
        },
        _ => n,
    }
}

fn bitcast(from: Kind, to: Kind, n: Num) -> Num {
    if !n.is_point() || from == to {
        return match from == to {
            true => n,
            false => to.full(),
        };
    }
    let bits = match from {
        Kind::F32 => (n.lo as f32).to_bits(),
        Kind::I32 => n.lo as i32 as u32,
        _ => n.lo as u32,
    };
    match to {
        Kind::F32 => {
            let f = f32::from_bits(bits);
            match f.is_nan() {
                true => Num::ANY,
                false => Num::point(f64::from(f)),
            }
        }
        Kind::I32 => Num::point(f64::from(bits as i32)),
        _ => Num::point(f64::from(bits)),
    }
}
