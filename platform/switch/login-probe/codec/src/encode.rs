//! Encoding of the two client frames that open a login: the handshake with
//! next state 2, then LoginStart with the offline-mode UUID.

use crate::md5::offline_uuid;
use crate::wire::{varint_len, write_varint};
use crate::{Error, MAX_HOST, MAX_NAME, MAX_WIRE, PROTOCOL};

fn valid_host(host: &[u8]) -> bool {
    // Graphic ASCII only: no spaces, no NUL, no non-ASCII host names (no IDNA
    // or SRV lookup happens here), and Vanilla's 253-byte limit.
    !host.is_empty() && host.len() <= MAX_HOST && host.iter().all(u8::is_ascii_graphic)
}

fn valid_name(name: &[u8]) -> bool {
    // Vanilla's offline username rule: letters, digits and underscore.
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name.iter().all(|&c| c.is_ascii_alphanumeric() || c == b'_')
}

/// Encode both complete TCP frames of a login request:
///
/// 1. handshake: id 0, protocol [`PROTOCOL`], host, port, next state 2;
/// 2. LoginStart: id 0, username, UUID v3 of `"OfflinePlayer:" + name`.
///
/// Nothing is written when the arguments or `out` are not acceptable, so a
/// caller's buffer is unchanged on every error.
pub fn encode_login_start(
    host: &[u8],
    port: u16,
    name: &[u8],
    out: &mut [u8],
) -> Result<usize, Error> {
    if !valid_host(host) || !valid_name(name) || port == 0 {
        return Err(Error::BadArgument);
    }
    let handshake = 1 + varint_len(PROTOCOL) + varint_len(host.len() as i32) + host.len() + 2 + 1;
    let login = 1 + varint_len(name.len() as i32) + name.len() + 16;
    let total = varint_len(handshake as i32) + handshake + varint_len(login as i32) + login;
    if total > MAX_WIRE {
        return Err(Error::BadArgument);
    }
    if out.len() < total {
        return Err(Error::OutputSmall);
    }

    // The capacity check above makes every write below in bounds.
    let mut pos = write_varint(handshake as i32, out)?;
    out[pos] = 0; // Handshake packet id.
    pos += 1;
    pos += write_varint(PROTOCOL, &mut out[pos..])?;
    pos += write_varint(host.len() as i32, &mut out[pos..])?;
    out[pos..pos + host.len()].copy_from_slice(host);
    pos += host.len();
    out[pos..pos + 2].copy_from_slice(&port.to_be_bytes());
    pos += 2;
    out[pos] = 2; // Next state: login.
    pos += 1;

    pos += write_varint(login as i32, &mut out[pos..])?;
    out[pos] = 0; // LoginStart packet id.
    pos += 1;
    pos += write_varint(name.len() as i32, &mut out[pos..])?;
    out[pos..pos + name.len()].copy_from_slice(name);
    pos += name.len();
    out[pos..pos + 16].copy_from_slice(&offline_uuid(name));
    pos += 16;
    Ok(pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    /// Independent wire vector, produced outside this crate: `localhost:25565`,
    /// next state 2, then LoginStart "Notch" with its offline UUID
    /// `b50ad385-829d-3141-a216-7e7d7539ba7f`.
    const REQUEST: &[u8] = b"\x10\x00\x89\x06\x09localhost\x63\xdd\x02\
                             \x17\x00\x05Notch\xb5\x0a\xd3\x85\x82\x9d\x31\x41\xa2\x16\x7e\x7d\x75\x39\xba\x7f";

    fn encoded(host: &[u8], port: u16, name: &[u8]) -> Vec<u8> {
        let mut out = [0xa5u8; 512];
        let len = encode_login_start(host, port, name, &mut out).expect("valid request");
        out[..len].to_vec()
    }

    #[test]
    fn request_matches_independent_wire_vector() {
        assert_eq!(encoded(b"localhost", 25565, b"Notch"), REQUEST);

        let mut out = [0u8; 64];
        let written = encode_login_start(b"localhost", 25565, b"Notch", &mut out).unwrap();
        assert_eq!(written, REQUEST.len());
        assert_eq!(&out[..written], REQUEST);
    }

    #[test]
    fn port_and_host_are_big_endian_and_bounds_checked() {
        // The handshake carries the port as an unsigned big-endian short.
        let bytes = encoded(b"h", 1, b"a");
        assert_eq!(&bytes[1 + 1 + 2 + 1 + 1..][..2], &[0x00, 0x01]);
        // 253 bytes is accepted, 254 is not.
        let longest_host = [b'a'; 253];
        assert!(encode_login_start(&longest_host, 25565, b"a", &mut [0u8; 512]).is_ok());
        let too_long_host = [b'a'; 254];
        assert_eq!(
            encode_login_start(&too_long_host, 25565, b"a", &mut [0u8; 512]),
            Err(Error::BadArgument)
        );
    }

    #[test]
    fn rejects_out_of_contract_arguments_without_touching_output() {
        let cases: [(&[u8], u16, &[u8]); 10] = [
            (b"", 25565, b"Notch"),                      // empty host
            (b"bad host", 25565, b"Notch"),              // space
            (b"bad\0host", 25565, b"Notch"),             // NUL
            (b"h\xc3\xa9", 25565, b"Notch"),             // non-ASCII
            (b"localhost", 0, b"Notch"),                 // zero port
            (b"localhost", 25565, b""),                  // empty name
            (b"localhost", 25565, b"abcdefghijklmnopq"), // 17 bytes
            (b"localhost", 25565, b"name-with-dash"),
            (b"localhost", 25565, b"name with space"),
            (b"localhost", 25565, b"nam\xc3\xa9"),
        ];
        for (host, port, name) in cases {
            let mut out = [0xa5u8; 128];
            assert_eq!(
                encode_login_start(host, port, name, &mut out),
                Err(Error::BadArgument),
                "{host:?}/{port}/{name:?}"
            );
            assert_eq!(out, [0xa5u8; 128], "{host:?}/{port}/{name:?}");
        }
    }

    #[test]
    fn short_output_is_reported_and_left_untouched() {
        let mut out = [0xa5u8; 64];
        for capacity in 0..REQUEST.len() {
            assert_eq!(
                encode_login_start(b"localhost", 25565, b"Notch", &mut out[..capacity]),
                Err(Error::OutputSmall),
                "capacity {capacity}"
            );
            assert_eq!(out, [0xa5u8; 64]);
        }
        assert_eq!(
            encode_login_start(b"localhost", 25565, b"Notch", &mut out[..REQUEST.len()]),
            Ok(REQUEST.len())
        );
    }

    #[test]
    fn longest_name_encodes_its_uuid() {
        let bytes = encoded(b"localhost", 25565, b"abcdefghijklmnop");
        assert!(bytes.ends_with(&[
            0x9d, 0xc7, 0x65, 0x58, 0x52, 0x0f, 0x35, 0x60, 0x86, 0xb0, 0xb5, 0xc7, 0xc5, 0x00,
            0x84, 0x8b
        ]));
    }
}
