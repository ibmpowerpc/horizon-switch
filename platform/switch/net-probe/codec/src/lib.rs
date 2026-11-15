//! Allocation-free Java server-list status framing for the Switch probe.
//! Production gameplay continues to use Horizon's existing protocol crate.
//! No JSON syntax parsing, login, DNS, TCP or console execution is provided.
#![no_std]

pub const MAX_JSON_BYTES: usize = 16_384;
// Each nested VarInt can occupy at most five bytes (including noncanonical
// encodings); the outer frame-length prefix is separate.
pub const MAX_FRAME_BYTES: usize = MAX_JSON_BYTES + 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    OutputTooSmall,
    NeedMore,
    VarIntOverflow,
    InvalidLength,
    InvalidUtf8,
    InvalidPacketId,
    FrameLengthMismatch,
    InvalidHost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusFrame<'a> {
    pub json: &'a str,
    pub consumed: usize,
}

fn varint_len(value: i32) -> usize {
    let mut value = value as u32;
    let mut n = 1;
    while value > 0x7f {
        value >>= 7;
        n += 1;
    }
    n
}

fn write_varint(value: i32, out: &mut [u8]) -> Result<usize, Error> {
    let n = varint_len(value);
    if out.len() < n {
        return Err(Error::OutputTooSmall);
    }
    let mut value = value as u32;
    for (i, byte) in out[..n].iter_mut().enumerate() {
        *byte = (value & 0x7f) as u8 | if i + 1 < n { 0x80 } else { 0 };
        value >>= 7;
    }
    Ok(n)
}

fn read_varint(input: &[u8]) -> Result<(i32, usize), Error> {
    let mut value = 0u32;
    for i in 0..5 {
        let byte = *input.get(i).ok_or(Error::NeedMore)?;
        if i == 4 && byte & 0xf0 != 0 {
            return Err(Error::VarIntOverflow);
        }
        value |= u32::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok((value as i32, i + 1));
        }
    }
    Err(Error::VarIntOverflow)
}

/// Encode two complete TCP frames: handshake (next-state status), then the
/// empty status request [0x01, 0x00]. Protocol -1 uses signed Java VarInt bits.
/// Host is restricted to 1..=253 printable ASCII bytes (no IDNA or SRV).
/// On an error the output is unchanged.
pub fn encode_status_request(
    host: &str,
    port: u16,
    protocol: i32,
    out: &mut [u8],
) -> Result<usize, Error> {
    if host.is_empty() || host.len() > 253 || !host.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(Error::InvalidHost);
    }
    let payload_len = 1 + varint_len(protocol) + varint_len(host.len() as i32) + host.len() + 2 + 1;
    let total = varint_len(payload_len as i32) + payload_len + 2;
    if out.len() < total {
        return Err(Error::OutputTooSmall);
    }
    let mut pos = write_varint(payload_len as i32, out)?;
    out[pos] = 0; // Handshake packet id.
    pos += 1;
    pos += write_varint(protocol, &mut out[pos..])?;
    pos += write_varint(host.len() as i32, &mut out[pos..])?;
    out[pos..pos + host.len()].copy_from_slice(host.as_bytes());
    pos += host.len();
    out[pos..pos + 2].copy_from_slice(&port.to_be_bytes());
    pos += 2;
    out[pos] = 1; // Next-state status.
    pos += 1;
    out[pos..pos + 2].copy_from_slice(&[1, 0]);
    Ok(total)
}

fn inner_varint(input: &[u8]) -> Result<(i32, usize), Error> {
    // The enclosing frame is complete: a truncated inner field cannot become
    // valid by reading bytes belonging to a subsequent TCP frame.
    read_varint(input).map_err(|e| {
        if e == Error::NeedMore {
            Error::FrameLengthMismatch
        } else {
            e
        }
    })
}

/// Parse one complete server status response: frame length, packet id 0,
/// UTF-8 string length and JSON bytes. A following frame is left unconsumed.
/// Incomplete outer framing returns NeedMore; excessive lengths are rejected
/// from the prefix alone, before asking the caller to buffer a large body.
pub fn parse_status_frame(input: &[u8]) -> Result<StatusFrame<'_>, Error> {
    let (length, prefix) = read_varint(input)?;
    if length <= 0 || length as usize > MAX_FRAME_BYTES {
        return Err(Error::InvalidLength);
    }
    let end = prefix + length as usize;
    if input.len() < end {
        return Err(Error::NeedMore);
    }
    let body = &input[prefix..end];
    let (id, id_bytes) = inner_varint(body)?;
    if id != 0 {
        return Err(Error::InvalidPacketId);
    }
    let (json_len, len_bytes) = inner_varint(&body[id_bytes..])?;
    if json_len < 0 || json_len as usize > MAX_JSON_BYTES {
        return Err(Error::InvalidLength);
    }
    let start = id_bytes + len_bytes;
    if start + json_len as usize != body.len() {
        return Err(Error::FrameLengthMismatch);
    }
    let json = core::str::from_utf8(&body[start..]).map_err(|_| Error::InvalidUtf8)?;
    Ok(StatusFrame {
        json,
        consumed: end,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_matches_independent_wire_vector() {
        let mut out = [0u8; 64];
        let n = encode_status_request("localhost", 25565, 777, &mut out).unwrap();
        assert_eq!(
            &out[..n],
            b"\x10\x00\x89\x06\x09localhost\x63\xdd\x01\x01\x00"
        );
        let n = encode_status_request("localhost", 25565, -1, &mut out).unwrap();
        assert_eq!(
            &out[..n],
            b"\x13\x00\xff\xff\xff\xff\x0f\x09localhost\x63\xdd\x01\x01\x00"
        );
    }

    #[test]
    fn invalid_host_and_small_buffer_leave_output_untouched() {
        for host in ["", "bad\0host", "bad host", "\u{e9}"] {
            let mut out = [0xa5; 64];
            assert_eq!(
                encode_status_request(host, 25565, 777, &mut out),
                Err(Error::InvalidHost)
            );
            assert_eq!(out, [0xa5; 64]);
        }
        for length in 0..19 {
            let mut out = [0xa5; 19];
            assert_eq!(
                encode_status_request("localhost", 25565, 777, &mut out[..length]),
                Err(Error::OutputTooSmall)
            );
            assert_eq!(out, [0xa5; 19]);
        }
        let mut out = [0; 300];
        assert!(encode_status_request(
            core::str::from_utf8(&[b'a'; 253]).unwrap(),
            25565,
            777,
            &mut out
        )
        .is_ok());
        assert_eq!(
            encode_status_request(
                core::str::from_utf8(&[b'a'; 254]).unwrap(),
                25565,
                777,
                &mut out
            ),
            Err(Error::InvalidHost)
        );
    }

    #[test]
    fn server_status_vector_and_next_frame_consumption() {
        let bytes = b"\x04\x00\x02{}\x01\x00";
        assert_eq!(
            parse_status_frame(bytes),
            Ok(StatusFrame {
                json: "{}",
                consumed: 5
            })
        );
        assert_eq!(
            parse_status_frame(b"\x06\x00\x04\x22\xc3\xa9\x22"),
            Ok(StatusFrame {
                json: "\"\u{e9}\"",
                consumed: 7
            })
        );
    }

    #[test]
    fn each_valid_incomplete_prefix_needs_more() {
        let bytes = b"\x04\x00\x02{}";
        for end in 0..bytes.len() {
            assert_eq!(parse_status_frame(&bytes[..end]), Err(Error::NeedMore));
        }
        assert_eq!(parse_status_frame(&[0x80; 4]), Err(Error::NeedMore));
    }

    #[test]
    fn reject_malformed_or_excessive_lengths_from_prefix() {
        for bytes in [&[0xff, 0xff, 0xff, 0xff, 0x10][..], &[0x80; 5][..]] {
            assert_eq!(parse_status_frame(bytes), Err(Error::VarIntOverflow));
        }
        for bytes in [
            &[0][..],
            &[0xff, 0xff, 0xff, 0xff, 0x0f][..],
            &[0x8b, 0x80, 1][..],
        ] {
            assert_eq!(parse_status_frame(bytes), Err(Error::InvalidLength));
        }
        assert_eq!(
            parse_status_frame(&[4, 0, 0x81, 0x80, 1]),
            Err(Error::InvalidLength)
        );
    }

    #[test]
    fn reject_wrong_id_invalid_utf8_and_mismatched_inner_fields() {
        assert_eq!(parse_status_frame(&[2, 1, 0]), Err(Error::InvalidPacketId));
        assert_eq!(
            parse_status_frame(&[3, 0, 1, 0xff]),
            Err(Error::InvalidUtf8)
        );
        for bytes in [
            &[1, 0x80][..],
            &[2, 0, 0x80][..],
            &[3, 0, 0, 0][..],
            &[3, 0, 2, b'{'][..],
        ] {
            assert_eq!(parse_status_frame(bytes), Err(Error::FrameLengthMismatch));
        }
    }

    #[test]
    fn exact_maximum_payload_and_all_short_inputs_are_bounded() {
        // JSON length 16384 occupies three bytes, so the full vector needs
        // seven prefix bytes: outer(3), id(1), string length(3).
        let mut frame = [b' '; MAX_JSON_BYTES + 7];
        frame[..7].copy_from_slice(&[0x84, 0x80, 1, 0, 0x80, 0x80, 1]);
        assert_eq!(
            parse_status_frame(&frame).unwrap().json.len(),
            MAX_JSON_BYTES
        );
        for first in 0u8..=255 {
            for second in 0u8..=255 {
                let _ = parse_status_frame(&[first, second]);
            }
        }
    }
}
