//! Several scenes open at once (#609).

use super::{setup_resources, tmp_path};
use crate::commands::Commands;
use crate::component::ComponentRegistry;
use crate::entity::Entity;
use crate::scene_manager::SceneManager;
use crate::scene_member::SceneMember;
use kooch_core::Guid;
use kooch_core::resource::Resources;

/// Writes a scene file holding `count` entities with the given hp values.
fn write_scene(name: &str, hps: &[u32]) -> std::path::PathBuf {
    use crate::scene::SceneDocument;

    let mut resources = setup_resources();
    {
        let mut commands = resources.remove::<Commands>().unwrap();
        for &hp in hps {
            commands
                .spawn(&mut resources)
                .insert_reflected(super::single_scene::Health { hp });
        }
        commands.apply(&mut resources);
        resources.insert(commands);
    }

    let path = tmp_path(name);
    let mut manager = SceneManager::new();
    manager
        .save_as(path.clone(), &mut resources)
        .expect("writes the fixture");
    path
}

fn live_hps(resources: &Resources) -> Vec<u32> {
    use crate::query::Query;
    let mut hps: Vec<u32> = Query::<&super::single_scene::Health>::new(resources)
        .iter()
        .map(|h| h.hp)
        .collect();
    hps.sort_unstable();
    hps
}

fn members(resources: &Resources) -> Vec<(Entity, Guid)> {
    resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<SceneMember>())
        .map(|s| s.iter().map(|(&e, m)| (e, m.scene)).collect())
        .unwrap_or_default()
}

mod identity;
mod open_close;
use super::single_scene;
