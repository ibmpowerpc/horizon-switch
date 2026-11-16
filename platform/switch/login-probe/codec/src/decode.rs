//! Decoding of one complete clientbound login frame: the outer length prefix,
//! the optional compression envelope and the packet body, with every field
//! structurally validated before an event is returned.

use crate::wire::Reader;
use crate::{
    Error, CONFIG_MAX_BODY, CONFIG_MAX_FRAME, CONFIG_MAX_WIRE, MAX_BODY, MAX_NAME, MAX_WIRE,
};

/// Vanilla `GameProfile` property name limit (`StrN<64>`).
pub const MAX_PROPERTY_NAME: usize = 64;
/// Vanilla property signature limit (`Opt<StrN<1024>>`).
pub const MAX_SIGNATURE: usize = 1024;
/// Vanilla `Hello.serverId` limit (`StrN<20>`).
pub const MAX_SERVER_ID: usize = 20;
/// Properties accepted in one `LoginFinished` profile before it is rejected as
/// a profile error; Vanilla has no count limit, so a diagnostic caps it.
pub const MAX_PROPERTIES: usize = 64;
/// Vanilla `Disconnect.reason` limit (`StrN<262144>`, itself bounded by the body).
pub const MAX_DISCONNECT_JSON: usize = 262_144;
/// Vanilla `Str` limit, used for unbounded fields such as property values.
pub const MAX_STRING: usize = 32_767;

/// A validated region of the caller's scratch buffer, which holds the whole
/// packet body, packet ID included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    /// Byte offset in the scratch buffer.
    pub offset: u32,
    /// Region length in bytes; every byte is inside the packet body.
    pub length: u32,
}

/// The validated content of one clientbound login packet. Values that the C
/// event struct cannot hold stay in the caller's scratch buffer; use
/// [`Decoded::data_offset`] and [`Decoded::data_length`] to reach them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Packet 0: a JSON chat component, valid UTF-8, not syntax-parsed.
    Disconnect,
    /// Packet 1: the server asked for encryption. `should_authenticate` is
    /// reported as sent; the caller decides how to proceed. The regions are the
    /// server's DER public key and its challenge, validated as byte arrays
    /// inside the body and not given any size limit beyond it.
    Hello {
        should_authenticate: bool,
        public_key: Region,
        challenge: Region,
    },
    /// Packet 2: `GameProfile` (UUID, name, properties) plus the session UUID.
    Success {
        uuid: [u8; 16],
        /// ASCII, NUL-padded to the C event's 17 bytes.
        name: [u8; 17],
        session: [u8; 16],
        properties: u32,
    },
    /// Packet 3: the threshold for the compression envelope that follows.
    Compression { threshold: i32 },
    /// Packet 4: a login plugin query; the payload is `data_offset..`.
    CustomQuery { transaction: i32 },
    /// Packet 5: a cookie request; the key is in the payload region.
    CookieRequest,
}

/// One decoded frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decoded {
    pub event: Event,
    /// Bytes of `data` this frame occupies; a following frame is untouched.
    pub consumed: usize,
    /// Offset in `scratch` of the packet payload, after the packet ID.
    pub data_offset: usize,
    /// Length of that payload (`body len - id varint len`).
    pub data_length: usize,
}

/// The outer-frame and envelope limits of one protocol state. Login and
/// configuration differ only in these numbers, so both share the same framing
/// code.
#[derive(Debug, Clone, Copy)]
pub struct FrameLimits {
    /// Largest uncompressed packet body (packet ID included).
    pub max_body: usize,
    /// Largest outer frame body, envelope prefix included.
    pub max_frame: usize,
    /// Largest complete frame on the wire, outer prefix included.
    pub max_wire: usize,
}

impl FrameLimits {
    /// Login: `LP_MAX_BODY` / `LP_MAX_WIRE` (`login_codec.h`).
    pub const LOGIN: Self = Self {
        max_body: MAX_BODY,
        max_frame: MAX_BODY,
        max_wire: MAX_WIRE,
    };
    /// Configuration: `CF_MAX_BODY` / `CF_MAX_FRAME` / `CF_MAX_WIRE`
    /// (`configuration_codec.h`).
    pub const CONFIGURATION: Self = Self {
        max_body: CONFIG_MAX_BODY,
        max_frame: CONFIG_MAX_FRAME,
        max_wire: CONFIG_MAX_WIRE,
    };
}

/// Read the outer frame length. A short read of a still-possible `VarInt` is
/// [`Error::NeedMore`]; an impossible one is [`Error::BadFrame`].
fn outer_length(data: &[u8]) -> Result<(usize, usize), Error> {
    let mut value = 0u32;
    for i in 0..5 {
        let Some(&byte) = data.get(i) else {
            return Err(Error::NeedMore);
        };
        if i == 4 && byte & 0xf0 != 0 {
            return Err(Error::BadFrame);
        }
        value |= u32::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok((value as usize, i + 1));
        }
    }
    Err(Error::BadFrame)
}

/// Inflate a zlib stream into exactly `out.len()` bytes, requiring the stream to
/// end exactly at the end of `input` (the Adler-32 trailer is checked by
/// `miniz_oxide`, and the declared output length bounds the expansion).
fn inflate_exact(input: &[u8], out: &mut [u8]) -> Result<(), Error> {
    use miniz_oxide::inflate::core::inflate_flags::{
        TINFL_FLAG_PARSE_ZLIB_HEADER, TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
    };
    use miniz_oxide::inflate::core::{decompress, DecompressorOxide};
    use miniz_oxide::inflate::TINFLStatus;

    let mut state = DecompressorOxide::new();
    let flags = TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF | TINFL_FLAG_PARSE_ZLIB_HEADER;
    let (status, used, written) = decompress(&mut state, input, out, 0, flags);
    if status != TINFLStatus::Done || used != input.len() || written != out.len() {
        return Err(Error::BadFrame);
    }
    Ok(())
}

/// Decode one compression envelope (frame body when a threshold is set) into
/// `scratch`, and return the body length.
///
/// The declared output length is checked against `limits.max_body` and against
/// `scratch` before anything is inflated, so an oversized or impossible
/// envelope costs no work.
fn envelope(
    frame: &[u8],
    threshold: i32,
    scratch: &mut [u8],
    limits: &FrameLimits,
) -> Result<usize, Error> {
    let mut reader = Reader::new(frame);
    let declared = reader.varint()?;
    let rest = reader.rest();
    if declared < 0 {
        return Err(Error::BadFrame);
    }
    if declared == 0 {
        // Vanilla only leaves a body uncompressed when it is below the
        // threshold, so accept exactly that shape (nothing can be raw at 0).
        if rest.is_empty() || rest.len() >= threshold as usize || rest.len() > limits.max_body {
            return Err(Error::BadFrame);
        }
        if rest.len() > scratch.len() {
            return Err(Error::OutputSmall);
        }
        scratch[..rest.len()].copy_from_slice(rest);
        return Ok(rest.len());
    }

    let declared = declared as usize;
    if declared > limits.max_body || declared < threshold as usize || rest.is_empty() {
        return Err(Error::BadFrame);
    }
    if declared > scratch.len() {
        return Err(Error::OutputSmall);
    }
    inflate_exact(rest, &mut scratch[..declared])?;
    Ok(declared)
}

/// Decode the outer frame and, when a threshold is set, its compression
/// envelope into `scratch`. Returns the wire bytes of this frame and the body
/// length at the start of `scratch`.
///
/// An incomplete outer frame is [`Error::NeedMore`]; an outer or inflated body
/// over `limits` is [`Error::BadFrame`] from the outer length prefix or complete
/// compression envelope, before inflation. The envelope is inspected once its
/// outer frame is complete.
pub(crate) fn frame_body(
    data: &[u8],
    threshold: i32,
    scratch: &mut [u8],
    limits: &FrameLimits,
) -> Result<(usize, usize), Error> {
    if threshold < -1 {
        return Err(Error::BadArgument);
    }
    let (frame_len, prefix) = outer_length(data)?;
    if frame_len == 0 || frame_len > limits.max_frame {
        return Err(Error::BadFrame);
    }
    let total = prefix + frame_len;
    if total > limits.max_wire {
        return Err(Error::BadFrame);
    }
    if data.len() < total {
        return Err(Error::NeedMore);
    }
    let frame = &data[prefix..total];

    let body_len = if threshold < 0 {
        if frame.len() > scratch.len() {
            return Err(Error::OutputSmall);
        }
        scratch[..frame.len()].copy_from_slice(frame);
        frame.len()
    } else {
        envelope(frame, threshold, scratch, limits)?
    };
    Ok((total, body_len))
}

pub(crate) fn finish(reader: &Reader<'_>) -> Result<(), Error> {
    if reader.remaining() != 0 {
        Err(Error::BadFrame)
    } else {
        Ok(())
    }
}

/// Packet 2: `GameProfile { uuid, name, properties }` then the session UUID.
/// Out-of-limit profile fields are [`Error::BadProfile`]; truncation, invalid
/// UTF-8 and trailing bytes are [`Error::BadFrame`].
fn parse_success(reader: &mut Reader<'_>) -> Result<Event, Error> {
    let uuid = reader.uuid()?;
    let name_bytes = reader.string(MAX_STRING)?;
    if name_bytes.is_empty()
        || name_bytes.len() > MAX_NAME
        || !name_bytes
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(Error::BadProfile);
    }
    let mut name = [0u8; 17];
    name[..name_bytes.len()].copy_from_slice(name_bytes);

    let count = reader.varint()?;
    if count < 0 || count as usize > MAX_PROPERTIES {
        return Err(Error::BadProfile);
    }
    for _ in 0..count {
        let property_name = reader.string(MAX_STRING)?;
        if property_name.len() > MAX_PROPERTY_NAME {
            return Err(Error::BadProfile);
        }
        reader.string(MAX_STRING)?; // value
        if reader.u8()? != 0 {
            let signature = reader.string(MAX_STRING)?;
            if signature.len() > MAX_SIGNATURE {
                return Err(Error::BadProfile);
            }
        }
    }
    let session = reader.uuid()?;
    finish(reader)?;
    Ok(Event::Success {
        uuid,
        name,
        session,
        properties: count as u32,
    })
}

/// Read a length-prefixed byte array (`byte_array`) and report where its bytes
/// sit. The reader consumes the `VarInt` length and then exactly that many
/// bytes, so the region starts where the read ended minus its length; that
/// position is a scratch offset because the body always starts at scratch 0.
fn byte_array_region(reader: &mut Reader<'_>) -> Result<Region, Error> {
    let bytes = reader.byte_array()?;
    Ok(Region {
        offset: (reader.position() - bytes.len()) as u32,
        length: bytes.len() as u32,
    })
}

fn parse_body(body: &[u8], consumed: usize) -> Result<Decoded, Error> {
    // `body` is always the start of the caller's scratch, so every reader
    // position is a scratch offset: `data_offset` and the Hello regions are
    // relative to the whole packet body, packet ID included.
    let mut reader = Reader::new(body);
    let id = reader.varint()?;
    let data_offset = reader.position();
    let data_length = body.len() - data_offset;
    let event = match id {
        // Disconnect: length-prefixed JSON, valid UTF-8, not syntax-parsed.
        0 => {
            reader.string(MAX_DISCONNECT_JSON)?;
            finish(&reader)?;
            Event::Disconnect
        }
        // Hello: server id, public key, challenge, should-authenticate.
        1 => {
            reader.string(MAX_SERVER_ID)?;
            let public_key = byte_array_region(&mut reader)?;
            let challenge = byte_array_region(&mut reader)?;
            let should_authenticate = reader.u8()? != 0;
            finish(&reader)?;
            Event::Hello {
                should_authenticate,
                public_key,
                challenge,
            }
        }
        2 => parse_success(&mut reader)?,
        // Set Compression: a threshold of -1 or more; no other fields.
        3 => {
            let threshold = reader.varint()?;
            if threshold < -1 {
                return Err(Error::BadFrame);
            }
            finish(&reader)?;
            Event::Compression { threshold }
        }
        // Custom Query: transaction, channel, then whatever is left.
        4 => {
            let transaction = reader.varint()?;
            reader.identifier()?;
            reader.rest();
            Event::CustomQuery { transaction }
        }
        // Cookie Request: an identifier key and nothing else.
        5 => {
            reader.identifier()?;
            finish(&reader)?;
            Event::CookieRequest
        }
        _ => return Err(Error::Unsupported),
    };
    Ok(Decoded {
        event,
        consumed,
        data_offset,
        data_length,
    })
}

/// Decode one complete clientbound login frame from `data` into `scratch`.
///
/// `threshold` is the compression threshold in force: `-1` before the server
/// sends Set Compression (every frame is raw), otherwise the last announced
/// threshold and every frame carries the envelope.
///
/// `scratch` must hold the whole packet body (packet ID included); on success
/// it holds exactly that body and [`Decoded`] points into it. Only this frame
/// is consumed, so a following frame in `data` is left untouched.
///
/// Outer frames that are not complete yet return [`Error::NeedMore`]; complete
/// frames whose envelope, zlib stream or body is malformed return
/// [`Error::BadFrame`]; unmodelled packet IDs return [`Error::Unsupported`].
pub fn decode_login_frame(
    data: &[u8],
    threshold: i32,
    scratch: &mut [u8],
) -> Result<Decoded, Error> {
    let (consumed, body_len) = frame_body(data, threshold, scratch, &FrameLimits::LOGIN)?;
    parse_body(&scratch[..body_len], consumed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;
    use std::vec::Vec;

    /// Clientbound Disconnect (id 0) with reason `{"text":"bye"}`.
    const DISCONNECT_BODY: &[u8] = b"\x00\x0e{\"text\":\"bye\"}";
    /// `zlib.compress(DISCONNECT_BODY)` from Python: 24 bytes, Adler-32 trailer
    /// `25 11 04 ce`.
    const DISCONNECT_ZLIB: &[u8] = b"\x78\xda\x63\xe0\xab\x56\x2a\x49\xad\x28\x51\xb2\x52\x4a\xaa\x4c\x55\xaa\x05\x00\x25\x11\x04\xce";
    /// Notch's offline UUID, as a wire field.
    const NOTCH_UUID: [u8; 16] = [
        0xb5, 0x0a, 0xd3, 0x85, 0x82, 0x9d, 0x31, 0x41, 0xa2, 0x16, 0x7e, 0x7d, 0x75, 0x39, 0xba,
        0x7f,
    ];
    const SESSION: [u8; 16] = [0xaa; 16];

    /// A `GameProfile` property: name, value and optional signature.
    type Property<'a> = (&'a [u8], &'a [u8], Option<&'a [u8]>);

    fn push_varint(out: &mut Vec<u8>, value: i32) {
        let mut bits = value as u32;
        loop {
            let byte = (bits & 0x7f) as u8;
            bits >>= 7;
            if bits == 0 {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
    }

    fn frame(body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        push_varint(&mut out, body.len() as i32);
        out.extend_from_slice(body);
        out
    }

    /// A compressed frame: envelope declared length, then the zlib stream.
    fn compressed_frame(body: &[u8], zlib: &[u8]) -> Vec<u8> {
        let mut inner = Vec::new();
        push_varint(&mut inner, body.len() as i32);
        inner.extend_from_slice(zlib);
        frame(&inner)
    }

    fn decode(data: &[u8], threshold: i32) -> (Result<Decoded, Error>, Vec<u8>) {
        let mut scratch = vec![0u8; MAX_BODY];
        let decoded = decode_login_frame(data, threshold, &mut scratch);
        (decoded, scratch)
    }

    fn success_body(name: &[u8], properties: &[Property<'_>]) -> Vec<u8> {
        let mut body = Vec::new();
        push_varint(&mut body, 2);
        body.extend_from_slice(&NOTCH_UUID);
        push_varint(&mut body, name.len() as i32);
        body.extend_from_slice(name);
        push_varint(&mut body, properties.len() as i32);
        for (property_name, value, signature) in properties {
            push_varint(&mut body, property_name.len() as i32);
            body.extend_from_slice(property_name);
            push_varint(&mut body, value.len() as i32);
            body.extend_from_slice(value);
            match signature {
                Some(signature) => {
                    body.push(1);
                    push_varint(&mut body, signature.len() as i32);
                    body.extend_from_slice(signature);
                }
                None => body.push(0),
            }
        }
        body.extend_from_slice(&SESSION);
        body
    }

    #[test]
    fn raw_frame_reports_event_and_offsets() {
        let wire = frame(DISCONNECT_BODY);
        let (decoded, scratch) = decode(&wire, -1);
        assert_eq!(
            decoded,
            Ok(Decoded {
                event: Event::Disconnect,
                consumed: 17,
                data_offset: 1,
                data_length: 15,
            })
        );
        assert_eq!(&scratch[..DISCONNECT_BODY.len()], DISCONNECT_BODY);

        // A second frame follows and is left untouched.
        let mut joined = wire.clone();
        joined.extend_from_slice(&[1, 0]);
        let (decoded, _) = decode(&joined, -1);
        assert_eq!(decoded.unwrap().consumed, 17);
        assert_eq!(&joined[17..], &[1, 0]);
    }

    #[test]
    fn every_incomplete_outer_frame_needs_more() {
        let wire = frame(DISCONNECT_BODY);
        for end in 0..wire.len() {
            assert_eq!(decode(&wire[..end], -1).0, Err(Error::NeedMore), "{end}");
        }
        assert_eq!(decode(&[], -1).0, Err(Error::NeedMore));

        // A still-possible five-byte length prefix is incomplete, not malformed.
        assert_eq!(decode(&[0x80; 4], -1).0, Err(Error::NeedMore));
        // A truncated compressed frame is incomplete too.
        let compressed = compressed_frame(DISCONNECT_BODY, DISCONNECT_ZLIB);
        for end in 0..compressed.len() {
            assert_eq!(
                decode(&compressed[..end], 16).0,
                Err(Error::NeedMore),
                "{end}"
            );
        }
    }

    #[test]
    fn login_success_carries_profile_and_session() {
        let body = success_body(b"Notch", &[(&b"textures"[..], &b"e30="[..], None)]);
        let (decoded, _) = decode(&frame(&body), -1);
        let decoded = decoded.unwrap();
        assert_eq!(decoded.consumed, 1 + body.len());
        assert_eq!(decoded.data_offset, 1);
        let mut expected_name = [0u8; 17];
        expected_name[..5].copy_from_slice(b"Notch");
        assert_eq!(
            decoded.event,
            Event::Success {
                uuid: NOTCH_UUID,
                name: expected_name,
                session: SESSION,
                properties: 1,
            }
        );

        // A signed property is accepted; the second one has no signature.
        let signed = success_body(
            b"Notch",
            &[
                (&b"textures"[..], &b"e30="[..], Some(&b"c2ln"[..])),
                (&b"extra"[..], &b"v"[..], None),
            ],
        );
        let (decoded, _) = decode(&frame(&signed), -1);
        match decoded.unwrap().event {
            Event::Success { properties, .. } => assert_eq!(properties, 2),
            other => panic!("{other:?}"),
        }

        // Longest accepted name and an empty property list.
        let body = success_body(b"abcdefghijklmnop", &[]);
        assert!(matches!(
            decode(&frame(&body), -1).0.unwrap().event,
            Event::Success { properties: 0, .. }
        ));
    }

    #[test]
    fn login_success_truncation_and_profile_limits() {
        let body = success_body(b"Notch", &[(&b"textures"[..], &b"e30="[..], None)]);
        // Every proper prefix of a complete frame is a malformed inner packet.
        for end in 0..body.len() {
            assert_eq!(
                decode(&frame(&body[..end]), -1).0,
                Err(Error::BadFrame),
                "prefix {end}"
            );
        }
        // One trailing byte inside the frame is a malformed inner packet.
        let mut trailing = body.clone();
        trailing.push(0);
        assert_eq!(decode(&frame(&trailing), -1).0, Err(Error::BadFrame));

        let too_long_name = success_body(&[b'a'; 17], &[]);
        assert_eq!(decode(&frame(&too_long_name), -1).0, Err(Error::BadProfile));
        for name in [b"".as_slice(), b"a\0b", b"bad name"] {
            assert_eq!(
                decode(&frame(&success_body(name, &[])), -1).0,
                Err(Error::BadProfile)
            );
        }
        let non_ascii_name = success_body("na\u{e9}me".as_bytes(), &[]);
        assert_eq!(
            decode(&frame(&non_ascii_name), -1).0,
            Err(Error::BadProfile)
        );

        let properties: Vec<Property<'_>> = (0..65).map(|_| (&b"n"[..], &b"v"[..], None)).collect();
        assert_eq!(
            decode(&frame(&success_body(b"Notch", &properties)), -1).0,
            Err(Error::BadProfile)
        );
        for (name, value, signature) in [
            (&[b'n'; 65][..], &b"v"[..], None),
            (&b"n"[..], &b"v"[..], Some(&[b's'; 1025][..])),
        ] {
            assert_eq!(
                decode(
                    &frame(&success_body(b"Notch", &[(name, value, signature)])),
                    -1
                )
                .0,
                Err(Error::BadProfile),
                "{name:?}/{signature:?}"
            );
        }
        // 64 properties and a 1024-byte signature are the accepted maxima.
        let properties: Vec<Property<'_>> = (0..64).map(|_| (&b"n"[..], &b"v"[..], None)).collect();
        assert!(decode(&frame(&success_body(b"Notch", &properties)), -1)
            .0
            .is_ok());
        assert!(decode(
            &frame(&success_body(
                b"Notch",
                &[(&b"n"[..], &b"v"[..], Some(&[b's'; 1024][..]))]
            )),
            -1
        )
        .0
        .is_ok());
    }

    #[test]
    fn compression_envelope_accepts_vanilla_shapes() {
        // Compressed: declared 16 >= threshold 16.
        let wire = compressed_frame(DISCONNECT_BODY, DISCONNECT_ZLIB);
        let (decoded, scratch) = decode(&wire, 16);
        let decoded = decoded.unwrap();
        assert_eq!(decoded.event, Event::Disconnect);
        assert_eq!(decoded.consumed, wire.len());
        assert_eq!(&scratch[..DISCONNECT_BODY.len()], DISCONNECT_BODY);
        // A declared length below the threshold is not something Vanilla writes.
        assert_eq!(decode(&wire, 17).0, Err(Error::BadFrame));
        // At threshold 0 everything is compressed, and this frame still is.
        assert!(decode(&wire, 0).0.is_ok());

        // Two compressed frames coalesced in one buffer: only the first is
        // consumed, so the caller can retry with the remainder.
        let mut joined = wire.clone();
        joined.extend_from_slice(&wire);
        let (decoded, scratch) = decode(&joined, 16);
        assert_eq!(decoded.unwrap().consumed, wire.len());
        assert_eq!(&scratch[..DISCONNECT_BODY.len()], DISCONNECT_BODY);
        let (decoded, _) = decode(&joined[wire.len()..], 16);
        assert_eq!(decoded.unwrap().consumed, wire.len());

        // Raw: declared 0 with a body below the threshold.
        let mut inner = vec![0u8];
        inner.extend_from_slice(DISCONNECT_BODY);
        let raw = frame(&inner);
        for threshold in [17, 257] {
            let (decoded, _) = decode(&raw, threshold);
            assert_eq!(decoded.unwrap().event, Event::Disconnect);
        }
        // The same body is not raw once it reaches the threshold.
        for threshold in [16, 0] {
            assert_eq!(
                decode(&raw, threshold).0,
                Err(Error::BadFrame),
                "{threshold}"
            );
        }
    }

    #[test]
    fn compression_envelope_rejects_corruption() {
        let wire = compressed_frame(DISCONNECT_BODY, DISCONNECT_ZLIB);
        // Declared length larger or smaller than the actual output.
        for declared in [15u8, 17] {
            let mut bad = wire.clone();
            bad[1] = declared;
            assert_eq!(decode(&bad, 16).0, Err(Error::BadFrame), "{declared}");
        }
        // A corrupted Adler-32 trailer.
        let mut bad = wire.clone();
        let last = bad.len() - 1;
        bad[last] ^= 0xff;
        assert_eq!(decode(&bad, 16).0, Err(Error::BadFrame));
        // A corrupted zlib header (FCHECK no longer divisible by 31).
        let mut bad = wire.clone();
        bad[3] ^= 1;
        assert_eq!(decode(&bad, 16).0, Err(Error::BadFrame));
        // One trailing byte inside the frame: the stream must end at the frame.
        let mut bad = compressed_frame(DISCONNECT_BODY, DISCONNECT_ZLIB);
        bad.push(0);
        let len = bad.len() - 1;
        bad[0] = len as u8;
        assert_eq!(decode(&bad, 16).0, Err(Error::BadFrame));
        // A zip bomb: a tiny stream that declares 65535 bytes of output.
        let bomb = compressed_frame(&[0xff; 65_535], DISCONNECT_ZLIB);
        assert_eq!(decode(&bomb, 16).0, Err(Error::BadFrame));
        // A declared length over the body cap is rejected from the envelope.
        let mut huge = Vec::new();
        push_varint(&mut huge, MAX_BODY as i32 + 1);
        huge.extend_from_slice(DISCONNECT_ZLIB);
        assert_eq!(decode(&frame(&huge), 16).0, Err(Error::BadFrame));
        // An empty raw body is not a packet.
        assert_eq!(decode(&frame(&[0]), 16).0, Err(Error::BadFrame));
        // A negative declared length.
        assert_eq!(
            decode(&frame(&[0xff, 0xff, 0xff, 0xff, 0x0f, 0]), 16).0,
            Err(Error::BadFrame)
        );
    }

    #[test]
    fn hello_compression_query_and_cookie_events() {
        // Hello: server id "", 64-byte key, 32-byte challenge, should-authenticate.
        let mut hello = vec![1u8, 0, 64];
        hello.extend_from_slice(&[0x11; 64]);
        hello.push(32);
        hello.extend_from_slice(&[0x22; 32]);
        hello.push(1);
        let (decoded, scratch) = decode(&frame(&hello), -1);
        let decoded = decoded.unwrap();
        // The regions point at the key and the challenge inside the body, which
        // starts with the packet ID at scratch offset 0.
        assert_eq!(
            decoded.event,
            Event::Hello {
                should_authenticate: true,
                public_key: Region {
                    offset: 3,
                    length: 64,
                },
                challenge: Region {
                    offset: 68,
                    length: 32,
                },
            }
        );
        assert_eq!(&scratch[3..67], &[0x11; 64]);
        assert_eq!(&scratch[68..100], &[0x22; 32]);
        assert_eq!(
            (decoded.data_offset, decoded.data_length),
            (1, hello.len() - 1)
        );
        let mut anonymous = hello.clone();
        let last = anonymous.len() - 1;
        anonymous[last] = 0;
        let (decoded, scratch) = decode(&frame(&anonymous), -1);
        assert_eq!(
            decoded.unwrap().event,
            Event::Hello {
                should_authenticate: false,
                public_key: Region {
                    offset: 3,
                    length: 64,
                },
                challenge: Region {
                    offset: 68,
                    length: 32,
                },
            }
        );
        assert_eq!(&scratch[3..67], &[0x11; 64]);
        assert_eq!(&scratch[68..100], &[0x22; 32]);

        // Set Compression: a VarInt threshold of -1 or more.
        for threshold in [-1i32, 0, 256] {
            let mut body = vec![3u8];
            push_varint(&mut body, threshold);
            assert_eq!(
                decode(&frame(&body), -1).0.unwrap().event,
                Event::Compression { threshold }
            );
        }
        for threshold in [-2i32, i32::MIN] {
            let mut body = vec![3u8];
            push_varint(&mut body, threshold);
            assert_eq!(decode(&frame(&body), -1).0, Err(Error::BadFrame));
        }

        // Custom Query: transaction, channel identifier, rest of the frame.
        let mut query = vec![4u8, 3, 20];
        query.extend_from_slice(b"velocity:player_info");
        query.push(7);
        let (decoded, _) = decode(&frame(&query), -1);
        let decoded = decoded.unwrap();
        assert_eq!(decoded.event, Event::CustomQuery { transaction: 3 });
        assert_eq!(decoded.data_length, query.len() - 1);
        let mut bad = query.clone();
        bad[3] = b'A';
        assert_eq!(decode(&frame(&bad), -1).0, Err(Error::BadFrame));

        // Cookie Request: an identifier key and nothing else.
        let cookie = b"\x05\x03a:b";
        let (decoded, _) = decode(&frame(cookie), -1);
        let decoded = decoded.unwrap();
        assert_eq!(decoded.event, Event::CookieRequest);
        assert_eq!((decoded.data_offset, decoded.data_length), (1, 4));
        let mut trailing = cookie.to_vec();
        trailing.push(0);
        assert_eq!(decode(&frame(&trailing), -1).0, Err(Error::BadFrame));
        assert_eq!(decode(&frame(b"\x05\x03A:b"), -1).0, Err(Error::BadFrame));
    }

    #[test]
    fn unknown_packet_ids_are_unsupported() {
        for id in [6u8, 7, 127] {
            assert_eq!(decode(&frame(&[id]), -1).0, Err(Error::Unsupported));
        }
        // A negative VarInt id is not a login packet either.
        assert_eq!(
            decode(&frame(&[0xff, 0xff, 0xff, 0xff, 0x0f]), -1).0,
            Err(Error::Unsupported)
        );
        // A malformed id VarInt is a frame error, not an unsupported packet.
        assert_eq!(
            decode(&frame(&[0x80, 0x80, 0x80, 0x80, 0x80]), -1).0,
            Err(Error::BadFrame)
        );
    }

    #[test]
    fn scratch_capacity_is_checked_before_writing() {
        let wire = frame(DISCONNECT_BODY);
        let mut exact = [0u8; 16];
        assert!(decode_login_frame(&wire, -1, &mut exact).is_ok());
        let mut one_short = [0xa5u8; 15];
        assert_eq!(
            decode_login_frame(&wire, -1, &mut one_short),
            Err(Error::OutputSmall)
        );
        assert_eq!(one_short, [0xa5u8; 15]);

        let compressed = compressed_frame(DISCONNECT_BODY, DISCONNECT_ZLIB);
        let mut one_short = [0xa5u8; 15];
        assert_eq!(
            decode_login_frame(&compressed, 16, &mut one_short),
            Err(Error::OutputSmall)
        );
        assert_eq!(one_short, [0xa5u8; 15]);
    }

    #[test]
    fn body_cap_matches_the_header_limits() {
        // A 65536-byte body fits: id 0, a three-byte string length, the rest JSON.
        let json = vec![b'a'; MAX_BODY - 4];
        let mut body = vec![0u8, 0xfc, 0xff, 0x03];
        body.extend_from_slice(&json);
        assert_eq!(body.len(), MAX_BODY);
        let wire = frame(&body);
        assert_eq!(wire.len(), 65_539);
        assert!(wire.len() <= MAX_WIRE);
        assert_eq!(decode(&wire, -1).0.unwrap().consumed, wire.len());

        // One byte more is over the body cap, and the prefix alone says so.
        let json = vec![b'a'; MAX_BODY - 3];
        let mut body = vec![0u8, 0xfd, 0xff, 0x03];
        body.extend_from_slice(&json);
        let wire = frame(&body);
        assert_eq!(decode(&wire, -1).0, Err(Error::BadFrame));
        // A zero-length frame and an impossible length prefix are frame errors.
        assert_eq!(decode(&[0], -1).0, Err(Error::BadFrame));
        assert_eq!(
            decode(&[0xff, 0xff, 0xff, 0xff, 0x0f], -1).0,
            Err(Error::BadFrame)
        );
        // An out-of-contract threshold is an argument error.
        assert_eq!(decode(&wire, -2).0, Err(Error::BadArgument));
    }

    #[test]
    fn malformed_input_never_panics() {
        let mut scratch = vec![0u8; MAX_BODY];
        for threshold in [-1, 0, 16] {
            for a in 0..=255u8 {
                for b in 0..=255u8 {
                    let _ = decode_login_frame(&[a, b], threshold, &mut scratch);
                }
            }
        }

        // Bit flips across every byte of two valid frames.
        let frames = [
            frame(&success_body(
                b"Notch",
                &[(&b"textures"[..], &b"e30="[..], None)],
            )),
            compressed_frame(DISCONNECT_BODY, DISCONNECT_ZLIB),
        ];
        for wire in frames {
            for i in 0..wire.len() {
                for bit in 0..8 {
                    let mut bad = wire.clone();
                    bad[i] ^= 1 << bit;
                    for threshold in [-1, 0, 16] {
                        let _ = decode_login_frame(&bad, threshold, &mut scratch);
                    }
                }
            }
        }

        // Deterministic pseudo-random buffers, short and long.
        let mut state = 0x1234_5678u32;
        for _ in 0..4_000 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let len = (state as usize % 96) + 1;
            let mut buffer = vec![0u8; len];
            for byte in buffer.iter_mut() {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                *byte = state as u8;
            }
            for threshold in [-1, 0, 1, 256] {
                let _ = decode_login_frame(&buffer, threshold, &mut scratch);
            }
        }
    }
}
