//! The shader graph (#1159): what the nodes are, and how a graph is carried inside the `.shader`
//! it generates.
//!
//! 🔴 The graph is the source of truth and the WGSL beside it is output. It rides in a block comment
//! at the top of the file, as Shader Forge carries `/*SF_DATA;…*/`, so the two cannot drift apart
//! and the editor can tell which shaders it may open.

use egui_snarl::Snarl;
use serde::{Deserialize, Serialize};

mod codegen;

pub(crate) use codegen::generate;

/// What the block comment holding a graph starts with.
const MARKER: &str = "/*KOOCH_GRAPH;";

/// Every wire carries a `vec4<f32>`: unused components are zero, and a node reads the components it
/// needs. One type means no casts, no coercion rules, and no wire a user cannot plug in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Node {
    /// The mesh's uv, in `xy`.
    Uv,
    /// The shaded point, in world space.
    WorldPosition,
    /// The interpolated normal, in world space.
    WorldNormal,
    /// `camera_position - world_position`, normalised.
    ViewDirection,
    /// A number the material edits: one member of `SurfaceParams`.
    Param {
        name: String,
        /// How wide the member is, 1..=4.
        width: u32,
        /// Drawn as a colour when it is four wide.
        color: bool,
        default: [f32; 4],
    },
    /// A texture the material assigns, sampled at `uv`.
    Texture {
        name: String,
        /// `white`, `black` or `normal` while unassigned.
        fallback: String,
    },
    /// A value written into the graph rather than the material.
    Constant([f32; 4]),
    Add,
    Multiply,
    /// `mix(a, b, t.x)`.
    Mix,
    /// `dot(a.xyz, b.xyz)`, in every component.
    Dot,
    /// `pow(max(a.x, 0), b.x)`.
    Power,
    Saturate,
    /// What Inti lights: base colour, normal, metallic, roughness, emissive.
    Output,
}

impl Node {
    /// What the node is called in the panel.
    pub(crate) fn title(&self) -> String {
        match self {
            Self::Uv => "UV".to_owned(),
            Self::WorldPosition => "World Position".to_owned(),
            Self::WorldNormal => "World Normal".to_owned(),
            Self::ViewDirection => "View Direction".to_owned(),
            Self::Param { name, .. } => format!("Param {name}"),
            Self::Texture { name, .. } => format!("Texture {name}"),
            Self::Constant(_) => "Constant".to_owned(),
            Self::Add => "Add".to_owned(),
            Self::Multiply => "Multiply".to_owned(),
            Self::Mix => "Mix".to_owned(),
            Self::Dot => "Dot".to_owned(),
            Self::Power => "Power".to_owned(),
            Self::Saturate => "Saturate".to_owned(),
            Self::Output => "Surface Output".to_owned(),
        }
    }

    /// The inputs it takes, named for their pins.
    pub(crate) fn inputs(&self) -> &'static [&'static str] {
        match self {
            Self::Uv
            | Self::WorldPosition
            | Self::WorldNormal
            | Self::ViewDirection
            | Self::Param { .. }
            | Self::Constant(_) => &[],
            Self::Texture { .. } => &["uv"],
            Self::Add | Self::Multiply | Self::Dot | Self::Power => &["a", "b"],
            Self::Mix => &["a", "b", "t"],
            Self::Saturate => &["value"],
            Self::Output => &["base color", "normal", "metallic", "roughness", "emissive"],
        }
    }

    /// Whether it produces a value.
    pub(crate) fn has_output(&self) -> bool {
        !matches!(self, Self::Output)
    }
}

/// A graph and where its nodes sit, as the panel holds it.
pub(crate) type Graph = Snarl<Node>;

/// The graph carried by `source`, if it holds one.
pub(crate) fn extract(source: &str) -> Option<Graph> {
    let start = source.find(MARKER)? + MARKER.len();
    let end = start + source[start..].find("*/")?;
    match ron::from_str(&source[start..end]) {
        Ok(graph) => Some(graph),
        Err(error) => {
            tracing::warn!(%error, "a shader carries a graph this editor cannot read");
            None
        }
    }
}

/// Whether `source` was written by the graph tool — what decides if the editor offers to open it.
pub(crate) fn is_generated(source: &str) -> bool {
    source.contains(MARKER)
}

/// The block that carries `graph`, as one line so the file stays readable below it.
fn embed(graph: &Graph) -> Result<String, String> {
    let ron = ron::ser::to_string(graph).map_err(|e| e.to_string())?;
    // 🔴 A `*/` inside the data would close the comment early. Nothing serialises one today, and a
    // graph that did would silently truncate rather than fail.
    if ron.contains("*/") {
        return Err("the graph holds `*/`, which would close its own comment".to_owned());
    }
    Ok(format!("{MARKER}{ron}*/"))
}

#[cfg(test)]
mod tests;
