//! A pivot that turns on its own axis.
//!
//! # 🔴 The name is a scar, not a description
//!
//! This module was gated behind a `testing` feature, on the rule that a
//! shipped game may not carry what it does not use (#558). [`Spin`] no
//! longer qualifies, and it never really did: the bar that module set
//! was "removing it from a release build must not change what a player
//! sees", and removing this stops every orbiting light in the scene.
//!
//! It could not be reached from a build at all, which is how it was
//! found — a game exported with lights that move in the editor and stand
//! still in the build, because an unregistered component is DROPPED on
//! load rather than refused.
//!
//! # Why it is still called `testing`
//!
//! Because a scene stores `"kooch_ecs::testing::spin::Spin"` and
//! resolves it by that string. Renaming the module is not a refactor,
//! it is a data migration: every entity carrying a `Spin` would come
//! back without one, no error and nothing in the log. There is no
//! `serde(alias)` for component type names here yet, and until there is,
//! the honest move is to leave the path alone and say why.
//!
//! # What used to live here
//!
//! `TestingPlugin`, which registered the component and scheduled its
//! system only when the feature was on. [`EcsPlugin`](crate::plugin) does
//! both unconditionally now. The `testing` feature still exists and is
//! empty — projects name it in their manifests, and removing it would
//! break their builds for no gain.

pub mod spin;
