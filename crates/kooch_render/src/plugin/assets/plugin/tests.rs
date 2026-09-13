use super::*;
use kooch_core::asset_loader::{AssetLoader, AssetResult, LoadContext};

#[derive(Debug, Clone, PartialEq)]
struct Probe(String);

#[derive(Clone)]
struct ProbeLoader;
impl AssetLoader<Probe> for ProbeLoader {
    fn extensions(&self) -> &[&'static str] {
        &["probe"]
    }
    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<Probe> {
        Ok(Probe(String::from_utf8_lossy(&bytes).into_owned()))
    }
}

fn empty_plugin() -> AssetPlugin {
    AssetPlugin::new().with_root(std::env::temp_dir().join("kooch_no_such_assets"))
}

/// 🔴 A contributed asset arrives with **both** halves: the loader that reads it and the `Assets<T>`
/// that loader fills.
#[test]
fn a_contributed_asset_brings_its_loader_and_its_storage() {
    let mut app = App::new();
    empty_plugin()
        .with_asset::<Probe, _>(ProbeLoader)
        .build(&mut app);

    let resources = app.resources_mut();
    assert!(
        resources
            .get::<AssetServer>()
            .is_some_and(|server| server.has_loader::<Probe>()),
        "the contributed loader never reached the server"
    );
    assert!(
        resources.get::<Assets<Probe>>().is_some(),
        "the loader is registered with nowhere to put what it loads,              which fails every load with `Assets<T> resource missing`"
    );
}

/// Several crates contributing is the case this exists for — input
/// today, audio next — so more than one has to survive.
#[test]
fn every_contributed_asset_survives() {
    #[derive(Debug, Clone)]
    struct Other(u8);
    #[derive(Clone)]
    struct OtherLoader;
    impl AssetLoader<Other> for OtherLoader {
        fn extensions(&self) -> &[&'static str] {
            &["other"]
        }
        fn load(&self, _b: &[u8], _c: &mut LoadContext<'_>) -> AssetResult<Other> {
            Ok(Other(0))
        }
    }

    let mut app = App::new();
    empty_plugin()
        .with_asset::<Probe, _>(ProbeLoader)
        .with_asset::<Other, _>(OtherLoader)
        .build(&mut app);

    let resources = app.resources_mut();
    assert!(resources.get::<Assets<Probe>>().is_some());
    assert!(resources.get::<Assets<Other>>().is_some());
}
