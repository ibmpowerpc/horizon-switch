//! Decoding of one complete configuration-state (or play-state) frame, and
//! framing of one small client acknowledgement, for the Switch probe's
//! [`configuration_codec.h`](../../../include/configuration_codec.h) ABI.
//!
//! The outer frame and compression envelope are the login codec's, reused with
//! [`FrameLimits::CONFIGURATION`](crate::decode::FrameLimits). Everything that
//! is not a small control reply stays deliberately opaque: registry data, tags,
//! NBT payloads and play packets are counted, never parsed, installed or
//! claimed. Nothing here allocates, logs, connects, stores a cookie, downloads a
//! pack or follows a transfer.

use crate::decode::{finish, frame_body, FrameLimits};
use crate::wire::{varint_len, write_varint, Reader};
use crate::{Error, CONFIG_MAX_CLIENT_BODY, CONFIG_MAX_REPLY};

pub const CF_OK: i32 = 0;
pub const CF_NEED_MORE: i32 = 1;
pub const CF_BAD_ARGUMENT: i32 = -1;
pub const CF_OUTPUT_SMALL: i32 = -2;
pub const CF_BAD_FRAME: i32 = -3;
pub const CF_UNSUPPORTED: i32 = -5;

/// `CF_PHASE_CONFIGURATION`: clientbound configuration packets.
pub const CF_PHASE_CONFIGURATION: u32 = 0;
/// `CF_PHASE_PLAY`: clientbound play packets, counted only.
pub const CF_PHASE_PLAY: u32 = 1;

/// `cf_kind` values, in header order.
pub const CF_METADATA: u32 = 1;
pub const CF_FINISH: u32 = 2;
pub const CF_KEEP_ALIVE: u32 = 3;
pub const CF_PING: u32 = 4;
pub const CF_KNOWN_PACKS: u32 = 5;
pub const CF_COOKIE_REQUEST: u32 = 6;
pub const CF_RESOURCE_PACK: u32 = 7;
pub const CF_DISCONNECT: u32 = 8;
pub const CF_USER_ACTION: u32 = 9;
pub const CF_TRANSFER: u32 = 10;
pub const CF_PLAY_PACKET: u32 = 11;

/// Bit 0 of `cf_event.flags`: the pushed resource pack is required.
pub const CF_FLAG_PACK_REQUIRED: u32 = 1;

/// Vanilla `SelectKnownPacks.packs` has no count limit; this diagnostic caps it.
const MAX_KNOWN_PACKS: usize = 256;
/// Vanilla `Str` limit, in UTF-16 code units.
const MAX_STRING_UNITS: usize = 32_767;
/// Vanilla `ResourcePackPush.hash` limit (`StrN<40>`).
const MAX_HASH_UNITS: usize = 40;

/// `struct cf_event`: exactly the fields, order and size of the C header.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CfEvent {
    pub kind: u32,
    /// Wire bytes this frame occupies, so a caller can consume a buffered tail.
    pub consumed: u32,
    /// Uncompressed packet body length, packet ID included.
    pub body_length: u32,
    /// The wire packet ID this event was decoded from.
    pub packet_id: i32,
    /// Payload offset in the caller's scratch, after the packet ID.
    pub payload_offset: u32,
    /// Payload length in the caller's scratch.
    pub payload_length: u32,
    /// Unframed reply body written to the caller's reply buffer, 0 when none.
    pub reply_length: u32,
    /// [`CF_FLAG_PACK_REQUIRED`] for a resource-pack push.
    pub flags: u32,
}

/// A reply that cannot fit `CF_REPLY_BODY` is a frame this diagnostic cannot
/// answer (`BadFrame`); one that does not fit the caller's buffer is
/// `OutputSmall`. Both are checked before a byte is serialized.
fn reply_room(needed: usize, reply: &[u8]) -> Result<(), Error> {
    if needed > CONFIG_MAX_REPLY {
        return Err(Error::BadFrame);
    }
    if reply.len() < needed {
        return Err(Error::OutputSmall);
    }
    Ok(())
}

/// Configuration phase: validate one known control packet and fill its reply.
///
/// IDs 1, 6, 7, 8, 10, 11, 13, 14, 16, 17, 18 and 19 (custom payload, registry
/// data, tags, feature flags, links, dialogs and the rest) are `CF_METADATA`:
/// only `body_length`, `packet_id` and the payload counters describe them. Their
/// payload is intentionally opaque, not NBT-validated or installed.
fn parse_configuration(body: &[u8], consumed: usize, reply: &mut [u8]) -> Result<CfEvent, Error> {
    let mut reader = Reader::new(body);
    let id = reader.varint()?;
    let payload_offset = reader.position();
    let mut event = CfEvent {
        kind: CF_METADATA,
        consumed: consumed as u32,
        body_length: body.len() as u32,
        packet_id: id,
        payload_offset: payload_offset as u32,
        payload_length: (body.len() - payload_offset) as u32,
        reply_length: 0,
        flags: 0,
    };

    match id {
        // CookieRequest: an identifier key. The reply is a CookieResponse for
        // that key with no payload: this probe stores no cookie.
        0 => {
            let key = reader.identifier()?;
            finish(&reader)?;
            let needed = 1 + varint_len(key.len() as i32) + key.len() + 1;
            reply_room(needed, reply)?;
            reply[0] = 1; // Serverbound CookieResponse.
            let pos = 1 + write_varint(key.len() as i32, &mut reply[1..])?;
            reply[pos..pos + key.len()].copy_from_slice(key);
            reply[pos + key.len()] = 0; // Opt<Bytes>: absent.
            event.kind = CF_COOKIE_REQUEST;
            event.reply_length = needed as u32;
        }
        // Opaque common/registry traffic. Deliberately no field parsing here:
        // these packets carry registries, tags, NBT or components this probe
        // never validates, installs or claims.
        1 | 6 | 7 | 8 | 10 | 11 | 13 | 14 | 16 | 17 | 18 | 19 => {}
        // Disconnect: a raw NBT component, bounded by the body and kept opaque.
        // It is not parsed as JSON and no message text is claimed.
        2 => {
            if reader.rest().is_empty() {
                return Err(Error::BadFrame);
            }
            event.kind = CF_DISCONNECT;
        }
        // FinishConfiguration: an empty body. The reply acknowledges it.
        3 => {
            finish(&reader)?;
            reply_room(1, reply)?;
            reply[0] = 3; // Serverbound FinishConfiguration.
            event.kind = CF_FINISH;
            event.reply_length = 1;
        }
        // KeepAlive: exactly one long, echoed back byte for byte.
        4 => {
            let value = reader.bytes(8)?;
            finish(&reader)?;
            reply_room(9, reply)?;
            reply[0] = 4; // Serverbound KeepAlive.
            reply[1..9].copy_from_slice(value);
            event.kind = CF_KEEP_ALIVE;
            event.reply_length = 9;
        }
        // Ping: exactly one int, answered by Pong with the same value.
        5 => {
            let value = reader.bytes(4)?;
            finish(&reader)?;
            reply_room(5, reply)?;
            reply[0] = 5; // Serverbound Pong.
            reply[1..5].copy_from_slice(value);
            event.kind = CF_PING;
            event.reply_length = 5;
        }
        // ResourcePackPush: uuid, URL, hash, required, optional prompt NBT. The
        // reply always declines the pack: nothing is downloaded or applied.
        9 => {
            let uuid = reader.uuid()?;
            reader.string(MAX_STRING_UNITS)?; // URL
            reader.string(MAX_HASH_UNITS)?; // hash
            let required = reader.u8()? != 0;
            if reader.u8()? != 0 {
                // The prompt is a full NBT tag; it must carry bytes but stays
                // opaque (not parsed or interpreted).
                if reader.rest().is_empty() {
                    return Err(Error::BadFrame);
                }
            } else {
                finish(&reader)?;
            }
            reply_room(18, reply)?;
            reply[0] = 6; // Serverbound ResourcePack.
            reply[1..17].copy_from_slice(&uuid);
            reply[17] = 1; // Declined.
            event.kind = CF_RESOURCE_PACK;
            event.reply_length = 18;
            event.flags = if required { CF_FLAG_PACK_REQUIRED } else { 0 };
        }
        // Transfer: host and port, reported without following anything.
        12 => {
            reader.string(MAX_STRING_UNITS)?;
            reader.varint()?;
            finish(&reader)?;
            event.kind = CF_TRANSFER;
        }
        // SelectKnownPacks: 0..=256 triples of short strings. The whole body is
        // validated; the reply always claims no packs, because this probe has
        // none, whatever the server offered.
        15 => {
            let count = reader.varint()?;
            if count < 0 || count as usize > MAX_KNOWN_PACKS {
                return Err(Error::BadFrame);
            }
            for _ in 0..count {
                reader.string(MAX_STRING_UNITS)?;
                reader.string(MAX_STRING_UNITS)?;
                reader.string(MAX_STRING_UNITS)?;
            }
            finish(&reader)?;
            reply_room(2, reply)?;
            reply[0] = 7; // Serverbound SelectKnownPacks.
            reply[1] = 0; // Empty list: no known packs.
            event.kind = CF_KNOWN_PACKS;
            event.reply_length = 2;
        }
        // CodeOfConduct: text for a human to read. It is never accepted
        // automatically; the event only reports that the server sent it.
        20 => {
            reader.string(MAX_STRING_UNITS)?;
            finish(&reader)?;
            event.kind = CF_USER_ACTION;
        }
        _ => return Err(Error::Unsupported),
    }
    Ok(event)
}

/// Decode **one** complete clientbound frame from `data` into `scratch` and
/// write the unframed reply body into `reply` when the packet has one.
///
/// `compression` is `-1` for raw frames or the announced threshold; `phase` is
/// [`CF_PHASE_CONFIGURATION`] or [`CF_PHASE_PLAY`]. Outer and inflated bodies are
/// bounded by [`FrameLimits::CONFIGURATION`]: an oversized length prefix or
/// envelope is [`Error::BadFrame`] before inflation. An oversized outer length
/// is rejected from its prefix; an incomplete outer frame is [`Error::NeedMore`]. A compressed stream must end
/// exactly at the end of its frame, inflate to exactly its declared length and
/// pass its Adler-32 trailer.
///
/// In the play phase only the packet ID is read: the payload is reported as an
/// opaque region and no gameplay schema or world state is claimed.
pub fn decode_configuration_frame(
    data: &[u8],
    compression: i32,
    phase: u32,
    scratch: &mut [u8],
    reply: &mut [u8],
) -> Result<CfEvent, Error> {
    if phase > CF_PHASE_PLAY {
        return Err(Error::BadArgument);
    }
    let (consumed, body_len) = frame_body(data, compression, scratch, &FrameLimits::CONFIGURATION)?;
    let body = &scratch[..body_len];
    if phase == CF_PHASE_PLAY {
        let mut reader = Reader::new(body);
        let id = reader.varint()?;
        if id < 0 {
            return Err(Error::BadFrame);
        }
        let payload_offset = reader.position();
        return Ok(CfEvent {
            kind: CF_PLAY_PACKET,
            consumed: consumed as u32,
            body_length: body_len as u32,
            packet_id: id,
            payload_offset: payload_offset as u32,
            payload_length: (body_len - payload_offset) as u32,
            reply_length: 0,
            flags: 0,
        });
    }
    parse_configuration(body, consumed, reply)
}

/// Adler-32 of `data`, as the zlib trailer stores it (RFC 1950).
fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    // Client bodies are at most 1024 bytes, so two modulos per byte stay cheap
    // and cannot overflow.
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

/// Frame one client body of 1..=1024 bytes.
///
/// `compression` is `-1` for a raw frame, or the announced threshold: a body
/// below it goes into an uncompressed envelope, and a body at or above it into a
/// compressed envelope. This crate has no compressor dependency, so the stream
/// is a single stored DEFLATE block (BTYPE 00) with an exact Adler-32 trailer: a
/// valid zlib stream that any inflater, Python's `zlib` included, accepts.
///
/// Every length, including the complete wire frame, is checked against `out`
/// before a byte is written, so a failure leaves `out` unchanged.
pub fn encode_client_frame(body: &[u8], compression: i32, out: &mut [u8]) -> Result<usize, Error> {
    if body.is_empty() || body.len() > CONFIG_MAX_CLIENT_BODY || compression < -1 {
        return Err(Error::BadArgument);
    }
    let body_len = body.len();
    if compression < 0 {
        let total = varint_len(body_len as i32) + body_len;
        if out.len() < total {
            return Err(Error::OutputSmall);
        }
        let pos = write_varint(body_len as i32, out)?;
        out[pos..pos + body_len].copy_from_slice(body);
        return Ok(total);
    }
    if body_len < compression as usize {
        // Uncompressed envelope: data length 0 then the body, as Vanilla writes
        // any body that stays below the threshold.
        let frame = 1 + body_len;
        let total = varint_len(frame as i32) + frame;
        if out.len() < total {
            return Err(Error::OutputSmall);
        }
        let mut pos = write_varint(frame as i32, out)?;
        out[pos] = 0;
        pos += 1;
        out[pos..pos + body_len].copy_from_slice(body);
        return Ok(total);
    }

    // Compressed envelope: declared length, then a zlib stream of
    // 2 header + 5 stored-block header + body + 4 Adler-32 bytes.
    let zlib = body_len + 11;
    let frame = varint_len(body_len as i32) + zlib;
    let total = varint_len(frame as i32) + frame;
    if out.len() < total {
        return Err(Error::OutputSmall);
    }
    let mut pos = write_varint(frame as i32, out)?;
    pos += write_varint(body_len as i32, &mut out[pos..])?;
    // zlib header: CMF 0x78, FLG 0x01 (FCHECK valid, no dictionary).
    out[pos] = 0x78;
    out[pos + 1] = 0x01;
    pos += 2;
    // One final stored block: BFINAL = 1, BTYPE = 00, then LEN and ~LEN.
    out[pos] = 1;
    pos += 1;
    let len = body_len as u16;
    out[pos..pos + 2].copy_from_slice(&len.to_le_bytes());
    pos += 2;
    out[pos..pos + 2].copy_from_slice(&(!len).to_le_bytes());
    pos += 2;
    out[pos..pos + body_len].copy_from_slice(body);
    pos += body_len;
    out[pos..pos + 4].copy_from_slice(&adler32(body).to_be_bytes());
    Ok(pos + 4)
}
