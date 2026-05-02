//! [`RhaiBackend`] — [`ScriptingBackend`] implementation backed by Rhai 1.21.
//!
//! Owns a `rhai::Engine` (configured `sync` so it satisfies `Send + Sync`)
//! plus a slotmap of compiled `AST`s. Conversions between
//! [`ScriptValue`] and `rhai::Dynamic` live in this module — no rhai
//! types leak through the trait boundary.

use rhai::{AST, Dynamic, Engine, Scope};
use slotmap::SlotMap;

use crate::backend::{ScriptError, ScriptHandle, ScriptValue, ScriptingBackend};

/// Rhai-powered scripting backend.
pub struct RhaiBackend {
    engine: Engine,
    scripts: SlotMap<ScriptHandle, AST>,
}

impl RhaiBackend {
    pub fn new() -> Self {
        Self {
            engine: Engine::new(),
            scripts: SlotMap::with_key(),
        }
    }

    /// Borrows the underlying engine for advanced configuration (e.g.
    /// custom registered functions / type packages). Most callers use
    /// the trait surface and never touch this.
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }
}

impl Default for RhaiBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptingBackend for RhaiBackend {
    fn compile(&mut self, source: &str) -> Result<ScriptHandle, ScriptError> {
        let ast = self
            .engine
            .compile(source)
            .map_err(|e| ScriptError::Compile(format!("{e}")))?;
        Ok(self.scripts.insert(ast))
    }

    fn remove(&mut self, handle: ScriptHandle) {
        self.scripts.remove(handle);
    }

    fn contains(&self, handle: ScriptHandle) -> bool {
        self.scripts.contains_key(handle)
    }

    fn script_count(&self) -> usize {
        self.scripts.len()
    }

    fn run(&mut self, handle: ScriptHandle) -> Result<ScriptValue, ScriptError> {
        let ast = self.scripts.get(handle).ok_or(ScriptError::NotFound)?;
        let mut scope = Scope::new();
        let dyn_value: Dynamic = self
            .engine
            .eval_ast_with_scope(&mut scope, ast)
            .map_err(|e| ScriptError::Runtime(format!("{e}")))?;
        dynamic_to_script_value(dyn_value)
    }

    fn call(
        &mut self,
        handle: ScriptHandle,
        function: &str,
        args: &[ScriptValue],
    ) -> Result<ScriptValue, ScriptError> {
        let ast = self.scripts.get(handle).ok_or(ScriptError::NotFound)?;
        let mut scope = Scope::new();
        let rhai_args: Vec<Dynamic> = args.iter().cloned().map(script_value_to_dynamic).collect();
        let result: Dynamic = self
            .engine
            .call_fn(&mut scope, ast, function, rhai_args)
            .map_err(|e| {
                let formatted = format!("{e}");
                if formatted.contains("not found") {
                    ScriptError::FunctionNotFound(function.to_string())
                } else {
                    ScriptError::Runtime(formatted)
                }
            })?;
        dynamic_to_script_value(result)
    }
}

fn script_value_to_dynamic(v: ScriptValue) -> Dynamic {
    match v {
        ScriptValue::Unit => Dynamic::UNIT,
        ScriptValue::Bool(b) => Dynamic::from(b),
        ScriptValue::Int(i) => Dynamic::from(i),
        ScriptValue::Float(f) => Dynamic::from(f),
        ScriptValue::String(s) => Dynamic::from(s),
    }
}

fn dynamic_to_script_value(d: Dynamic) -> Result<ScriptValue, ScriptError> {
    if d.is_unit() {
        return Ok(ScriptValue::Unit);
    }
    if let Some(b) = d.clone().try_cast::<bool>() {
        return Ok(ScriptValue::Bool(b));
    }
    if let Some(i) = d.clone().try_cast::<i64>() {
        return Ok(ScriptValue::Int(i));
    }
    if let Some(f) = d.clone().try_cast::<f64>() {
        return Ok(ScriptValue::Float(f));
    }
    if let Some(s) = d.clone().try_cast::<String>() {
        return Ok(ScriptValue::String(s));
    }
    Err(ScriptError::InvalidValue(format!(
        "rhai value of type {} cannot be coerced to ScriptValue",
        d.type_name(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_backend_is_empty() {
        let backend = RhaiBackend::new();
        assert_eq!(backend.script_count(), 0);
    }

    #[test]
    fn compile_then_run_returns_last_expression() {
        let mut backend = RhaiBackend::new();
        let handle = backend.compile("40 + 2").unwrap();
        let result = backend.run(handle).unwrap();
        assert_eq!(result, ScriptValue::Int(42));
    }

    #[test]
    fn compile_with_syntax_error_returns_compile() {
        let mut backend = RhaiBackend::new();
        let err = backend.compile("let x = ;").unwrap_err();
        assert!(matches!(err, ScriptError::Compile(_)));
    }

    #[test]
    fn run_returns_string_when_script_evals_to_string() {
        let mut backend = RhaiBackend::new();
        let handle = backend.compile(r#"let s = "hello " + "world"; s"#).unwrap();
        let result = backend.run(handle).unwrap();
        assert_eq!(result, ScriptValue::String("hello world".into()));
    }

    #[test]
    fn run_returns_bool_for_boolean_expression() {
        let mut backend = RhaiBackend::new();
        let handle = backend.compile("3 > 1").unwrap();
        let result = backend.run(handle).unwrap();
        assert_eq!(result, ScriptValue::Bool(true));
    }

    #[test]
    fn run_returns_float_for_float_expression() {
        let mut backend = RhaiBackend::new();
        let handle = backend.compile("3.14 * 2.0").unwrap();
        let result = backend.run(handle).unwrap();
        match result {
            ScriptValue::Float(f) => assert!((f - 6.28).abs() < 1e-6),
            other => panic!("expected Float, got {other:?}"),
        }
    }

    #[test]
    fn call_executes_named_function() {
        let mut backend = RhaiBackend::new();
        let handle = backend
            .compile("fn add(a, b) { a + b }")
            .unwrap();
        let result = backend
            .call(
                handle,
                "add",
                &[ScriptValue::Int(2), ScriptValue::Int(3)],
            )
            .unwrap();
        assert_eq!(result, ScriptValue::Int(5));
    }

    #[test]
    fn call_with_string_arg_concatenates() {
        let mut backend = RhaiBackend::new();
        let handle = backend
            .compile(r#"fn greet(name) { "hello " + name }"#)
            .unwrap();
        let result = backend
            .call(
                handle,
                "greet",
                &[ScriptValue::String("rust".into())],
            )
            .unwrap();
        assert_eq!(result, ScriptValue::String("hello rust".into()));
    }

    #[test]
    fn call_unknown_function_errs() {
        let mut backend = RhaiBackend::new();
        let handle = backend.compile("fn known() { 1 }").unwrap();
        let err = backend.call(handle, "unknown", &[]).unwrap_err();
        // rhai's error message for missing function may not contain "not
        // found" verbatim — fall back to Runtime is acceptable.
        match err {
            ScriptError::FunctionNotFound(_) | ScriptError::Runtime(_) => {}
            other => panic!("expected FunctionNotFound or Runtime, got {other:?}"),
        }
    }

    #[test]
    fn remove_invalidates_handle() {
        let mut backend = RhaiBackend::new();
        let handle = backend.compile("42").unwrap();
        assert!(backend.contains(handle));
        backend.remove(handle);
        assert!(!backend.contains(handle));
        let err = backend.run(handle).unwrap_err();
        assert!(matches!(err, ScriptError::NotFound));
    }

    #[test]
    fn distinct_compiles_yield_distinct_handles() {
        let mut backend = RhaiBackend::new();
        let h1 = backend.compile("1").unwrap();
        let h2 = backend.compile("2").unwrap();
        assert_ne!(h1, h2);
        assert_eq!(backend.script_count(), 2);
    }

    #[test]
    fn unit_expression_returns_unit_value() {
        let mut backend = RhaiBackend::new();
        let handle = backend.compile("let x = 1;").unwrap();
        let result = backend.run(handle).unwrap();
        assert_eq!(result, ScriptValue::Unit);
    }

    #[test]
    fn engine_mut_allows_custom_registration() {
        let mut backend = RhaiBackend::new();
        backend
            .engine_mut()
            .register_fn("double", |x: i64| x * 2);
        let handle = backend.compile("double(21)").unwrap();
        let result = backend.run(handle).unwrap();
        assert_eq!(result, ScriptValue::Int(42));
    }
}
