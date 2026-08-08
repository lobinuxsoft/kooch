//! Methods this crate does not know about.
//!
//! # Why the protocol has a hole in it on purpose
//!
//! `kooch_remote` depends on `kooch_core` and `kooch_ecs` and nothing else. That
//! is deliberate: the protocol is about entities, components and fields,
//! which is what every subsystem has in common.
//!
//! Physics wanted more than that — the editor needs the solver's own
//! account of itself to draw a debug overlay (#634), and the solver is in
//! this process while the editor is in another. Answering it by having
//! `kooch_remote` depend on `kooch_physics` would work once, and then again for
//! scene queries (#562), and again for gravity fields (#624), until the
//! protocol crate depends on every subsystem in the engine and cannot be
//! built without them.
//!
//! So subsystems register instead. A handler is a name, a JSON payload in
//! and a JSON value out; the protocol carries it without understanding it.
//! Neither crate learns about the other — they meet in the facade, which
//! already depends on both.
//!
//! # Namespacing is not decoration
//!
//! Names are `subsystem.method`, because two subsystems will eventually
//! both want to be called `query`. A flat namespace with two claimants is a
//! bug that only appears when both features are on.

use std::collections::HashMap;

use kooch_core::resource::Resources;

/// What an extension does: read or change the world, and answer.
///
/// The error is a string rather than a type, because the protocol cannot
/// know what a subsystem's failures are. It reaches the caller inside
/// [`RemoteError::ExtensionFailed`](crate::protocol::RemoteError).
pub type ExtensionHandler = Box<
    dyn Fn(&mut Resources, &serde_json::Value) -> Result<serde_json::Value, String> + Send + Sync,
>;

/// The extensions this host serves, by name.
///
/// A resource, so a plugin registers into it at startup the same way it
/// inserts anything else.
#[derive(Default)]
pub struct RemoteExtensions {
    handlers: HashMap<String, ExtensionHandler>,
}

impl RemoteExtensions {
    /// Registers a handler, replacing one of the same name.
    ///
    /// Replacing rather than refusing: a plugin added twice is a
    /// configuration mistake that should behave like the plugin was added
    /// once, not one that half-registers.
    pub fn register(&mut self, name: impl Into<String>, handler: ExtensionHandler) {
        let name = name.into();
        debug_assert!(
            name.contains('.'),
            "extension names are `subsystem.method`, got {name:?}",
        );
        self.handlers.insert(name, handler);
    }

    /// Whether a name is served here.
    pub fn contains(&self, name: &str) -> bool {
        self.handlers.contains_key(name)
    }

    /// The names on offer, for a client that wants to know what this host
    /// can do before asking.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.handlers.keys().map(String::as_str)
    }

    /// How many are registered.
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// `true` when nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

/// Calls an extension, if this host serves it.
///
/// The registry is lifted out of `Resources` and put back, because a
/// handler takes `&mut Resources` and cannot borrow the list it is being
/// read from. The same shape the event updaters use.
///
/// `None` means no such extension — the caller turns that into the
/// protocol's own error, so this stays free of protocol types.
pub fn call(
    resources: &mut Resources,
    name: &str,
    payload: &serde_json::Value,
) -> Option<Result<serde_json::Value, String>> {
    let extensions = resources.remove::<RemoteExtensions>()?;
    let result = extensions
        .handlers
        .get(name)
        .map(|handler| handler(resources, payload));
    resources.insert(extensions);
    result
}

#[cfg(test)]
mod tests;
