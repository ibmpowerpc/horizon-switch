//! Offline, host-side Minecraft Java blockstate selection for Switch model
//! preparation.
//!
//! This crate turns one blockstate definition (`variants` or `multipart`) plus a
//! block's property values into the model options a preparer has to have on
//! disk, then bakes the deduplicated `model`/rotation requests into quads with
//! the existing `horizon-assets` resolver and baker. The atlas module packs
//! verified static texture bytes; `scene_inputs` loads bounded, pinned resource
//! manifests for the host examples. No module starts Minecraft, renders, picks
//! randomly among weighted options or guesses lighting.

#![forbid(unsafe_code)]

pub mod atlas;
pub mod bank;
pub mod geometry;
pub mod scene_inputs;
pub mod section;
pub mod selector;

pub use atlas::{
    build_static_atlas, Atlas, AtlasError, AtlasLimits, SpriteRect, GUTTER, MAX_SOURCE_BYTES,
    MAX_SOURCE_DIMENSION,
};
pub use geometry::{
    bake_requested, BakeError, BakeRequest, BakedModel, MAX_DECODED_SOURCE_BYTES, MAX_MODEL_BYTES,
    MAX_PARENT_DEPTH, MAX_QUADS, MAX_REACHABLE_MODELS, MAX_REQUESTS,
};
pub use selector::{select, Error, ModelOption, Part};
