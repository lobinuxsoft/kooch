//! Closed intervals of reals: every number the evaluator carries. A point is an interval of width
//! zero, an unknown is the whole line. NaN never lives in one: whatever could produce it is the
//! whole line instead.

/// How the evaluator rounds: to the nearest f32 as the GPU computes one point, or outward with the
/// GPU's own error allowance so the answer holds whatever the device rounds to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Point,
    Range,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Num {
    pub lo: f64,
    pub hi: f64,
}

impl Num {
    pub const ANY: Num = Num {
        lo: f64::NEG_INFINITY,
        hi: f64::INFINITY,
    };
    /// A boolean nobody can decide.
    pub const EITHER: Num = Num { lo: 0.0, hi: 1.0 };

    pub fn point(x: f64) -> Num {
        Num::new(x, x)
    }

    /// The interval between `a` and `b`, in either order.
    pub fn new(a: f64, b: f64) -> Num {
        if a.is_nan() || b.is_nan() {
            return Num::ANY;
        }
        Num {
            lo: a.min(b),
            hi: a.max(b),
        }
    }

    /// The smallest interval holding every value.
    pub fn of(values: &[f64]) -> Num {
        if values.iter().any(|v| v.is_nan()) {
            return Num::ANY;
        }
        let lo = values.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Num { lo, hi }
    }

    pub fn truth(known: bool) -> Num {
        Num::point(f64::from(u8::from(known)))
    }

    pub fn is_point(self) -> bool {
        self.lo == self.hi
    }

    /// A boolean's value, when it is decided.
    pub fn known(self) -> Option<bool> {
        match (self.lo > 0.5, self.hi > 0.5) {
            (true, true) => Some(true),
            (false, false) => Some(false),
            _ => None,
        }
    }

    pub fn hull(self, other: Num) -> Num {
        Num {
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }

    pub fn add(self, o: Num) -> Num {
        Num::new(self.lo + o.lo, self.hi + o.hi)
    }

    pub fn sub(self, o: Num) -> Num {
        Num::new(self.lo - o.hi, self.hi - o.lo)
    }

    pub fn neg(self) -> Num {
        Num {
            lo: -self.hi,
            hi: -self.lo,
        }
    }

    pub fn mul(self, o: Num) -> Num {
        Num::of(&[
            times(self.lo, o.lo),
            times(self.lo, o.hi),
            times(self.hi, o.lo),
            times(self.hi, o.hi),
        ])
    }

    /// `x · x`, which cannot be negative however wide `x` is.
    pub fn sqr(self) -> Num {
        let (a, b) = (self.lo * self.lo, self.hi * self.hi);
        match (self.lo >= 0.0, self.hi <= 0.0) {
            (true, _) => Num::new(a, b),
            (_, true) => Num::new(b, a),
            _ => Num::new(0.0, a.max(b)),
        }
    }

    pub fn div(self, o: Num) -> Num {
        if !(o.lo > 0.0 || o.hi < 0.0) {
            return Num::ANY;
        }
        Num::of(&[
            self.lo / o.lo,
            self.lo / o.hi,
            self.hi / o.lo,
            self.hi / o.hi,
        ])
    }

    pub fn min(self, o: Num) -> Num {
        Num::new(self.lo.min(o.lo), self.hi.min(o.hi))
    }

    pub fn max(self, o: Num) -> Num {
        Num::new(self.lo.max(o.lo), self.hi.max(o.hi))
    }

    /// `f` over the interval, for an `f` that never decreases.
    pub fn rising(self, f: impl Fn(f64) -> f64) -> Num {
        Num::new(f(self.lo), f(self.hi))
    }

    /// Wider by `by` each way.
    pub fn pad(self, by: f64) -> Num {
        Num::new(self.lo - by, self.hi + by)
    }
}

/// A product whose zero factor wins over an infinite one, as the limit does.
fn times(a: f64, b: f64) -> f64 {
    match a == 0.0 || b == 0.0 {
        true => 0.0,
        false => a * b,
    }
}

/// `n` as the GPU holds an f32: the nearest one in [`Mode::Point`]; in [`Mode::Range`] rounded
/// outward and then `ulps` more each way, for an operation the device may round worse than exact.
pub fn to_f32(n: Num, ulps: u32, mode: Mode) -> Num {
    if mode == Mode::Point && n.is_point() {
        return Num::point(f64::from(n.lo as f32));
    }
    let lo = down(n.lo - ulp(n.lo) * f64::from(ulps));
    let hi = up(n.hi + ulp(n.hi) * f64::from(ulps));
    Num::new(f64::from(lo), f64::from(hi))
}

/// The gap above `|x|` between f32s: never smaller than the gap below, so a pad of it is enough.
fn ulp(x: f64) -> f64 {
    let f = (x as f32).abs();
    match f.is_finite() {
        true => f64::from(f.next_up()) - f64::from(f),
        false => 0.0,
    }
}

fn down(x: f64) -> f32 {
    let f = x as f32;
    match f64::from(f) > x {
        true => f.next_down(),
        false => f,
    }
}

fn up(x: f64) -> f32 {
    let f = x as f32;
    match f64::from(f) < x {
        true => f.next_up(),
        false => f,
    }
}
