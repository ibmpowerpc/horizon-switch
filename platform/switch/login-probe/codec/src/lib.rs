//! Bounded, allocation-free Java 26.3 (protocol 777) login codec for the
//! CraftRustNX Switch login probe.
//!
//! The crate is both the wire codec and its exact C ABI
//! (`libcraftrustnx_login_codec.a`, `../include/login_codec.h`). It only
//! *describes* the login exchange: it never connects, authenticates, replies,
//! acknowledges configuration or implements play. All buffers are caller-owned.
//!
//! - [`encode_login_start`] writes two complete TCP frames: the handshake
//!   (next state 2 = login) and LoginStart (id 0, name, offline UUID v3).
//! - [`decode_login_frame`] parses one complete bounded server frame from the
//!   login state, with or without the compression envelope, and validates one
//!   structurally complete event.
//!
//! `no_std`, no floats, no allocation: the only dependency is `miniz_oxide`
//! (no-alloc `inflate::core`) for the zlib body of the compression envelope.
#![no_std]

#[cfg(test)]
#[macro_use]
extern crate std;

pub mod decode;
pub mod encode;
pub mod md5;
pub mod wire;

pub mod ffi;

pub use decode::{decode_login_frame, Decoded, Event};
pub use encode::encode_login_start;
pub use ffi::{craftrustnx_login_decode, craftrustnx_login_start, LpEvent};

/// Protocol version this codec pins: Java 26.3.
pub const PROTOCOL: i32 = 777;

/// `LP_MAX_BODY`: largest packet body (packet id included) a frame may carry.
pub const MAX_BODY: usize = 65_536;
/// `LP_MAX_WIRE`: largest complete frame on the wire, body plus a five-byte
/// (widest) outer length prefix.
pub const MAX_WIRE: usize = 65_541;

/// Longest host accepted in the handshake (1..=253 graphic ASCII bytes).
pub const MAX_HOST: usize = 253;
/// Longest username accepted in LoginStart (1..=16 ASCII alphanumerics or `_`).
pub const MAX_NAME: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// A pointer, length or scalar argument is out of contract.
    BadArgument,
    /// A caller buffer cannot hold the frame body.
    OutputSmall,
    /// The outer frame is not complete yet: buffer more bytes and retry.
    NeedMore,
    /// A complete frame, envelope or packet body is malformed.
    BadFrame,
    /// A structurally valid LoginFinished whose profile breaks the limits.
    BadProfile,
    /// A structurally valid packet this diagnostic does not model.
    Unsupported,
}
