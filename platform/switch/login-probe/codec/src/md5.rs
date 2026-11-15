//! MD5 for exactly the one padded block a Java offline UUID needs.
//!
//! `"OfflinePlayer:"` is 14 bytes and a username is at most 16, so the message
//! is at most 30 bytes and the padded form fits a single 64-byte block: no
//! allocation, and no floating point (the round constants are the fixed RFC
//! 1321 table, not values computed from `sin`).

/// Java's `UUID.nameUUIDFromBytes` prefix for offline-mode players.
pub const PREFIX: &[u8] = b"OfflinePlayer:";

const K: [u32; 64] = [
    0xd76a_a478,
    0xe8c7_b756,
    0x2420_70db,
    0xc1bd_ceee,
    0xf57c_0faf,
    0x4787_c62a,
    0xa830_4613,
    0xfd46_9501,
    0x6980_98d8,
    0x8b44_f7af,
    0xffff_5bb1,
    0x895c_d7be,
    0x6b90_1122,
    0xfd98_7193,
    0xa679_438e,
    0x49b4_0821,
    0xf61e_2562,
    0xc040_b340,
    0x265e_5a51,
    0xe9b6_c7aa,
    0xd62f_105d,
    0x0244_1453,
    0xd8a1_e681,
    0xe7d3_fbc8,
    0x21e1_cde6,
    0xc337_07d6,
    0xf4d5_0d87,
    0x455a_14ed,
    0xa9e3_e905,
    0xfcef_a3f8,
    0x676f_02d9,
    0x8d2a_4c8a,
    0xfffa_3942,
    0x8771_f681,
    0x6d9d_6122,
    0xfde5_380c,
    0xa4be_ea44,
    0x4bde_cfa9,
    0xf6bb_4b60,
    0xbebf_bc70,
    0x289b_7ec6,
    0xeaa1_27fa,
    0xd4ef_3085,
    0x0488_1d05,
    0xd9d4_d039,
    0xe6db_99e5,
    0x1fa2_7cf8,
    0xc4ac_5665,
    0xf429_2244,
    0x432a_ff97,
    0xab94_23a7,
    0xfc93_a039,
    0x655b_59c3,
    0x8f0c_cc92,
    0xffef_f47d,
    0x8584_5dd1,
    0x6fa8_7e4f,
    0xfe2c_e6e0,
    0xa301_4314,
    0x4e08_11a1,
    0xf753_7e82,
    0xbd3a_f235,
    0x2ad7_d2bb,
    0xeb86_d391,
];

const S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// `UUID.nameUUIDFromBytes("OfflinePlayer:" + name)`: MD5, then version 3 and
/// the IETF variant bits, in big-endian byte order (as the wire UUID).
///
/// A name longer than 30 bytes is outside the login contract and yields zeros;
/// the C ABI rejects such names before calling this.
pub fn offline_uuid(name: &[u8]) -> [u8; 16] {
    if name.len() > 30 {
        return [0u8; 16];
    }
    let mut message = [0u8; 30];
    message[..PREFIX.len()].copy_from_slice(PREFIX);
    message[PREFIX.len()..PREFIX.len() + name.len()].copy_from_slice(name);
    let mut uuid = md5_one_block(&message[..PREFIX.len() + name.len()]);
    uuid[6] = (uuid[6] & 0x0f) | 0x30;
    uuid[8] = (uuid[8] & 0x3f) | 0x80;
    uuid
}

/// MD5 of a message of at most 55 bytes, which is one block after padding.
/// A longer message is outside this module's contract and yields zeros.
pub fn md5_one_block(message: &[u8]) -> [u8; 16] {
    if message.len() > 55 {
        return [0u8; 16];
    }
    let mut block = [0u8; 64];
    block[..message.len()].copy_from_slice(message);
    block[message.len()] = 0x80;
    // RFC 1321 appends the bit length as a little-endian 64-bit integer.
    block[56..64].copy_from_slice(&(message.len() as u64).wrapping_mul(8).to_le_bytes());

    let mut words = [0u32; 16];
    for (i, word) in words.iter_mut().enumerate() {
        let bytes = &block[i * 4..i * 4 + 4];
        *word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    }

    let mut h = [0x6745_2301u32, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let (mut a, mut b, mut c, mut d) = (h[0], h[1], h[2], h[3]);
    for i in 0..64 {
        let (f, g) = match i {
            0..=15 => ((b & c) | (!b & d), i),
            16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
            32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
            _ => (c ^ (b | !d), (7 * i) % 16),
        };
        let sum = a.wrapping_add(f).wrapping_add(K[i]).wrapping_add(words[g]);
        a = d;
        d = c;
        c = b;
        b = b.wrapping_add(sum.rotate_left(S[i]));
    }
    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);

    let mut digest = [0u8; 16];
    for (i, word) in h.iter().enumerate() {
        digest[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    digest
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    fn hex(bytes: &[u8]) -> Vec<u8> {
        bytes.to_vec()
    }

    fn from_hex(text: &str) -> Vec<u8> {
        let digits: Vec<u8> = text
            .bytes()
            .map(|b| (b as char).to_digit(16).unwrap() as u8)
            .collect();
        digits
            .chunks(2)
            .map(|pair| pair[0] << 4 | pair[1])
            .collect()
    }

    #[test]
    fn md5_vectors_are_the_published_ones() {
        assert_eq!(
            md5_one_block(b"").to_vec(),
            from_hex("d41d8cd98f00b204e9800998ecf8427e")
        );
        assert_eq!(
            md5_one_block(b"abc").to_vec(),
            from_hex("900150983cd24fb0d6963f7d28e17f72")
        );
        assert_eq!(
            md5_one_block(b"message digest").to_vec(),
            from_hex("f96b697d7cb7938d525a2f31aaf161d0")
        );
        // 55 bytes: the largest message this single-block implementation accepts.
        let longest = [b'a'; 55];
        assert_eq!(
            md5_one_block(&longest).to_vec(),
            from_hex("ef1772b6dff9a122358552954ad0df65")
        );
    }

    #[test]
    fn offline_uuids_match_independent_vectors() {
        // Computed outside this crate with Python's hashlib and version/variant
        // fix-up; note v3 in the third group and the 8xxx/9xxx/a-b variant nibble.
        assert_eq!(
            hex(&offline_uuid(b"Notch")),
            from_hex("b50ad385829d3141a2167e7d7539ba7f")
        );
        assert_eq!(
            hex(&offline_uuid(b"Horizon")),
            from_hex("3206a86c66b13068b73c7baef2e98b41")
        );
        assert_eq!(
            hex(&offline_uuid(b"a")),
            from_hex("52428a0e1e303cb1976ce728b2614047")
        );
        // Longest accepted name (16 bytes).
        assert_eq!(
            hex(&offline_uuid(b"abcdefghijklmnop")),
            from_hex("9dc76558520f356086b0b5c7c500848b")
        );
    }
}
