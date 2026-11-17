//! Offline, host-side Minecraft Java blockstate selection for Switch model
//! preparation.
//!
//! This crate turns one blockstate definition (`variants` or `multipart`) plus a
//! block's property values into the model options a preparer has to have on
//! disk. It never resolves model geometry, never renders, never reads game
//! files and never picks randomly among weighted options: the caller keeps every
//! alternative.

#![forbid(unsafe_code)]

pub mod selector;

pub use selector::{select, Error, ModelOption, Part};
