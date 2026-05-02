//! [`ScriptingBackend`] trait + shared types.
//!
//! Backends own their compiled scripts via [`ScriptHandle`] (slotmap key
//! pattern, same as `ome_physics::BodyHandle`). Game code stores the
//! handle in a component, calls `run` / `call` on the backend trait
//! object.
//!
//! Cross-backend value type [`ScriptValue`] is a tagged union of the
//! primitive types every host language exposes uniformly. Backends
//! convert from / to their internal representations
//! (`rhai::Dynamic`, `mlua::Value`, `wasm_bindgen` JsValue, etc.).

use std::fmt;

slotmap::new_key_type! {
    /// Opaque handle to a compiled script owned by a [`ScriptingBackend`].
    /// 16 bytes Copy + Hash. Stale handles (after `remove`) yield `None`
    /// from `contains`, errors from `run`/`call` — generation-counter
    /// safety is the slotmap default.
    pub struct ScriptHandle;
}

/// Cross-backend script value. Primitive-only by design — complex types
/// (entities, vec3, asset handles) round-trip via integer ids and the
/// host code interprets them.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptValue {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
}

impl ScriptValue {
    /// Borrows the inner string when the variant is `String`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ScriptValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Returns the boolean payload, or `None` for other variants.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ScriptValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Returns the integer payload, or `None` for other variants.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            ScriptValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Returns the float payload, or `None` for other variants.
    /// Integer values are NOT auto-promoted — use `as_float_lossy` if
    /// the script may return either.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            ScriptValue::Float(f) => Some(*f),
            _ => None,
        }
    }
}

/// Errors surfaced by [`ScriptingBackend`] operations.
#[derive(Debug)]
pub enum ScriptError {
    /// The source failed to parse / compile. Backend-specific message.
    Compile(String),
    /// Runtime trap during `run` / `call`. Includes the backend's
    /// formatted error chain (line, column, etc., when available).
    Runtime(String),
    /// Handle does not name a live script.
    NotFound,
    /// The named function was not found in the script.
    FunctionNotFound(String),
    /// A returned or argument value could not be coerced into the
    /// expected [`ScriptValue`] variant.
    InvalidValue(String),
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScriptError::Compile(msg) => write!(f, "script compile failed: {msg}"),
            ScriptError::Runtime(msg) => write!(f, "script runtime error: {msg}"),
            ScriptError::NotFound => write!(f, "script handle is stale"),
            ScriptError::FunctionNotFound(name) => write!(f, "script function not found: {name}"),
            ScriptError::InvalidValue(msg) => write!(f, "invalid script value: {msg}"),
        }
    }
}

impl std::error::Error for ScriptError {}

/// Engine-facing scripting interface.
///
/// Implementations own a handle table of compiled scripts. Each method
/// touches at most one script; concurrent calls are serialized via the
/// `&mut self` discipline (or the impl's interior mutex when needed).
///
/// # Lifecycle
///
/// 1. Engine inserts a backend at startup
///    (`Box<dyn ScriptingBackend>` as a Resource).
/// 2. Loaders / hot-reload watchers call [`compile`](Self::compile) and
///    stash the returned handle on the owning entity / asset.
/// 3. Game systems call [`run`](Self::run) (top-level body) or
///    [`call`](Self::call) (named function) per frame.
/// 4. [`remove`](Self::remove) frees the script when the asset unloads.
pub trait ScriptingBackend: Send + Sync + 'static {
    /// Parses + type-checks `source`, returning a handle to the
    /// resulting compiled artifact.
    fn compile(&mut self, source: &str) -> Result<ScriptHandle, ScriptError>;

    /// Drops a compiled script. No-op for stale handles.
    fn remove(&mut self, handle: ScriptHandle);

    /// Whether the handle is live.
    fn contains(&self, handle: ScriptHandle) -> bool;

    /// How many scripts are currently compiled.
    fn script_count(&self) -> usize;

    /// Executes the script's top-level body and returns the final
    /// expression value (`Unit` if there is none).
    fn run(&mut self, handle: ScriptHandle) -> Result<ScriptValue, ScriptError>;

    /// Calls a named function inside the script with `args`. Top-level
    /// body is NOT re-executed.
    fn call(
        &mut self,
        handle: ScriptHandle,
        function: &str,
        args: &[ScriptValue],
    ) -> Result<ScriptValue, ScriptError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_value_accessors_isolate_variant() {
        let v = ScriptValue::Int(42);
        assert_eq!(v.as_int(), Some(42));
        assert_eq!(v.as_float(), None);
        assert_eq!(v.as_bool(), None);
        assert_eq!(v.as_str(), None);
    }

    #[test]
    fn script_value_string_accessor_borrows() {
        let v = ScriptValue::String("hello".into());
        assert_eq!(v.as_str(), Some("hello"));
    }

    #[test]
    fn script_error_display_includes_message() {
        let err = ScriptError::Compile("syntax error at line 3".into());
        let s = format!("{err}");
        assert!(s.contains("compile"));
        assert!(s.contains("syntax error"));
    }
}
