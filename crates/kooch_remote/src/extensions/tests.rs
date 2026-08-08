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
