//! `-kooch-material` (#1690): the `kind: ui` shader a node draws with and the values it hands it.
//! This crate only carries them: the renderer reads the shader and packs the values by name.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::SystemTime;

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_loader::AssetServer;
use kooch_core::resource::Resources;

use crate::documents::{path_of, read_bytes, stamp};
use crate::{UiNode, UiTree, UiTrees};

const BIND_PREFIX: &str = "bind:param.";

/// One value of a node's material.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NodeParam {
    pub name: Arc<str>,
    pub value: [f32; 4],
    /// Set by code or a binding: a restyle keeps it.
    pub pinned: bool,
}

/// `bind:param.fill="Type.field"`: a parameter that follows a field.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamBind {
    pub node: UiNode,
    pub param: Arc<str>,
    pub component: Box<str>,
    pub field: Box<str>,
}

/// The text of every shader a tree draws with, and the mtime it was read at.
#[derive(Debug, Default)]
pub struct UiShaders(HashMap<Guid, (SystemTime, Arc<str>)>);

impl UiShaders {
    pub fn source(&self, guid: Guid) -> Option<&Arc<str>> {
        self.0.get(&guid).map(|(_, source)| source)
    }
}

impl UiTree {
    pub fn set_material(&mut self, node: UiNode, material: Option<Guid>) {
        if let Some(own) = self.material.get_mut(node.0 as usize) {
            *own = material;
        }
    }

    pub fn material(&self, node: UiNode) -> Option<Guid> {
        self.material.get(node.0 as usize).copied().flatten()
    }

    /// A parameter of the node's material, kept over what the style says. No restyle runs.
    pub fn set_param(&mut self, node: UiNode, name: &str, value: impl Into<ParamValue>) {
        let value = value.into().0;
        let Some(params) = self.params.get_mut(node.0 as usize) else {
            return;
        };
        match params.iter_mut().find(|param| &*param.name == name) {
            Some(param) => {
                param.value = value;
                param.pinned = true;
            }
            None => params.push(NodeParam {
                name: name.into(),
                value,
                pinned: true,
            }),
        }
    }

    pub fn param(&self, node: UiNode, name: &str) -> Option<[f32; 4]> {
        self.params
            .get(node.0 as usize)?
            .iter()
            .find(|param| &*param.name == name)
            .map(|param| param.value)
    }

    /// What the cascade says, under whatever code pinned.
    pub(crate) fn style_params(&mut self, node: UiNode, styled: &[(Box<str>, [f32; 4])]) {
        let Some(params) = self.params.get_mut(node.0 as usize) else {
            return;
        };
        params.retain(|param| param.pinned);
        for (name, value) in styled {
            if !params.iter().any(|param| *param.name == **name) {
                params.push(NodeParam {
                    name: (&**name).into(),
                    value: *value,
                    pinned: false,
                });
            }
        }
    }

    /// Records `key` when it is a `bind:param.<name>`; any other attribute is not this one's.
    pub(crate) fn bind_param(&mut self, node: UiNode, key: &str, path: &str) {
        let Some(param) = key.strip_prefix(BIND_PREFIX) else {
            return;
        };
        if let Some((component, field)) = path.split_once('.') {
            self.param_binds.push(ParamBind {
                node,
                param: param.into(),
                component: component.into(),
                field: field.into(),
            });
        }
    }

    pub fn param_binds(&self) -> &[ParamBind] {
        &self.param_binds
    }
}

/// A float or up to four, for [`UiTree::set_param`].
pub struct ParamValue([f32; 4]);

impl From<f32> for ParamValue {
    fn from(value: f32) -> Self {
        Self([value, 0.0, 0.0, 0.0])
    }
}

impl From<[f32; 4]> for ParamValue {
    fn from(value: [f32; 4]) -> Self {
        Self(value)
    }
}

/// Reads every shader a tree names that is missing or older than its file, so editing one shows
/// without Play. A stat per shader per frame, no read.
pub fn load_shaders(resources: &mut Resources) {
    let wanted: HashSet<Guid> = resources
        .get::<UiTrees>()
        .map(|trees| {
            let mut wanted = HashSet::new();
            trees.for_each(|tree| wanted.extend(tree.material.iter().flatten().copied()));
            wanted
        })
        .unwrap_or_default();
    if wanted.is_empty() {
        return;
    }
    let Some(mut shaders) = resources.remove::<UiShaders>() else {
        return;
    };
    let mut server = resources.remove::<AssetServer>();
    if let Some(database) = resources.get::<AssetDatabase>() {
        read_shaders(wanted, database, server.as_mut(), &mut shaders);
    }
    if let Some(server) = server {
        resources.insert(server);
    }
    resources.insert(shaders);
}

/// [`load_shaders`] for one tree drawn on its own, as the UI Builder's canvas is.
pub fn load_tree_shaders(
    tree: &UiTree,
    database: &AssetDatabase,
    server: Option<&mut AssetServer>,
    shaders: &mut UiShaders,
) {
    let wanted = tree.material.iter().flatten().copied().collect();
    read_shaders(wanted, database, server, shaders);
}

fn read_shaders(
    wanted: HashSet<Guid>,
    database: &AssetDatabase,
    mut server: Option<&mut AssetServer>,
    shaders: &mut UiShaders,
) {
    for guid in wanted {
        let modified = stamp(database, guid, None, &[]);
        if shaders.0.get(&guid).is_some_and(|(at, _)| *at == modified) {
            continue;
        }
        let text = path_of(database, guid).and_then(|path| {
            let bytes = read_bytes(server.as_deref_mut(), &path)?;
            String::from_utf8(bytes).map_err(|e| format!("{}: {e}", path.display()))
        });
        match text {
            Ok(text) => {
                shaders.0.insert(guid, (modified, text.into()));
            }
            Err(error) => kooch_core::warn_once!("UI shader not loaded: {error}"),
        }
    }
}
