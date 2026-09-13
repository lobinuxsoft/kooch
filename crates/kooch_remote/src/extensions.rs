//! Methods this crate does not know about: subsystems register `subsystem.method` handlers (JSON
//! in, JSON out), so `kooch_remote` never depends on physics, queries or gravity (#634).
//! Namespaced because two subsystems will both want `query`.

use std::collections::HashMap;

use kooch_core::resource::Resources;

/// What an extension does: read or change the world, and answer. Errors are strings the protocol
/// cannot type, returned as [`RemoteError::ExtensionFailed`](crate::protocol::RemoteError).
pub type ExtensionHandler = Box<
    dyn Fn(&mut Resources, &serde_json::Value) -> Result<serde_json::Value, String> + Send + Sync,
>;

/// The extensions this host serves, by name — a resource plugins register into at startup.
#[derive(Default)]
pub struct RemoteExtensions {
    handlers: HashMap<String, ExtensionHandler>,
}

impl RemoteExtensions {
    /// Registers a handler, replacing one of the same name, so a plugin added twice behaves like
    /// once.
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

/// Calls an extension if this host serves it, lifting the registry out of `Resources` so a handler
/// can borrow them. `None` means no such extension.
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
