//! Decoding of one complete play-state body for the Switch probe's
//! [`play_codec.h`](../../../include/play_codec.h) ABI.
//!
//! The caller has already removed the outer frame and the compression
//! envelope, so this module never reads a frame prefix, inflates or buffered
//! more than the body it is handed: the existing configuration decoder owns
//! stream caps and consumed bytes. It reuses the login codec's [`Reader`].
//!
//! What is parsed is exactly what the probe needs to report and answer:
//! `Login` (the join data the header mirrors into `struct pl_state`),
//! `PlayerPosition` (resolved and acknowledged, but never simulated),
//! `ChunkBatchFinished` (answered with a receipt, not a load claim) and the
//! small control packets the client must answer or surface. Everything else,
//! including `Respawn`, chunk contents and the level lists, stays deliberately
//! opaque: no registry is loaded, no world exists, no cookie is stored, no pack
//! is installed and no transfer is followed.

use crate::decode::finish;
use crate::wire::{varint_len, write_varint, Reader};
use crate::{Error, PLAY_DIMENSION_CAP, PLAY_MAX_REPLY};

pub const PL_OK: i32 = 0;
pub const PL_BAD_ARGUMENT: i32 = -1;
pub const PL_OUTPUT_SMALL: i32 = -2;
pub const PL_BAD_FRAME: i32 = -3;
pub const PL_UNSUPPORTED: i32 = -5;

/// `pl_kind` values, in header order.
pub const PL_METADATA: u32 = 1;
pub const PL_LOGIN: u32 = 2;
pub const PL_POSITION: u32 = 3;
pub const PL_KEEP_ALIVE: u32 = 4;
pub const PL_PING: u32 = 5;
pub const PL_CHUNK: u32 = 6;
pub const PL_BATCH_START: u32 = 7;
pub const PL_BATCH_END: u32 = 8;
pub const PL_COOKIE: u32 = 9;
pub const PL_RESOURCE_PACK: u32 = 10;
pub const PL_DISCONNECT: u32 = 11;
pub const PL_RECONFIGURE: u32 = 12;
pub const PL_TRANSFER: u32 = 13;
pub const PL_RESPAWN: u32 = 14;

/// Bit 0 of `pl_event.flags`: the pushed resource pack is required.
pub const PL_FLAG_PACK_REQUIRED: u32 = 1;

/// Vanilla clientbound `Login.levels` has no count limit; this diagnostic caps
/// it, like the configuration codec caps known packs.
const MAX_LEVELS: usize = 256;
/// Vanilla `Str` limit, in UTF-16 code units.
const MAX_STRING_UNITS: usize = 32_767;
/// Vanilla `ResourcePackPush.hash` limit (`StrN<40>`).
const MAX_HASH_UNITS: usize = 40;
/// Longest dimension identifier the fixed-size state can hold (bytes, no NUL).
const MAX_DIMENSION_BYTES: usize = PLAY_DIMENSION_CAP - 1;
/// `Relative.pack`: X, Y, Z, Y_ROT, X_ROT, DELTA_X/Y/Z, ROTATE_DELTA.
const RELATIVE_MASK: i32 = 0x1ff;
/// Position and rotation bits: the ones that need a prior pose to resolve.
const RELATIVE_POSE: i32 = 0x1f;

/// `struct pl_state`: exactly the fields, order and size of the C header. The
/// header zero-initializes it; only a successful `Login` or `PlayerPosition`
/// changes it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlState {
    /// IEEE binary64 position, one raw bit pattern per axis.
    pub position_bits: [u64; 3],
    /// IEEE binary32 yaw bits.
    pub yaw_bits: u32,
    /// IEEE binary32 pitch bits.
    pub pitch_bits: u32,
    pub player_id: i32,
    pub dimension_type: i32,
    pub game_mode: i32,
    pub teleport_id: i32,
    pub login_seen: u32,
    pub position_seen: u32,
    /// The dimension identifier bytes, NUL-padded (always terminated here).
    pub dimension: [u8; PLAY_DIMENSION_CAP],
}

/// `struct pl_event`: exactly the fields, order and size of the C header.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlEvent {
    pub kind: u32,
    pub packet_id: i32,
    /// Unframed reply body written to the caller's reply buffer, 0 when none.
    pub reply_length: u32,
    /// Payload offset in the supplied body, after the packet ID.
    pub payload_offset: u32,
    /// Payload length in the supplied body.
    pub payload_length: u32,
    /// Teleport ID (`PlayerPosition`), batch size (`ChunkBatchFinished`) or the
    /// chunk column x (`LevelChunkWithLight`).
    pub value: i32,
    /// The chunk column z (`LevelChunkWithLight`).
    pub aux: i32,
    /// [`PL_FLAG_PACK_REQUIRED`] for a resource-pack push.
    pub flags: u32,
}

/// The state fields one successful `Login` installs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginUpdate {
    pub player_id: i32,
    pub dimension_type: i32,
    pub game_mode: i32,
    pub dimension: [u8; PLAY_DIMENSION_CAP],
}

/// The state fields one successful `PlayerPosition` installs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoseUpdate {
    pub position_bits: [u64; 3],
    pub yaw_bits: u32,
    pub pitch_bits: u32,
    pub teleport_id: i32,
}

/// Which state fields a decoded body installs. `None` leaves the state alone.
// `LoginUpdate` carries the fixed 257-byte dimension field by value: this crate
// never allocates, so the stack copy is the point, not an oversight.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateChange {
    None,
    Login(LoginUpdate),
    Pose(PoseUpdate),
}

/// One decoded body: the event to report and the state change to apply after
/// the caller has accepted the reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayDecoded {
    pub event: PlEvent,
    pub change: StateChange,
}

/// A reply that cannot fit `PL_REPLY_BODY` is a packet this diagnostic cannot
/// answer (`BadFrame`); one that does not fit the caller's buffer is
/// `OutputSmall`. Both are checked before any reply byte or state field is
/// written.
fn reply_room(needed: usize, reply: &[u8]) -> Result<(), Error> {
    if needed > PLAY_MAX_REPLY {
        return Err(Error::BadFrame);
    }
    if reply.len() < needed {
        return Err(Error::OutputSmall);
    }
    Ok(())
}

fn bool_at(reader: &mut Reader<'_>) -> Result<bool, Error> {
    Ok(reader.u8()? != 0)
}

fn i32_be(reader: &mut Reader<'_>) -> Result<i32, Error> {
    let bytes = reader.bytes(4)?;
    Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn i64_be(reader: &mut Reader<'_>) -> Result<i64, Error> {
    let mut value = [0u8; 8];
    value.copy_from_slice(reader.bytes(8)?);
    Ok(i64::from_be_bytes(value))
}

fn f64_be(reader: &mut Reader<'_>) -> Result<f64, Error> {
    let mut value = [0u8; 8];
    value.copy_from_slice(reader.bytes(8)?);
    Ok(f64::from_be_bytes(value))
}

fn f32_be(reader: &mut Reader<'_>) -> Result<f32, Error> {
    let bytes = reader.bytes(4)?;
    Ok(f32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// `Login` (clientbound 50): every field is validated, the join data the
/// header mirrors is kept, and the level list only consumes its identifiers.
///
/// A structurally valid identifier longer than the fixed `dimension` field is
/// [`Error::Unsupported`] (this probe cannot report it), while invalid
/// identifiers, negative counts and distances, a game mode outside 0..=3 and a
/// previous game mode outside 0..=4 are [`Error::BadFrame`].
fn parse_login(reader: &mut Reader<'_>) -> Result<LoginUpdate, Error> {
    let player_id = i32_be(reader)?;
    bool_at(reader)?; // hardcore: reported nowhere, no world exists.

    let levels = reader.varint()?;
    if levels < 0 || levels as usize > MAX_LEVELS {
        return Err(Error::BadFrame);
    }
    for _ in 0..levels {
        reader.identifier()?; // Level names are never resolved as registries.
    }

    let max_players = reader.varint()?;
    let chunk_radius = reader.varint()?;
    let simulation_distance = reader.varint()?;
    if max_players < 0 || chunk_radius < 0 || simulation_distance < 0 {
        return Err(Error::BadFrame);
    }
    bool_at(reader)?; // reduced_debug_info
    bool_at(reader)?; // show_death_screen
    bool_at(reader)?; // limited_crafting

    // CommonPlayerSpawnInfo.
    let dimension_type = reader.varint()?;
    if dimension_type < 0 {
        return Err(Error::BadFrame);
    }
    let dimension = reader.identifier()?;
    if dimension.len() > MAX_DIMENSION_BYTES {
        // Structurally valid, but the fixed state field cannot carry it.
        return Err(Error::Unsupported);
    }
    i64_be(reader)?; // seed: not used, never a world seed claim.
    let game_mode = reader.varint()?;
    if !(0..=3).contains(&game_mode) {
        return Err(Error::BadFrame);
    }
    // OptVarInt: 0 none, otherwise the GameType + 1.
    let previous_game_type = reader.varint()?;
    // Raw optional code: 0 absent, 1..=4 encode game modes 0..=3.
    // Validate before subtracting: i32::MIN would overflow a debug build.
    if !(0..=4).contains(&previous_game_type) {
        return Err(Error::BadFrame);
    }
    bool_at(reader)?; // debug
    bool_at(reader)?; // flat
    if bool_at(reader)? {
        // Opt<GlobalPos>: identifier then the packed block position long.
        reader.identifier()?;
        reader.bytes(8)?;
    }
    reader.varint()?; // portal_cooldown
    reader.varint()?; // sea_level

    bool_at(reader)?; // online_mode
    bool_at(reader)?; // enforces_secure_chat
    finish(reader)?;

    let mut name = [0u8; PLAY_DIMENSION_CAP];
    name[..dimension.len()].copy_from_slice(dimension);
    Ok(LoginUpdate {
        player_id,
        dimension_type,
        game_mode,
        dimension: name,
    })
}

/// Resolve one axis or angle: an absolute field is itself, a relative one is
/// the prior value plus the wire delta, which must stay finite.
fn resolve(summand: bool, value: f64, prior: f64) -> Result<f64, Error> {
    if !summand {
        return Ok(value);
    }
    let sum = prior + value;
    if sum.is_finite() {
        Ok(sum)
    } else {
        Err(Error::BadFrame)
    }
}

fn resolve_f32(summand: bool, value: f32, prior: f32) -> Result<f32, Error> {
    if !summand {
        return Ok(value);
    }
    let sum = prior + value;
    if sum.is_finite() {
        Ok(sum)
    } else {
        Err(Error::BadFrame)
    }
}

/// `PlayerPosition` (clientbound 73): teleport id, position, delta, yaw, pitch
/// and the relative flags, acknowledged with the resolved full pose.
///
/// A relative position or angle needs a prior absolute teleport
/// ([`Error::Unsupported`] otherwise); relative deltas are accepted but never
/// simulated. Every number and every resolved value must be finite, the
/// teleport id non-negative and only [`RELATIVE_MASK`] bits may be set.
fn parse_position(
    reader: &mut Reader<'_>,
    state: &PlState,
    reply: &mut [u8],
) -> Result<(PoseUpdate, u32), Error> {
    let teleport_id = reader.varint()?;
    if teleport_id < 0 {
        return Err(Error::BadFrame);
    }
    let position = [f64_be(reader)?, f64_be(reader)?, f64_be(reader)?];
    let delta = [f64_be(reader)?, f64_be(reader)?, f64_be(reader)?];
    let yaw = f32_be(reader)?;
    let pitch = f32_be(reader)?;
    let relatives = i32_be(reader)?;
    finish(reader)?;

    if !position.iter().all(|value| value.is_finite())
        || !delta.iter().all(|value| value.is_finite())
        || !yaw.is_finite()
        || !pitch.is_finite()
    {
        return Err(Error::BadFrame);
    }
    if relatives < 0 || relatives & !RELATIVE_MASK != 0 {
        return Err(Error::BadFrame);
    }
    if relatives & RELATIVE_POSE != 0 && state.position_seen == 0 {
        // No absolute pose has been seen: a relative axis cannot be resolved.
        return Err(Error::Unsupported);
    }

    let prior = [
        f64::from_bits(state.position_bits[0]),
        f64::from_bits(state.position_bits[1]),
        f64::from_bits(state.position_bits[2]),
    ];
    let x = resolve(relatives & 0x01 != 0, position[0], prior[0])?;
    let y = resolve(relatives & 0x02 != 0, position[1], prior[1])?;
    let z = resolve(relatives & 0x04 != 0, position[2], prior[2])?;
    let new_yaw = resolve_f32(relatives & 0x08 != 0, yaw, f32::from_bits(state.yaw_bits))?;
    let new_pitch = resolve_f32(
        relatives & 0x10 != 0,
        pitch,
        f32::from_bits(state.pitch_bits),
    )?
    .clamp(-90.0, 90.0); // PositionMoveRotation.calculateAbsolute pitch.

    // Java 26.3's AcceptTeleportation carries the full pose back.
    let needed = 1 + varint_len(teleport_id) + 24 + 8;
    reply_room(needed, reply)?;
    reply[0] = 0; // Serverbound AcceptTeleportation.
    let mut pos = 1 + write_varint(teleport_id, &mut reply[1..])?;
    reply[pos..pos + 8].copy_from_slice(&x.to_be_bytes());
    pos += 8;
    reply[pos..pos + 8].copy_from_slice(&y.to_be_bytes());
    pos += 8;
    reply[pos..pos + 8].copy_from_slice(&z.to_be_bytes());
    pos += 8;
    reply[pos..pos + 4].copy_from_slice(&new_yaw.to_be_bytes());
    pos += 4;
    reply[pos..pos + 4].copy_from_slice(&new_pitch.to_be_bytes());

    Ok((
        PoseUpdate {
            position_bits: [x.to_bits(), y.to_bits(), z.to_bits()],
            yaw_bits: new_yaw.to_bits(),
            pitch_bits: new_pitch.to_bits(),
            teleport_id,
        },
        needed as u32,
    ))
}

/// Decode **one** complete, already-unframed and already-inflated play body.
///
/// `body` starts with the packet ID and is at most [`crate::PLAY_MAX_BODY`]
/// bytes.
/// The reply, when the packet has one, is an *unframed* body the caller frames
/// and ciphers exactly once; it never exceeds [`PLAY_MAX_REPLY`]. On any error
/// `reply` is untouched and the returned change is never applied, so the
/// caller's state and reply stay as they were.
///
/// A negative packet ID is [`Error::BadFrame`]; an ID above the last known
/// clientbound play ID (143) is [`Error::Unsupported`]; every known ID this
/// module does not model is opaque metadata.
pub fn decode_play_body(
    body: &[u8],
    state: &PlState,
    reply: &mut [u8],
) -> Result<PlayDecoded, Error> {
    let mut reader = Reader::new(body);
    let id = reader.varint()?;
    if id < 0 {
        return Err(Error::BadFrame);
    }
    let payload_offset = reader.position();
    let mut event = PlEvent {
        kind: PL_METADATA,
        packet_id: id,
        reply_length: 0,
        payload_offset: payload_offset as u32,
        payload_length: (body.len() - payload_offset) as u32,
        value: 0,
        aux: 0,
        flags: 0,
    };

    let change = match id {
        // BundleDelimiter: an empty body that only brackets a bundle.
        0 => {
            finish(&reader)?;
            StateChange::None
        }
        // ChunkBatchFinished: a non-negative declared size, answered with the
        // receipt (CHUNK_BATCH_RECEIVED, one float), offering conservative
        // one-chunk-per-tick credit. It is not measured processing speed or a
        // load: no chunk is applied by this probe.
        11 => {
            let size = reader.varint()?;
            if size < 0 {
                return Err(Error::BadFrame);
            }
            finish(&reader)?;
            reply_room(5, reply)?;
            reply[0] = 11; // Serverbound ChunkBatchReceived.
            reply[1..5].copy_from_slice(&1.0f32.to_be_bytes());
            event.kind = PL_BATCH_END;
            event.value = size;
            event.reply_length = 5;
            StateChange::None
        }
        // ChunkBatchStart: an empty body.
        12 => {
            finish(&reader)?;
            event.kind = PL_BATCH_START;
            StateChange::None
        }
        // CookieRequest: an identifier key, answered for that key with no
        // payload. Nothing is stored.
        21 => {
            let key = reader.identifier()?;
            finish(&reader)?;
            let needed = 1 + varint_len(key.len() as i32) + key.len() + 1;
            reply_room(needed, reply)?;
            reply[0] = 21; // Serverbound CookieResponse.
            let pos = 1 + write_varint(key.len() as i32, &mut reply[1..])?;
            reply[pos..pos + key.len()].copy_from_slice(key);
            reply[pos + key.len()] = 0; // Opt<Bytes>: absent.
            event.kind = PL_COOKIE;
            event.reply_length = needed as u32;
            StateChange::None
        }
        // Disconnect: a raw NBT component, bounded and kept opaque. It is not
        // parsed as JSON and no message text is claimed.
        32 => {
            if reader.rest().is_empty() {
                return Err(Error::BadFrame);
            }
            event.kind = PL_DISCONNECT;
            StateChange::None
        }
        // KeepAlive: exactly one long, echoed back byte for byte.
        45 => {
            let value = reader.bytes(8)?;
            finish(&reader)?;
            reply_room(9, reply)?;
            reply[0] = 28; // Serverbound KeepAlive.
            reply[1..9].copy_from_slice(value);
            event.kind = PL_KEEP_ALIVE;
            event.reply_length = 9;
            StateChange::None
        }
        // LevelChunkWithLight: only the column header is read; the chunk and
        // light data stay opaque and must be present.
        46 => {
            let x = i32_be(&mut reader)?;
            let z = i32_be(&mut reader)?;
            if reader.remaining() == 0 {
                return Err(Error::BadFrame);
            }
            event.kind = PL_CHUNK;
            event.value = x;
            event.aux = z;
            StateChange::None
        }
        // Login: the join data, validated in full, mirrored into the state.
        50 => {
            let update = parse_login(&mut reader)?;
            event.kind = PL_LOGIN;
            // No reply: the caller reports the login and decides what follows.
            StateChange::Login(update)
        }
        // Ping: exactly one int, answered by Pong with the same bytes.
        62 => {
            let value = reader.bytes(4)?;
            finish(&reader)?;
            reply_room(5, reply)?;
            reply[0] = 45; // Serverbound Pong.
            reply[1..5].copy_from_slice(value);
            event.kind = PL_PING;
            event.reply_length = 5;
            StateChange::None
        }
        // PlayerPosition: resolved and acknowledged; never simulated.
        73 => {
            let (pose, reply_length) = parse_position(&mut reader, state, reply)?;
            event.kind = PL_POSITION;
            event.value = pose.teleport_id;
            event.reply_length = reply_length;
            StateChange::Pose(pose)
        }
        // ResourcePackPush: uuid, URL, hash, required, optional prompt NBT. The
        // reply always declines: nothing is downloaded or installed.
        82 => {
            let uuid = reader.uuid()?;
            reader.string(MAX_STRING_UNITS)?; // URL
            reader.string(MAX_HASH_UNITS)?; // hash
            let required = bool_at(&mut reader)?;
            if bool_at(&mut reader)? {
                // The prompt is a full NBT tag; it must carry bytes but stays
                // opaque (not parsed or interpreted).
                if reader.rest().is_empty() {
                    return Err(Error::BadFrame);
                }
            } else {
                finish(&reader)?;
            }
            reply_room(18, reply)?;
            reply[0] = 50; // Serverbound ResourcePack.
            reply[1..17].copy_from_slice(&uuid);
            reply[17] = 1; // Declined.
            event.kind = PL_RESOURCE_PACK;
            event.reply_length = 18;
            event.flags = if required { PL_FLAG_PACK_REQUIRED } else { 0 };
            StateChange::None
        }
        // Respawn: a full spawn description this probe does not simulate. The
        // event says a continuation would be needed; the body stays opaque.
        84 => {
            event.kind = PL_RESPAWN;
            StateChange::None
        }
        // StartConfiguration: an empty body asking for a reconfiguration.
        120 => {
            finish(&reader)?;
            event.kind = PL_RECONFIGURE;
            StateChange::None
        }
        // Transfer: host and port, validated and reported, never followed.
        132 => {
            let host = reader.string(MAX_STRING_UNITS)?;
            let port = reader.varint()?;
            finish(&reader)?;
            if host.is_empty() || !(1..=65_535).contains(&port) {
                return Err(Error::BadFrame);
            }
            event.kind = PL_TRANSFER;
            StateChange::None
        }
        // Every other known clientbound play ID (including 63 PongResponse,
        // movement, entity, inventory, chat, registry and UI traffic) stays
        // opaque metadata: counted, never parsed, installed or claimed.
        1..=143 => StateChange::None,
        _ => return Err(Error::Unsupported),
    };

    Ok(PlayDecoded { event, change })
}
