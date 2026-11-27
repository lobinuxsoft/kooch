//! Walks naga's IR of one function: statements in order, each `Emit` evaluated where it stands so
//! a load reads what was stored before it. A branch nobody can decide runs both ways and keeps
//! the hull of what each left in the local variables.

use naga::{Block, Expression as E, Function, Handle, Literal, Module, Statement as S};

use super::num::{Mode, Num};
use super::value::{ANY_INDEX, Kind, Lanes, Place, Value, zero};
use super::{Fault, math, ops};

/// Statements one evaluation may run before it is given up as unbounded.
const BUDGET: u32 = 4_000_000;

pub struct Machine<'m> {
    pub module: &'m Module,
    /// The module's constants, evaluated once.
    pub constants: &'m [Option<Value>],
    pub mode: Mode,
    steps: u32,
}

struct Frame<'f> {
    func: &'f Function,
    args: Vec<Value>,
    locals: Vec<Value>,
    exprs: Vec<Option<Value>>,
}

enum Flow {
    Next,
    Break,
    Continue,
    Return(Option<Value>),
}

impl<'m> Machine<'m> {
    pub fn new(module: &'m Module, constants: &'m [Option<Value>], mode: Mode) -> Self {
        Self {
            module,
            constants,
            mode,
            steps: 0,
        }
    }

    pub fn call(&mut self, function: Handle<Function>, args: Vec<Value>) -> Result<Value, Fault> {
        let func = &self.module.functions[function];
        let mut frame = Frame {
            func,
            args,
            locals: Vec::with_capacity(func.local_variables.len()),
            exprs: vec![None; func.expressions.len()],
        };
        for (_, var) in func.local_variables.iter() {
            let value = match var.init {
                Some(init) => self.expr(&mut frame, init)?,
                None => zero(self.module, var.ty)?,
            };
            frame.locals.push(value);
        }
        match self.block(&mut frame, &func.body)? {
            Flow::Return(Some(value)) => Ok(value),
            _ => Err(Fault::Unsupported("a function that returns nothing")),
        }
    }

    fn block(&mut self, frame: &mut Frame, block: &Block) -> Result<Flow, Fault> {
        for statement in block.iter() {
            self.steps += 1;
            if self.steps > BUDGET {
                return Err(Fault::Budget);
            }
            let flow = self.statement(frame, statement)?;
            if !matches!(flow, Flow::Next) {
                return Ok(flow);
            }
        }
        Ok(Flow::Next)
    }

    fn statement(&mut self, frame: &mut Frame, statement: &S) -> Result<Flow, Fault> {
        match statement {
            S::Emit(range) => {
                for h in range.clone() {
                    let value = self.eval(frame, h)?;
                    frame.exprs[h.index()] = Some(value);
                }
            }
            S::Block(block) => return self.block(frame, block),
            S::If {
                condition,
                accept,
                reject,
            } => {
                let decided = self.expr(frame, *condition)?.num()?.known();
                return match decided {
                    Some(true) => self.block(frame, accept),
                    Some(false) => self.block(frame, reject),
                    None => self.both(frame, accept, reject),
                };
            }
            S::Switch { selector, cases } => {
                let selector = self.expr(frame, *selector)?.num()?;
                if !selector.is_point() {
                    return Err(Fault::Unbounded);
                }
                return self.switch(frame, selector.lo, cases);
            }
            S::Loop {
                body,
                continuing,
                break_if,
            } => return self.looped(frame, body, continuing, *break_if),
            S::Break => return Ok(Flow::Break),
            S::Continue => return Ok(Flow::Continue),
            S::Return { value } => {
                let value = value.map(|v| self.expr(frame, v)).transpose()?;
                return Ok(Flow::Return(value));
            }
            S::Store { pointer, value } => {
                let Value::Ptr(place) = self.expr(frame, *pointer)? else {
                    return Err(Fault::Unsupported("a store through a value"));
                };
                let value = self.expr(frame, *value)?;
                let local = frame
                    .locals
                    .get_mut(place.var as usize)
                    .ok_or(Fault::Unsupported("a store to an unknown variable"))?;
                local.store(&place.path[..usize::from(place.depth)], value)?;
            }
            S::Call {
                function,
                arguments,
                result,
            } => {
                let args = arguments
                    .iter()
                    .map(|&a| self.expr(frame, a))
                    .collect::<Result<_, _>>()?;
                let value = self.call(*function, args)?;
                if let Some(result) = result {
                    frame.exprs[result.index()] = Some(value);
                }
            }
            _ => return Err(Fault::Unsupported("a statement a generator has no use for")),
        }
        Ok(Flow::Next)
    }

    /// Both arms of an undecided `if`, each from the same variables, joined after.
    fn both(&mut self, frame: &mut Frame, accept: &Block, reject: &Block) -> Result<Flow, Fault> {
        let before = frame.locals.clone();
        let accepted = self.block(frame, accept)?;
        let after_accept = std::mem::replace(&mut frame.locals, before);
        let rejected = self.block(frame, reject)?;
        for (local, other) in frame.locals.iter_mut().zip(&after_accept) {
            *local = local.hull(other);
        }
        Ok(match (accepted, rejected) {
            (Flow::Next, Flow::Next) => Flow::Next,
            (Flow::Break, Flow::Break) => Flow::Break,
            (Flow::Continue, Flow::Continue) => Flow::Continue,
            (Flow::Return(Some(a)), Flow::Return(Some(b))) => Flow::Return(Some(a.hull(&b))),
            _ => return Err(Fault::Unbounded),
        })
    }

    fn switch(
        &mut self,
        frame: &mut Frame,
        at: f64,
        cases: &[naga::SwitchCase],
    ) -> Result<Flow, Fault> {
        let hit = cases.iter().position(|case| match case.value {
            naga::SwitchValue::I32(v) => f64::from(v) == at,
            naga::SwitchValue::U32(v) => f64::from(v) == at,
            naga::SwitchValue::Default => false,
        });
        let start = hit
            .or_else(|| {
                cases
                    .iter()
                    .position(|c| c.value == naga::SwitchValue::Default)
            })
            .ok_or(Fault::Unsupported("a switch with no case to take"))?;
        for case in &cases[start..] {
            match self.block(frame, &case.body)? {
                Flow::Next if case.fall_through => continue,
                Flow::Next | Flow::Break => return Ok(Flow::Next),
                flow => return Ok(flow),
            }
        }
        Ok(Flow::Next)
    }

    fn looped(
        &mut self,
        frame: &mut Frame,
        body: &Block,
        continuing: &Block,
        break_if: Option<Handle<E>>,
    ) -> Result<Flow, Fault> {
        loop {
            match self.block(frame, body)? {
                Flow::Break => return Ok(Flow::Next),
                Flow::Return(value) => return Ok(Flow::Return(value)),
                Flow::Next | Flow::Continue => {}
            }
            if !matches!(self.block(frame, continuing)?, Flow::Next) {
                return Err(Fault::Unsupported(
                    "control flow in a loop's continuing block",
                ));
            }
            if let Some(condition) = break_if {
                match self.expr(frame, condition)?.num()?.known() {
                    Some(true) => return Ok(Flow::Next),
                    Some(false) => {}
                    None => return Err(Fault::Unbounded),
                }
            }
        }
    }

    /// An expression's value: emitted ones were evaluated where they stood, the rest are
    /// constant and evaluated when first read.
    fn expr(&mut self, frame: &mut Frame, h: Handle<E>) -> Result<Value, Fault> {
        if let Some(Some(value)) = frame.exprs.get(h.index()) {
            return Ok(value.clone());
        }
        let value = self.eval(frame, h)?;
        frame.exprs[h.index()] = Some(value.clone());
        Ok(value)
    }

    fn lanes(&mut self, frame: &mut Frame, h: Handle<E>) -> Result<Lanes, Fault> {
        self.expr(frame, h)?.lanes()
    }

    fn eval(&mut self, frame: &mut Frame, h: Handle<E>) -> Result<Value, Fault> {
        let mode = self.mode;
        Ok(match &frame.func.expressions[h] {
            E::Literal(literal) => Value::Lanes(literal_lanes(*literal)?),
            E::Constant(c) => self
                .constants
                .get(c.index())
                .cloned()
                .flatten()
                .ok_or(Fault::Unsupported("a constant it could not evaluate"))?,
            E::ZeroValue(ty) => zero(self.module, *ty)?,
            E::Compose { ty, components } => {
                let parts = components
                    .iter()
                    .map(|&c| self.expr(frame, c))
                    .collect::<Result<Vec<_>, _>>()?;
                compose(self.module, *ty, parts)?
            }
            E::Access { base, index } => {
                let index = self.expr(frame, *index)?.num()?;
                let index = match index.is_point() {
                    true => index.lo as u32,
                    false => ANY_INDEX,
                };
                access(self.expr(frame, *base)?, index)?
            }
            E::AccessIndex { base, index } => access(self.expr(frame, *base)?, *index)?,
            E::Splat { size, value } => {
                let lane = self.lanes(frame, *value)?;
                Value::Lanes(Lanes::splat(lane.kind, *size as u8, lane.n[0]))
            }
            E::Swizzle {
                size,
                vector,
                pattern,
            } => {
                let v = self.lanes(frame, *vector)?;
                let mut out = Lanes::splat(v.kind, *size as u8, Num::ANY);
                for (lane, from) in out.n.iter_mut().zip(pattern) {
                    *lane = v.lane(*from as usize);
                }
                Value::Lanes(out)
            }
            E::FunctionArgument(i) => frame
                .args
                .get(*i as usize)
                .cloned()
                .ok_or(Fault::Unsupported("an argument past the end"))?,
            E::LocalVariable(var) => Value::Ptr(Place {
                var: var.index() as u32,
                path: [0; 4],
                depth: 0,
            }),
            E::Load { pointer } => {
                let Value::Ptr(place) = self.expr(frame, *pointer)? else {
                    return Err(Fault::Unsupported("a load through a value"));
                };
                frame
                    .locals
                    .get(place.var as usize)
                    .ok_or(Fault::Unsupported("a load of an unknown variable"))?
                    .load(&place.path[..usize::from(place.depth)])?
            }
            E::Unary { op, expr } => Value::Lanes(ops::unary(*op, self.lanes(frame, *expr)?)),
            E::Binary { op, left, right } => {
                let a = self.lanes(frame, *left)?;
                let b = self.lanes(frame, *right)?;
                Value::Lanes(ops::binary(*op, a, b, left == right, mode)?)
            }
            E::Select {
                condition,
                accept,
                reject,
            } => {
                let c = self.lanes(frame, *condition)?;
                let a = self.lanes(frame, *accept)?;
                let r = self.lanes(frame, *reject)?;
                Value::Lanes(ops::select(c, a, r))
            }
            E::Relational { fun, argument } => {
                let a = self.lanes(frame, *argument)?;
                Value::Lanes(relational(*fun, a))
            }
            E::Math {
                fun,
                arg,
                arg1,
                arg2,
                arg3,
            } => {
                let mut args = vec![self.lanes(frame, *arg)?];
                for extra in [arg1, arg2, arg3].into_iter().flatten() {
                    args.push(self.lanes(frame, *extra)?);
                }
                Value::Lanes(math::call(*fun, &args, Some(*arg) == *arg1, mode)?)
            }
            E::As {
                expr,
                kind,
                convert,
            } => Value::Lanes(ops::convert(
                self.lanes(frame, *expr)?,
                *kind,
                *convert,
                mode,
            )?),
            E::CallResult(_) => {
                return Err(Fault::Unsupported("a call result read before the call"));
            }
            _ => {
                return Err(Fault::Unsupported(
                    "an expression a generator has no use for",
                ));
            }
        })
    }
}

pub fn literal_lanes(literal: Literal) -> Result<Lanes, Fault> {
    let (kind, value) = match literal {
        Literal::F32(v) => (Kind::F32, f64::from(v)),
        Literal::F64(v) | Literal::AbstractFloat(v) => (Kind::Float, v),
        Literal::I32(v) => (Kind::I32, f64::from(v)),
        Literal::U32(v) => (Kind::U32, f64::from(v)),
        Literal::AbstractInt(v) => (Kind::Int, v as f64),
        Literal::Bool(v) => (Kind::Bool, f64::from(u8::from(v))),
        _ => return Err(Fault::Unsupported("a 16- or 64-bit literal")),
    };
    Ok(Lanes::scalar(kind, Num::point(value)))
}

/// A vector from scalars and smaller vectors, or a struct, array or matrix from its parts.
pub fn compose(module: &Module, ty: Handle<naga::Type>, parts: Vec<Value>) -> Result<Value, Fault> {
    let naga::TypeInner::Vector { size, scalar } = module.types[ty].inner else {
        return Ok(Value::Parts(parts));
    };
    let mut out = Lanes::splat(Kind::of(scalar)?, size as u8, Num::ANY);
    let mut at = 0;
    for part in &parts {
        for &n in part.lanes()?.lanes() {
            if at < 4 {
                out.n[at] = n;
            }
            at += 1;
        }
    }
    Ok(Value::Lanes(out))
}

/// One part of `base`: of a pointer, a pointer one level in.
fn access(base: Value, index: u32) -> Result<Value, Fault> {
    match base {
        Value::Ptr(mut place) => {
            let depth = usize::from(place.depth);
            if depth >= place.path.len() {
                return Err(Fault::Unsupported("a pointer nested too deep"));
            }
            place.path[depth] = index;
            place.depth += 1;
            Ok(Value::Ptr(place))
        }
        value => value.part(index),
    }
}

fn relational(fun: naga::RelationalFunction, a: Lanes) -> Lanes {
    use naga::RelationalFunction as R;
    let truths = a.lanes().iter().map(|n| n.known());
    let decided = match fun {
        R::All => match truths.clone().all(|t| t == Some(true)) {
            true => Some(true),
            false => truths.clone().any(|t| t == Some(false)).then_some(false),
        },
        R::Any => match truths.clone().any(|t| t == Some(true)) {
            true => Some(true),
            false => truths.clone().all(|t| t == Some(false)).then_some(false),
        },
        // NaN and infinity never live in an interval as values: undecided unless finite points.
        R::IsNan | R::IsInf => {
            return a.map(Kind::Bool, |n| match n.lo.is_finite() && n.hi.is_finite() {
                true => Num::truth(false),
                false => Num::EITHER,
            });
        }
    };
    Lanes::scalar(Kind::Bool, decided.map_or(Num::EITHER, Num::truth))
}
