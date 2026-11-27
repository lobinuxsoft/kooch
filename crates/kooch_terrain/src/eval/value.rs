//! What the evaluator holds for one expression: scalars and vectors as lanes of intervals,
//! structs and arrays as parts, and pointers to a local variable.

use naga::{Handle, Module, Scalar, ScalarKind, Type, TypeInner};

use super::Fault;
use super::num::Num;

/// A lane's type. `Float` and `Int` are WGSL's abstract numbers, carried unrounded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    F32,
    Float,
    I32,
    U32,
    Int,
    Bool,
}

impl Kind {
    pub fn of(scalar: Scalar) -> Result<Kind, Fault> {
        match (scalar.kind, scalar.width) {
            (ScalarKind::Float, 4) => Ok(Kind::F32),
            (ScalarKind::Float, 8) | (ScalarKind::AbstractFloat, _) => Ok(Kind::Float),
            (ScalarKind::Sint, 4) => Ok(Kind::I32),
            (ScalarKind::Uint, 4) => Ok(Kind::U32),
            (ScalarKind::AbstractInt, _) => Ok(Kind::Int),
            (ScalarKind::Bool, _) => Ok(Kind::Bool),
            _ => Err(Fault::Unsupported("a 16- or 64-bit scalar")),
        }
    }

    pub fn is_float(self) -> bool {
        matches!(self, Kind::F32 | Kind::Float)
    }

    /// Every value the type can hold, as an interval.
    pub fn full(self) -> Num {
        match self {
            Kind::I32 => Num::new(f64::from(i32::MIN), f64::from(i32::MAX)),
            Kind::U32 => Num::new(0.0, f64::from(u32::MAX)),
            Kind::Bool => Num::EITHER,
            _ => Num::ANY,
        }
    }
}

/// A scalar or a vector of up to four lanes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lanes {
    pub kind: Kind,
    pub len: u8,
    pub n: [Num; 4],
}

impl Lanes {
    pub fn scalar(kind: Kind, n: Num) -> Lanes {
        Lanes::splat(kind, 1, n)
    }

    pub fn splat(kind: Kind, len: u8, n: Num) -> Lanes {
        Lanes {
            kind,
            len,
            n: [n; 4],
        }
    }

    pub fn lanes(&self) -> &[Num] {
        &self.n[..usize::from(self.len)]
    }

    /// Lane `i`, the only lane of a scalar standing for every lane of a vector.
    pub fn lane(&self, i: usize) -> Num {
        match self.len {
            1 => self.n[0],
            _ => self.n[i.min(3)],
        }
    }

    pub fn map(&self, kind: Kind, f: impl Fn(Num) -> Num) -> Lanes {
        let mut out = *self;
        out.kind = kind;
        for n in &mut out.n[..usize::from(self.len)] {
            *n = f(*n);
        }
        out
    }
}

/// Where a pointer points: a local variable and the indices into it. `ANY_INDEX` stands for an
/// index the analysis could not pin down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub var: u32,
    pub path: [u32; 4],
    pub depth: u8,
}

pub const ANY_INDEX: u32 = u32::MAX;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Lanes(Lanes),
    Parts(Vec<Value>),
    Ptr(Place),
}

impl Value {
    pub fn lanes(&self) -> Result<Lanes, Fault> {
        match self {
            Value::Lanes(lanes) => Ok(*lanes),
            _ => Err(Fault::Unsupported("a struct or matrix where a number goes")),
        }
    }

    /// A single number's interval.
    pub fn num(&self) -> Result<Num, Fault> {
        Ok(self.lanes()?.n[0])
    }

    /// Either value: what a branch nobody can decide leaves behind.
    pub fn hull(&self, other: &Value) -> Value {
        match (self, other) {
            (Value::Lanes(a), Value::Lanes(b)) => {
                let mut out = *a;
                for (n, m) in out.n.iter_mut().zip(b.n) {
                    *n = n.hull(m);
                }
                Value::Lanes(out)
            }
            (Value::Parts(a), Value::Parts(b)) if a.len() == b.len() => {
                Value::Parts(a.iter().zip(b).map(|(x, y)| x.hull(y)).collect())
            }
            _ => self.clone(),
        }
    }

    /// The part at `index`, or every part's hull when the index is not known.
    pub fn part(&self, index: u32) -> Result<Value, Fault> {
        match self {
            Value::Parts(parts) if index == ANY_INDEX => parts
                .iter()
                .cloned()
                .reduce(|a, b| a.hull(&b))
                .ok_or(Fault::Unsupported("an empty array")),
            Value::Parts(parts) => parts
                .get(index as usize)
                .cloned()
                .ok_or(Fault::Unsupported("an index past the end")),
            Value::Lanes(lanes) if index == ANY_INDEX => {
                let all = lanes.lanes().iter().copied().reduce(Num::hull);
                Ok(Value::Lanes(Lanes::scalar(
                    lanes.kind,
                    all.unwrap_or(Num::ANY),
                )))
            }
            Value::Lanes(lanes) if index < u32::from(lanes.len) => Ok(Value::Lanes(Lanes::scalar(
                lanes.kind,
                lanes.n[index as usize],
            ))),
            _ => Err(Fault::Unsupported("an index past the end")),
        }
    }

    /// Writes `value` at `path`; an unknown index widens every part it could have been.
    pub fn store(&mut self, path: &[u32], value: Value) -> Result<(), Fault> {
        let Some((&first, rest)) = path.split_first() else {
            *self = value;
            return Ok(());
        };
        match self {
            Value::Parts(parts) if first == ANY_INDEX => {
                for part in parts {
                    let mut written = part.clone();
                    written.store(rest, value.clone())?;
                    *part = part.hull(&written);
                }
                Ok(())
            }
            Value::Parts(parts) => parts
                .get_mut(first as usize)
                .ok_or(Fault::Unsupported("an index past the end"))?
                .store(rest, value),
            Value::Lanes(lanes) if rest.is_empty() => {
                let n = value.num()?;
                match first {
                    ANY_INDEX => {
                        let len = usize::from(lanes.len);
                        for lane in &mut lanes.n[..len] {
                            *lane = lane.hull(n);
                        }
                    }
                    i if i < u32::from(lanes.len) => lanes.n[i as usize] = n,
                    _ => return Err(Fault::Unsupported("an index past the end")),
                }
                Ok(())
            }
            _ => Err(Fault::Unsupported(
                "a store through a pointer it cannot follow",
            )),
        }
    }

    /// What `path` reads.
    pub fn load(&self, path: &[u32]) -> Result<Value, Fault> {
        path.iter()
            .try_fold(self.clone(), |value, &index| value.part(index))
    }
}

/// A value of type `ty` with every lane zero.
pub fn zero(module: &Module, ty: Handle<Type>) -> Result<Value, Fault> {
    let lanes = |scalar, len| {
        Ok(Value::Lanes(Lanes::splat(
            Kind::of(scalar)?,
            len,
            Num::point(0.0),
        )))
    };
    match &module.types[ty].inner {
        TypeInner::Scalar(scalar) => lanes(*scalar, 1),
        TypeInner::Vector { size, scalar } => lanes(*scalar, *size as u8),
        TypeInner::Matrix {
            columns,
            rows,
            scalar,
        } => {
            let column = lanes(*scalar, *rows as u8)?;
            Ok(Value::Parts(vec![column; *columns as usize]))
        }
        TypeInner::Array {
            base,
            size: naga::ArraySize::Constant(count),
            ..
        } => Ok(Value::Parts(vec![
            zero(module, *base)?;
            count.get() as usize
        ])),
        TypeInner::Struct { members, .. } => members
            .iter()
            .map(|member| zero(module, member.ty))
            .collect::<Result<_, _>>()
            .map(Value::Parts),
        _ => Err(Fault::Unsupported("a type the evaluator has no value for")),
    }
}
