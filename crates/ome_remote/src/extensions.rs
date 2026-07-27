//! Methods this crate does not know about.
//!
//! # Why the protocol has a hole in it on purpose
//!
//! `ome_remote` depends on `ome_core` and `ome_ecs` and nothing else. That
//! is deliberate: the protocol is about entities, components and fields,
//! which is what every subsystem has in common.
//!
//! Physics wanted more than that — the editor needs the solver's own
//! account of itself to draw a debug overlay (#634), and the solver is in
//! this process while the editor is in another. Answering it by having
//! `ome_remote` depend on `ome_physics` would work once, and then again for
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

use ome_core::resource::Resources;

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
mod tests {
    use super::*;

    fn echo() -> ExtensionHandler {
        Box::new(|_, payload| Ok(payload.clone()))
    }

    #[test]
    fn a_registered_extension_is_called() {
        let mut resources = Resources::new();
        let mut extensions = RemoteExtensions::default();
        extensions.register("test.echo", echo());
        resources.insert(extensions);

        let payload = serde_json::json!({ "hello": 1 });
        let result = call(&mut resources, "test.echo", &payload);

        assert_eq!(result, Some(Ok(payload)));
    }

    /// The registry has to go back, or the host serves one request and
    /// then forgets every extension it had.
    #[test]
    fn the_registry_survives_a_call() {
        let mut resources = Resources::new();
        let mut extensions = RemoteExtensions::default();
        extensions.register("test.echo", echo());
        resources.insert(extensions);

        let _ = call(&mut resources, "test.echo", &serde_json::Value::Null);

        assert!(
            resources
                .get::<RemoteExtensions>()
                .is_some_and(|e| e.contains("test.echo")),
            "the registry did not come back",
        );
    }

    /// An unknown name is `None`, not an error — the caller decides what
    /// that means to a client, and it is a different thing from a handler
    /// that ran and failed.
    #[test]
    fn an_unknown_name_is_not_a_failure() {
        let mut resources = Resources::new();
        resources.insert(RemoteExtensions::default());

        assert!(call(&mut resources, "test.missing", &serde_json::Value::Null).is_none());
    }

    /// A handler that fails is `Some(Err(..))`, so the two cases stay
    /// distinguishable all the way to the client.
    #[test]
    fn a_failing_handler_reports_its_own_error() {
        let mut resources = Resources::new();
        let mut extensions = RemoteExtensions::default();
        extensions.register(
            "test.fails",
            Box::new(|_, _| Err("the solver said no".to_owned())),
        );
        resources.insert(extensions);

        let result = call(&mut resources, "test.fails", &serde_json::Value::Null);

        assert_eq!(result, Some(Err("the solver said no".to_owned())));
    }

    /// A host with no registry at all — nothing registered any extension —
    /// answers "no such extension" rather than panicking.
    #[test]
    fn a_host_without_the_resource_serves_nothing() {
        let mut resources = Resources::new();
        assert!(call(&mut resources, "test.echo", &serde_json::Value::Null).is_none());
    }

    /// Registering twice replaces, so a plugin added twice behaves like a
    /// plugin added once.
    #[test]
    fn registering_twice_replaces() {
        let mut extensions = RemoteExtensions::default();
        extensions.register("test.echo", echo());
        extensions.register(
            "test.echo",
            Box::new(|_, _| Ok(serde_json::json!("second"))),
        );
        assert_eq!(extensions.len(), 1);
    }
}
