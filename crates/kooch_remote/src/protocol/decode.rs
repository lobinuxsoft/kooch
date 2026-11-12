//! Reads a [`Response`] straight off the JSON, the same shape [`Serialize`] writes (#1891).
//!
//! 🔴 The derived reader copied every reply into serde's `Content` tree twice, once for `flatten`
//! and once for the `kind` tag, before building it: a world snapshot at Play cost 237 MB of small blocks.

use serde::de::value::MapAccessDeserializer;
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

use super::{
    EntityId, EntitySnapshot, HostMetrics, HostSignal, MovedComponent, MovedTransform, RemoteError,
    Response, ResponseData, ResponsePayload, SceneEntry,
};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ResponseKey {
    Id,
    Result,
    Error,
    #[serde(other)]
    Other,
}

impl<'de> Deserialize<'de> for Response {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(ResponseVisitor)
    }
}

struct ResponseVisitor;

impl<'de> Visitor<'de> for ResponseVisitor {
    type Value = Response;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a response with an id and a result or an error")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Response, A::Error> {
        let mut id = None;
        let mut payload = None;
        while let Some(key) = map.next_key()? {
            match key {
                ResponseKey::Id => id = Some(map.next_value()?),
                ResponseKey::Result => {
                    payload = Some(ResponsePayload::Result(map.next_value_seed(DataSeed)?))
                }
                ResponseKey::Error => {
                    payload = Some(ResponsePayload::Error(map.next_value::<RemoteError>()?))
                }
                ResponseKey::Other => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(Response {
            id: id.ok_or_else(|| de::Error::missing_field("id"))?,
            payload: payload.ok_or_else(|| de::Error::missing_field("result"))?,
        })
    }
}

struct DataSeed;

impl<'de> DeserializeSeed<'de> for DataSeed {
    type Value = ResponseData;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<ResponseData, D::Error> {
        deserializer.deserialize_map(DataVisitor)
    }
}

struct DataVisitor;

impl<'de> Visitor<'de> for DataVisitor {
    type Value = ResponseData;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a result object whose first key is `kind`")
    }

    // `kind` is first because `Serialize` writes the tag first; that is what lets the big
    // replies stream into their fields. The small ones go through the derived reader.
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ResponseData, A::Error> {
        let first: Option<String> = map.next_key()?;
        if first.as_deref() != Some("kind") {
            return Err(de::Error::custom("a result must start with `kind`"));
        }
        let kind: String = map.next_value()?;
        let rest = MapAccessDeserializer::new(map);
        match kind.as_str() {
            "moved" => MovedBody::deserialize(rest).map(MovedBody::into_data),
            "entities" => EntitiesBody::deserialize(rest).map(EntitiesBody::into_data),
            _ => {
                let mut fields = serde_json::Map::deserialize(rest)?;
                fields.insert("kind".to_owned(), serde_json::Value::String(kind));
                ResponseData::deserialize(serde_json::Value::Object(fields))
                    .map_err(de::Error::custom)
            }
        }
    }
}

/// [`ResponseData::Moved`]'s fields with its defaults. Built whole in `into_data`, so a field
/// added to the variant and not here does not compile.
#[derive(Deserialize)]
struct MovedBody {
    moved: Vec<MovedTransform>,
    #[serde(default)]
    components: Vec<MovedComponent>,
    #[serde(default)]
    removed: Vec<EntityId>,
    revision: u64,
    full: bool,
    #[serde(default)]
    host: Option<HostMetrics>,
    #[serde(default)]
    signals: Vec<HostSignal>,
    #[serde(default)]
    ui: Option<crate::ui_frame::UiFrame>,
}

impl MovedBody {
    fn into_data(self) -> ResponseData {
        let Self {
            moved,
            components,
            removed,
            revision,
            full,
            host,
            signals,
            ui,
        } = self;
        ResponseData::Moved {
            moved,
            components,
            removed,
            revision,
            full,
            host,
            signals,
            ui,
        }
    }
}

/// [`ResponseData::Entities`]'s fields with its defaults, like [`MovedBody`].
#[derive(Deserialize)]
struct EntitiesBody {
    entities: Vec<EntitySnapshot>,
    #[serde(default)]
    removed: Vec<EntityId>,
    #[serde(default)]
    revision: u64,
    #[serde(default)]
    full: bool,
    #[serde(default)]
    host: Option<HostMetrics>,
    #[serde(default)]
    scenes: Option<Vec<SceneEntry>>,
}

impl EntitiesBody {
    fn into_data(self) -> ResponseData {
        let Self {
            entities,
            removed,
            revision,
            full,
            host,
            scenes,
        } = self;
        ResponseData::Entities {
            entities,
            removed,
            revision,
            full,
            host,
            scenes,
        }
    }
}
