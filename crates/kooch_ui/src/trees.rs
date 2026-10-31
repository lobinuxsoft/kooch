//! Every document's tree, by entity. CPU-only: what crosses to the GPU is the quads (#96 phase 4).

use std::collections::HashMap;

use kooch_core::resource::Resources;
use kooch_core::run_state::Playing;
use kooch_ecs::Entity;
use kooch_ecs::query::Query;

use crate::{UiDocument, UiFocus, UiPointer, UiTree};

/// The node tree of each entity with a [`UiDocument`].
#[derive(Debug, Default)]
pub struct UiTrees(HashMap<Entity, UiTree>);

impl UiTrees {
    pub fn insert(&mut self, document: Entity, tree: UiTree) {
        self.0.insert(document, tree);
    }

    pub fn get(&self, document: Entity) -> Option<&UiTree> {
        self.0.get(&document)
    }

    pub fn get_mut(&mut self, document: Entity) -> Option<&mut UiTree> {
        self.0.get_mut(&document)
    }

    /// Writes `content` into the node called `name` of `document`, at the size its style gave it.
    pub fn write_text(&mut self, document: Entity, name: &str, content: &str) {
        if let Some(tree) = self.get_mut(document)
            && let Some(node) = tree.query(name)
        {
            tree.write_text(node, content);
        }
    }

    pub(crate) fn for_each(&self, each: impl FnMut(&UiTree)) {
        self.0.values().for_each(each);
    }

    pub(crate) fn for_each_mut(&mut self, mut each: impl FnMut(&mut UiTree)) {
        self.0.values_mut().for_each(|tree| each(tree));
    }
}

/// Drops the tree of every entity that no longer has a [`UiDocument`], despawned or not, and every
/// tree on Stop: Stop restores the entities, and a tree the game wrote into is play state too.
pub fn prune_trees(resources: &mut Resources) {
    let playing = Playing::is_playing(resources);
    let stopped = resources
        .insert(WasPlaying(playing))
        .is_some_and(|was| was.0 && !playing);
    let mut documents = Vec::new();
    Query::<&UiDocument>::new(resources).for_each_entity(|entity, _| documents.push(entity));
    if let Some(trees) = resources.get_mut::<UiTrees>() {
        trees
            .0
            .retain(|entity, _| !stopped && documents.contains(entity));
    }
    if stopped {
        resources.insert(UiPointer::default());
        resources.insert(UiFocus::default());
    }
}

struct WasPlaying(bool);
