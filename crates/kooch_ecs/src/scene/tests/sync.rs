use crate::commands::Commands;
use crate::component::ComponentRegistry;
use crate::query::Query;
use crate::reflect::ReflectValue;
use crate::scene::{ComponentDescription, EntityDescription, SceneDocument, sync_scene_to_ecs};
use crate::transform::Transform;

use super::{Health, TestAssetHolder, TestEphemeral, setup_resources};

mod round_trips;
use super::{add_to_archetype, captured};
mod names_and_warnings;
