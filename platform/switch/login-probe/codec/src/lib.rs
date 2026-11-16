//! Bounded, allocation-free Java 26.3 (protocol 777) login codec for the
//! CraftRustNX Switch login probe.
//!
//! The crate is both the wire codec and its exact C ABI
//! (`libcraftrustnx_login_codec.a`, `../include/login_codec.h`). It only
//! *describes* the login, configuration and first play packets: it never
//! connects, authenticates, sends packets on its own, acknowledges
//! configuration or simulates play. All buffers are caller-owned.
//!
//! - [`encode_login_start`] writes two complete TCP frames: the handshake
//!   (next state 2 = login) and LoginStart (id 0, name, offline UUID v3).
//! - [`encode_login_key`] writes EncryptionResponse (id 1) for the encrypted
//!   secret and challenge a validated Hello asks for.
//! - [`decode_login_frame`] parses one complete bounded server frame from the
//!   login state, with or without the compression envelope, and validates one
//!   structurally complete event.
//! - [`decode_play_body`] parses one complete, already-unframed play body and
//!   answers the small client controls; it never simulates a world.
//!
//! `no_std`, no allocation: the only dependency is `miniz_oxide` (no-alloc
//! `inflate::core`) for the zlib body of the compression envelope.
#![no_std]

#[cfg(test)]
#[macro_use]
extern crate std;

pub mod configuration;
pub mod configuration_ffi;
pub mod decode;
pub mod encode;
pub mod md5;
pub mod nbt;
pub mod play;
pub mod wire;
pub mod world;

pub mod ffi;
pub mod play_ffi;
pub mod world_ffi;

pub use configuration::{
    decode_configuration_frame, encode_client_frame, CfEvent, CF_BAD_ARGUMENT, CF_BAD_FRAME,
    CF_COOKIE_REQUEST, CF_DISCONNECT, CF_FINISH, CF_KEEP_ALIVE, CF_KNOWN_PACKS, CF_METADATA,
    CF_NEED_MORE, CF_OK, CF_OUTPUT_SMALL, CF_PHASE_CONFIGURATION, CF_PHASE_PLAY, CF_PING,
    CF_PLAY_PACKET, CF_RESOURCE_PACK, CF_TRANSFER, CF_UNSUPPORTED, CF_USER_ACTION,
};
pub use configuration_ffi::{craftrustnx_configuration_decode, craftrustnx_configuration_frame};
pub use decode::{decode_login_frame, Decoded, Event, Region};
pub use encode::{encode_login_key, encode_login_start};
pub use ffi::{craftrustnx_login_decode, craftrustnx_login_key, craftrustnx_login_start, LpEvent};
pub use play::{
    decode_play_body, LoginUpdate, PlEvent, PlState, PlayDecoded, PoseUpdate, StateChange,
    PL_BAD_ARGUMENT, PL_BAD_FRAME, PL_BATCH_END, PL_BATCH_START, PL_CHUNK, PL_COOKIE,
    PL_DISCONNECT, PL_FLAG_PACK_REQUIRED, PL_KEEP_ALIVE, PL_LOGIN, PL_METADATA, PL_OK,
    PL_OUTPUT_SMALL, PL_PING, PL_POSITION, PL_RECONFIGURE, PL_RESOURCE_PACK, PL_RESPAWN,
    PL_TRANSFER, PL_UNSUPPORTED,
};
pub use play_ffi::craftrustnx_play_decode_body;
pub use world::{
    decode_world_body, WcDimension, WcEvent, WcState, WorldDecoded, WorldError, WC_BAD_ARGUMENT,
    WC_BAD_FRAME, WC_BIOMES, WC_CHUNK, WC_CONFIGURATION, WC_DIMENSIONS, WC_LIGHT, WC_LIMIT,
    WC_MAX_BODY, WC_MAX_DIMENSIONS, WC_MAX_SECTIONS, WC_METADATA, WC_OK, WC_PLAY, WC_UNSUPPORTED,
};
pub use world_ffi::craftrustnx_world_decode_body;

/// Protocol version this codec pins: Java 26.3.
pub const PROTOCOL: i32 = 777;

/// `LP_MAX_BODY`: largest packet body (packet id included) a frame may carry.
pub const MAX_BODY: usize = 65_536;
/// `LP_MAX_WIRE`: largest complete frame on the wire, body plus a five-byte
/// (widest) outer length prefix.
pub const MAX_WIRE: usize = 65_541;

/// `CF_MAX_BODY`: largest uncompressed configuration packet body (packet ID
/// included), after an envelope is inflated.
pub const CONFIG_MAX_BODY: usize = 8_388_608;
/// `CF_MAX_FRAME`: largest outer configuration frame body, envelope prefix
/// included.
pub const CONFIG_MAX_FRAME: usize = 2_097_151;
/// `CF_MAX_WIRE`: largest complete configuration frame, outer prefix included.
pub const CONFIG_MAX_WIRE: usize = 2_097_156;
/// `CF_REPLY_BODY`: largest reply body this diagnostic writes.
pub const CONFIG_MAX_REPLY: usize = 1024;
/// Largest client body [`encode_client_frame`] frames.
pub const CONFIG_MAX_CLIENT_BODY: usize = 1024;

/// `PL_MAX_BODY`: largest complete play packet body (packet ID included) the
/// play decoder accepts. Framing and inflation are the configuration
/// decoder's job; this is the already-uncompressed body.
pub const PLAY_MAX_BODY: usize = 8_388_608;
/// `PL_REPLY_BODY`: largest unframed reply body the play decoder writes.
pub const PLAY_MAX_REPLY: usize = 1024;
/// `PL_DIMENSION_CAP`: the state's dimension identifier field, NUL included.
pub const PLAY_DIMENSION_CAP: usize = 257;

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
