//! Offline, host-side Minecraft Java blockstate selection for Switch model
//! preparation.
//!
//! This crate turns one blockstate definition (`variants` or `multipart`) plus a
//! block's property values into the model options a preparer has to have on
//! disk, then bakes the deduplicated `model`/rotation requests into quads with
//! the existing `horizon-assets` resolver and baker. It never renders, never
//! reads game files, never picks randomly among weighted options and never
//! guesses atlases or lighting: the caller keeps every alternative.

#![forbid(unsafe_code)]

pub mod geometry;
pub mod selector;

pub use geometry::{
    bake_requested, BakeError, BakeRequest, BakedModel, MAX_DECODED_SOURCE_BYTES, MAX_MODEL_BYTES,
    MAX_PARENT_DEPTH, MAX_QUADS, MAX_REACHABLE_MODELS, MAX_REQUESTS,
};
pub use selector::{select, Error, ModelOption, Part};
