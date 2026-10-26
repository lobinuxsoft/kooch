use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

struct TestPlugin {
    built: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
}

impl Plugin for TestPlugin {
    fn build(&self, _app: &mut App) {
        self.built.store(true, Ordering::SeqCst);
    }

    fn finish(&self, _app: &mut App) {
        self.finished.store(true, Ordering::SeqCst);
    }

    fn name(&self) -> &str {
        "TestPlugin"
    }
}

#[test]
fn plugin_build_and_finish() {
    let built = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));

    let plugin = TestPlugin {
        built: built.clone(),
        finished: finished.clone(),
    };

    let mut app = App::new();
    app.add_plugin(plugin);
    app.finish_plugins();

    assert!(built.load(Ordering::SeqCst));
    assert!(finished.load(Ordering::SeqCst));
}

struct PluginA;
struct PluginB;

impl Plugin for PluginA {
    fn build(&self, _app: &mut App) {}
    fn name(&self) -> &str {
        "PluginA"
    }
}

impl Plugin for PluginB {
    fn build(&self, _app: &mut App) {}
    fn name(&self) -> &str {
        "PluginB"
    }
}

struct TestGroup;

impl PluginGroup for TestGroup {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::new().add(PluginA).add(PluginB)
    }
}

#[test]
fn plugin_group_builder() {
    let plugins = TestGroup.build().finish();
    assert_eq!(plugins.len(), 2);
    assert_eq!(plugins[0].name(), "PluginA");
    assert_eq!(plugins[1].name(), "PluginB");
}

#[test]
fn minimal_plugins() {
    let plugins = MinimalPlugins.build().finish();
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].name(), "CorePlugin");
}

struct Counted(Arc<std::sync::atomic::AtomicU32>);

impl Plugin for Counted {
    fn build(&self, _app: &mut App) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct Twice(Arc<std::sync::atomic::AtomicU32>);

impl PluginGroup for Twice {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::new()
            .add(Counted(self.0.clone()))
            .add(Counted(self.0))
    }
}

/// A plugin built twice registers its systems twice: double gravity in a shipped game (#673).
#[test]
fn a_repeated_plugin_builds_once() {
    let builds = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let mut app = App::new();
    app.add_plugins(Twice(builds.clone()));
    app.add_plugin(Counted(builds.clone()));
    assert_eq!(builds.load(Ordering::SeqCst), 1);
}
