//! Every document's tree, by entity. CPU-only: what crosses to the GPU is the quads (#96 phase 4).

use std::collections::HashMap;

use kooch_core::resource::Resources;
use kooch_ecs::Entity;
use kooch_ecs::query::Query;

use crate::{UiDocument, UiTree};

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
}

/// Drops the tree of every entity that no longer has a [`UiDocument`], despawned or not.
pub fn prune_trees(resources: &mut Resources) {
    let mut documents = Vec::new();
    Query::<&UiDocument>::new(resources).for_each_entity(|entity, _| documents.push(entity));
    if let Some(trees) = resources.get_mut::<UiTrees>() {
        trees.0.retain(|entity, _| documents.contains(entity));
    }
}
