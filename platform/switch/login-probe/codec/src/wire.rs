//! Java's `VarInt`, fixed-width primitives, `Utf8String` and `Identifier`
//! encodings, bounded by the caller's slice.

use crate::Error;

/// Java `VarInt` length in bytes of `value`'s 32-bit two's complement bits.
pub fn varint_len(value: i32) -> usize {
    let mut bits = value as u32;
    let mut len = 1;
    while bits > 0x7f {
        bits >>= 7;
        len += 1;
    }
    len
}

/// Write a `VarInt`; the caller guarantees `out` is large enough
/// ([`varint_len`]) or gets [`Error::OutputSmall`].
pub fn write_varint(value: i32, out: &mut [u8]) -> Result<usize, Error> {
    let len = varint_len(value);
    if out.len() < len {
        return Err(Error::OutputSmall);
    }
    let mut bits = value as u32;
    for (i, byte) in out[..len].iter_mut().enumerate() {
        *byte = (bits & 0x7f) as u8 | if i + 1 < len { 0x80 } else { 0 };
        bits >>= 7;
    }
    Ok(len)
}

/// A cursor over one packet body or one compression envelope. Every read is
/// bounds-checked; a read past the end is [`Error::BadFrame`] because the
/// enclosing frame is already complete (more bytes cannot fix it).
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// `VarInt.read`: at most five 7-bit groups, least significant first, with
    /// the fifth group restricted to four bits (Vanilla's `VarInt` is 32-bit).
    pub fn varint(&mut self) -> Result<i32, Error> {
        let mut value = 0u32;
        for i in 0..5 {
            let byte = *self.data.get(self.pos).ok_or(Error::BadFrame)?;
            self.pos += 1;
            if i == 4 && byte & 0xf0 != 0 {
                return Err(Error::BadFrame);
            }
            value |= u32::from(byte & 0x7f) << (7 * i);
            if byte & 0x80 == 0 {
                return Ok(value as i32);
            }
        }
        Err(Error::BadFrame)
    }

    pub fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, Error> {
        let bytes = self.bytes(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub fn bytes(&mut self, len: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(len).ok_or(Error::BadFrame)?;
        let slice = self.data.get(self.pos..end).ok_or(Error::BadFrame)?;
        self.pos = end;
        Ok(slice)
    }

    /// A byte array is a `VarInt` length followed by that many bytes.
    pub fn byte_array(&mut self) -> Result<&'a [u8], Error> {
        let len = self.varint()?;
        if len < 0 {
            return Err(Error::BadFrame);
        }
        self.bytes(len as usize)
    }

    pub fn uuid(&mut self) -> Result<[u8; 16], Error> {
        let slice = self.bytes(16)?;
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(slice);
        Ok(uuid)
    }

    /// `Utf8String.read`: a `VarInt` byte length, valid UTF-8, at most
    /// `max_utf16` UTF-16 code units (Vanilla counts Java `String` length).
    pub fn string(&mut self, max_utf16: usize) -> Result<&'a [u8], Error> {
        let len = self.varint()?;
        if len < 0 {
            return Err(Error::BadFrame);
        }
        let bytes = self.bytes(len as usize)?;
        let text = core::str::from_utf8(bytes).map_err(|_| Error::BadFrame)?;
        if text.encode_utf16().count() > max_utf16 {
            return Err(Error::BadFrame);
        }
        Ok(bytes)
    }

    /// `Identifier.STREAM_CODEC`: a resource location with Vanilla's character
    /// rules, an optional `namespace:` (default `minecraft`) and a non-empty
    /// path.
    pub fn identifier(&mut self) -> Result<&'a [u8], Error> {
        let value = self.string(32_767)?;
        let (namespace, path) = match value.iter().position(|&b| b == b':') {
            Some(colon) if colon > 0 => (&value[..colon], &value[colon + 1..]),
            _ => (b"minecraft".as_slice(), value),
        };
        let namespace_ok = !namespace.is_empty()
            && namespace.iter().all(|&c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'.' | b'-')
            });
        let path_ok = !path.is_empty()
            && path.iter().all(|&c| {
                c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || matches!(c, b'/' | b'_' | b'.' | b'-')
            });
        if !namespace_ok || !path_ok {
            return Err(Error::BadFrame);
        }
        Ok(value)
    }

    /// Everything from the cursor to the end (Vanilla's `RestN` payload).
    pub fn rest(&mut self) -> &'a [u8] {
        let rest = &self.data[self.pos..];
        self.pos = self.data.len();
        rest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_matches_vanilla_vectors() {
        let mut out = [0u8; 5];
        assert_eq!(write_varint(0, &mut out), Ok(1));
        assert_eq!(out[0], 0);
        assert_eq!(write_varint(777, &mut out), Ok(2));
        assert_eq!(&out[..2], &[0x89, 0x06]);
        assert_eq!(write_varint(16_384, &mut out), Ok(3));
        assert_eq!(&out[..3], &[0x80, 0x80, 0x01]);
        // -1 is the five-byte form of 0xffff_ffff.
        assert_eq!(write_varint(-1, &mut out), Ok(5));
        assert_eq!(&out[..5], &[0xff, 0xff, 0xff, 0xff, 0x0f]);
        assert_eq!(write_varint(1, &mut [0u8; 0]), Err(Error::OutputSmall));

        let mut reader = Reader::new(&[0x89, 0x06]);
        assert_eq!(reader.varint(), Ok(777));
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn varint_rejects_overflow_and_truncation() {
        for bytes in [
            &[0x80u8, 0x80, 0x80, 0x80, 0x80][..],
            &[0xff, 0xff, 0xff, 0xff, 0x10][..],
        ] {
            assert_eq!(Reader::new(bytes).varint(), Err(Error::BadFrame));
        }
        for end in 0..5 {
            assert_eq!(
                Reader::new(&[0x80u8, 0x80, 0x80, 0x80, 0x00][..end]).varint(),
                Err(Error::BadFrame)
            );
        }
    }

    #[test]
    fn strings_check_length_utf8_and_utf16_units() {
        // "héllo" is 6 UTF-8 bytes but 5 UTF-16 units.
        let mut reader = Reader::new(b"\x06h\xc3\xa9llo");
        assert_eq!(reader.string(5), Ok(&b"h\xc3\xa9llo"[..]));
        let mut reader = Reader::new(b"\x06h\xc3\xa9llo");
        assert_eq!(reader.string(4), Err(Error::BadFrame));
        // Invalid UTF-8, negative length, truncated bytes and past-the-end.
        assert_eq!(Reader::new(&[1, 0xff]).string(16), Err(Error::BadFrame));
        assert_eq!(
            Reader::new(&[0xff, 0xff, 0xff, 0xff, 0x0f]).string(16),
            Err(Error::BadFrame)
        );
        assert_eq!(Reader::new(&[4, b'a']).string(16), Err(Error::BadFrame));
        assert_eq!(Reader::new(&[0x80]).string(16), Err(Error::BadFrame));
    }

    #[test]
    fn identifiers_follow_vanilla_character_rules() {
        for ok in [
            &b"\x09a:b/c.dat"[..],
            &b"\x03a_b"[..],
            &b"\x01a"[..],
            &b"\x0bminecraft:s"[..],
        ] {
            assert!(Reader::new(ok).identifier().is_ok(), "{ok:?}");
        }
        for bad in [
            &b"\x09A:b/c.dat"[..],  // uppercase namespace
            &b"\x05a b:c"[..],      // space
            &b"\x03a::"[..],        // empty path
            &b"\x04:a/b"[..],       // empty namespace with colon
            &b"\x00"[..],           // empty identifier
            &b"\x04a:\xc3\xa9"[..], // non-ASCII path
        ] {
            assert_eq!(
                Reader::new(bad).identifier(),
                Err(Error::BadFrame),
                "{bad:?}"
            );
        }
    }
}
