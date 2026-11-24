//! Binary snapshot format for normalized column data.
//!
//! CTCOL001 format: 48-byte header + sections*4096 block IDs + sections*64 biome IDs.
//! All integers little-endian. Header: magic "CTCOL001" (8), protocol 777 (4), format 1 (4),
//! chunk_x (4), chunk_z (4), min_y (4), sections (4), blocks_written (4), biomes_written (4),
//! reserved (8). Then exactly sections*4096 LE u32 block-state IDs, sections*64 LE u32 biome IDs.
//!
//! No light retention, name mapping or readiness assumptions. Pure raw u32 IDs.

fn read_u32_le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn read_i32_le(bytes: &[u8]) -> i32 {
    i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn write_u32_le(out: &mut [u8], value: u32) {
    out.copy_from_slice(&value.to_le_bytes());
}

fn write_i32_le(out: &mut [u8], value: i32) {
    out.copy_from_slice(&value.to_le_bytes());
}

/// Magic header bytes for CTCOL001.
pub const MAGIC: &[u8; 8] = b"CTCOL001";

/// Protocol version this format pins: Java 26.3 (777).
pub const PROTOCOL: u32 = 777;

/// Format version: 1.
pub const FORMAT: u32 = 1;

/// Header size in bytes: 48.
pub const HEADER_SIZE: usize = 48;

/// Reserved bytes at the end of the header.
pub const RESERVED_SIZE: usize = 8;

/// Maximum sections supported: 256.
pub const MAX_SECTIONS: usize = 256;

/// Maximum blocks per column: 256 * 4096.
pub const MAX_BLOCKS: usize = MAX_SECTIONS * 4096;

/// Maximum biomes per column: 256 * 64.
pub const MAX_BIOMES: usize = MAX_SECTIONS * 64;

/// Maximum file size: 48 + 256 * (4096 + 64) * 4 bytes.
pub const MAX_FILE_SIZE: usize = HEADER_SIZE + MAX_BLOCKS * 4 + MAX_BIOMES * 4;

/// C status codes matching column codec conventions.
pub const SNAP_OK: i32 = 0;
pub const SNAP_BAD_ARGUMENT: i32 = -1;
pub const SNAP_OUTPUT_SMALL: i32 = -2;
pub const SNAP_BAD_FRAME: i32 = -3;

/// Max file size constant (duplicate for header consistency).
pub const SNAP_MAX_FILE_SIZE: u32 = MAX_FILE_SIZE as u32;
/// Max sections constant.
pub const SNAP_MAX_SECTIONS: u32 = MAX_SECTIONS as u32;
/// Max blocks constant.
pub const SNAP_MAX_BLOCKS: u32 = MAX_BLOCKS as u32;
/// Max biomes constant.
pub const SNAP_MAX_BIOMES: u32 = MAX_BIOMES as u32;
/// Header size constant.
pub const SNAP_HEADER_SIZE: u32 = HEADER_SIZE as u32;

/// Header fields in order: magic, protocol, format, chunk_x, chunk_z, min_y,
/// sections, blocks_written, biomes_written, reserved.
const MAGIC_OFFSET: usize = 0;
const PROTOCOL_OFFSET: usize = 8;
const FORMAT_OFFSET: usize = 12;
const CHUNK_X_OFFSET: usize = 16;
const CHUNK_Z_OFFSET: usize = 20;
const MIN_Y_OFFSET: usize = 24;
const SECTIONS_OFFSET: usize = 28;
const BLOCKS_WRITTEN_OFFSET: usize = 32;
const BIOMES_WRITTEN_OFFSET: usize = 36;
const RESERVED_OFFSET: usize = 40;

/// Decode a CTCOL001 snapshot header.
///
/// Returns `(protocol, format, chunk_x, chunk_z, min_y, sections, blocks_written, biomes_written)`
/// on success. Validates magic, protocol, format, bounds and reserved bytes.
/// Returns `Err(-3)` for a truncated or malformed header.
#[allow(clippy::type_complexity)]
pub fn decode_header(data: &[u8]) -> Result<(u32, u32, i32, i32, i32, u32, u32, u32), i32> {
    if data.len() < HEADER_SIZE {
        return Err(-3);
    }

    let magic = &data[MAGIC_OFFSET..MAGIC_OFFSET + 8];
    if magic != MAGIC {
        return Err(-3);
    }

    let protocol = read_u32_le(&data[PROTOCOL_OFFSET..PROTOCOL_OFFSET + 4]);
    if protocol != PROTOCOL {
        return Err(-3);
    }

    let format = read_u32_le(&data[FORMAT_OFFSET..FORMAT_OFFSET + 4]);
    if format != FORMAT {
        return Err(-3);
    }

    let chunk_x = read_i32_le(&data[CHUNK_X_OFFSET..CHUNK_X_OFFSET + 4]);
    let chunk_z = read_i32_le(&data[CHUNK_Z_OFFSET..CHUNK_Z_OFFSET + 4]);
    let min_y = read_i32_le(&data[MIN_Y_OFFSET..MIN_Y_OFFSET + 4]);

    let sections = read_u32_le(&data[SECTIONS_OFFSET..SECTIONS_OFFSET + 4]);
    if sections == 0 || sections > MAX_SECTIONS as u32 {
        return Err(-3);
    }

    let blocks_written = read_u32_le(&data[BLOCKS_WRITTEN_OFFSET..BLOCKS_WRITTEN_OFFSET + 4]);
    if blocks_written != sections * 4096 {
        return Err(-3);
    }

    let biomes_written = read_u32_le(&data[BIOMES_WRITTEN_OFFSET..BIOMES_WRITTEN_OFFSET + 4]);
    if biomes_written != sections * 64 {
        return Err(-3);
    }

    // Verify reserved bytes are zero
    let reserved = &data[RESERVED_OFFSET..RESERVED_OFFSET + RESERVED_SIZE];
    if reserved.iter().any(|&b| b != 0) {
        return Err(-3);
    }

    // Validate min_y bounds: must be multiple of 16, and min_y + sections*16 must fit i32
    if min_y % 16 != 0 {
        return Err(-3);
    }
    let height = sections as i32 * 16;
    if min_y.checked_add(height).is_none() {
        return Err(-3);
    }

    Ok((
        protocol,
        format,
        chunk_x,
        chunk_z,
        min_y,
        sections,
        blocks_written,
        biomes_written,
    ))
}

/// Encode a CTCOL001 snapshot header into `out`.
///
/// `out` must have at least `HEADER_SIZE` bytes. Returns `Ok(HEADER_SIZE)` on success.
/// `Err(-1)` for bad argument (null/short), `Err(-3)` on invalid bounds.
pub fn encode_header(
    chunk_x: i32,
    chunk_z: i32,
    min_y: i32,
    sections: u32,
    blocks_written: u32,
    biomes_written: u32,
    out: &mut [u8],
) -> Result<usize, i32> {
    if out.len() < HEADER_SIZE {
        return Err(-1);
    }

    // Validate inputs
    if sections == 0 || sections > MAX_SECTIONS as u32 {
        return Err(-3);
    }
    if blocks_written != sections * 4096 {
        return Err(-3);
    }
    if biomes_written != sections * 64 {
        return Err(-3);
    }
    if min_y % 16 != 0 {
        return Err(-3);
    }
    let height = sections as i32 * 16;
    if min_y.checked_add(height).is_none() {
        return Err(-3);
    }

    out[MAGIC_OFFSET..MAGIC_OFFSET + 8].copy_from_slice(MAGIC);
    write_u32_le(&mut out[PROTOCOL_OFFSET..PROTOCOL_OFFSET + 4], PROTOCOL);
    write_u32_le(&mut out[FORMAT_OFFSET..FORMAT_OFFSET + 4], FORMAT);
    write_i32_le(&mut out[CHUNK_X_OFFSET..CHUNK_X_OFFSET + 4], chunk_x);
    write_i32_le(&mut out[CHUNK_Z_OFFSET..CHUNK_Z_OFFSET + 4], chunk_z);
    write_i32_le(&mut out[MIN_Y_OFFSET..MIN_Y_OFFSET + 4], min_y);
    write_u32_le(&mut out[SECTIONS_OFFSET..SECTIONS_OFFSET + 4], sections);
    write_u32_le(
        &mut out[BLOCKS_WRITTEN_OFFSET..BLOCKS_WRITTEN_OFFSET + 4],
        blocks_written,
    );
    write_u32_le(
        &mut out[BIOMES_WRITTEN_OFFSET..BIOMES_WRITTEN_OFFSET + 4],
        biomes_written,
    );

    // Zero reserved bytes
    out[RESERVED_OFFSET..RESERVED_OFFSET + RESERVED_SIZE].fill(0);

    Ok(HEADER_SIZE)
}

/// Encode a complete CTCOL001 snapshot.
///
/// `blocks` and `biomes` are the caller's raw u32 IDs. `out` receives the binary
/// file. Input arrays must have exactly `sections * 4096`/`sections * 64`
/// elements. Output after the complete file is untouched. Returns its byte size
/// or `Err(status)` without modifying output on failure.
///
/// Statuses: 0 success, `-1` bad argument, `-2` output too small, `-3` bad frame.
pub fn encode_snapshot(
    chunk_x: i32,
    chunk_z: i32,
    min_y: i32,
    sections: u32,
    blocks: &[u32],
    biomes: &[u32],
    out: &mut [u8],
) -> Result<usize, i32> {
    if sections == 0 || sections > MAX_SECTIONS as u32 {
        return Err(-3);
    }
    let blocks_written = sections as usize * 4096;
    let biomes_written = sections as usize * 64;
    // Validate into local storage: no caller output changes until every input
    // and the entire output capacity have been checked.
    let mut header = [0; HEADER_SIZE];
    let mut written = encode_header(
        chunk_x,
        chunk_z,
        min_y,
        sections,
        blocks_written as u32,
        biomes_written as u32,
        &mut header,
    )?;
    debug_assert_eq!(written, HEADER_SIZE);

    if blocks.len() != blocks_written || biomes.len() != biomes_written {
        return Err(-1);
    }
    if out.len() < header_size(sections) {
        return Err(-2);
    }
    out[..HEADER_SIZE].copy_from_slice(&header);

    // Write block IDs (LE u32, sections*4096 elements).
    let block_bytes = &mut out[written..written + blocks_written * 4];
    for (i, block) in blocks.iter().take(blocks_written).enumerate() {
        let offset = i * 4;
        write_u32_le(&mut block_bytes[offset..offset + 4], *block);
    }
    written += blocks_written * 4;

    // Write biome IDs (LE u32, sections*64 elements).
    let biome_bytes = &mut out[written..written + biomes_written * 4];
    for (i, biome) in biomes.iter().enumerate().take(biomes_written) {
        let offset = i * 4;
        write_u32_le(&mut biome_bytes[offset..offset + 4], *biome);
    }
    written += biomes_written * 4;

    Ok(written)
}

/// Decode a complete CTCOL001 snapshot.
///
/// `blocks` and `biomes` are caller-owned arrays receiving IDs. Only the first
/// `sections * 4096`/`sections * 64` elements are written; tail is untouched.
/// Returns `Ok((chunk_x, chunk_z, min_y, sections, blocks_written, biomes_written))`
/// or `Err(status)`.
///
/// Statuses: 0 success, `-1` bad argument, `-2` output too small, `-3` bad frame.
pub fn decode_snapshot(
    data: &[u8],
    blocks: &mut [u32],
    biomes: &mut [u32],
) -> Result<(i32, i32, i32, u32, u32, u32), i32> {
    if data.is_empty() || data.len() > MAX_FILE_SIZE {
        return Err(-1);
    }

    let (_protocol, _format, chunk_x, chunk_z, min_y, sections, blocks_written, biomes_written) =
        decode_header(data)?;

    if data.len() != header_size(sections) {
        return Err(-3);
    }

    let blocks_needed = sections as usize * 4096;
    let biomes_needed = sections as usize * 64;
    if blocks.len() < blocks_needed || biomes.len() < biomes_needed {
        return Err(-2);
    }

    // Read block IDs
    let mut offset = HEADER_SIZE;
    for block in blocks.iter_mut().take(blocks_needed) {
        *block = read_u32_le(&data[offset..offset + 4]);
        offset += 4;
    }

    // Read biome IDs
    for biome in biomes.iter_mut().take(biomes_needed) {
        *biome = read_u32_le(&data[offset..offset + 4]);
        offset += 4;
    }

    Ok((
        chunk_x,
        chunk_z,
        min_y,
        sections,
        blocks_written,
        biomes_written,
    ))
}

/// Calculate file size for given sections (header + data).
pub fn header_size(sections: u32) -> usize {
    HEADER_SIZE + (sections as usize * 4096 + sections as usize * 64) * 4
}

/// Calculate file size for given sections.
#[cfg(test)]
pub fn size_of_sections(sections: u32) -> usize {
    HEADER_SIZE + (sections as usize * 4096 + sections as usize * 64) * 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    const SENTINEL_BLOCK: u32 = 0xDEAD_BEEF;
    const SENTINEL_BIOME: u32 = 0xBEEF_1234;

    #[test]
    fn malformed_extents_leave_all_outputs_unchanged() {
        let blocks = vec![0x1234_5678; 4097];
        let biomes = vec![0x8765_4321; 65];
        let sentinel = vec![0xA5; header_size(1) + 1];
        let mut file = sentinel.clone();
        for (block_count, biome_count) in [(4095, 64), (4097, 64), (4096, 63), (4096, 65)] {
            assert_eq!(
                encode_snapshot(
                    0,
                    0,
                    -64,
                    1,
                    &blocks[..block_count],
                    &biomes[..biome_count],
                    &mut file
                ),
                Err(SNAP_BAD_ARGUMENT)
            );
            assert_eq!(file, sentinel);
        }
        let size =
            encode_snapshot(0, 0, -64, 1, &blocks[..4096], &biomes[..64], &mut file).unwrap();
        let mut output_blocks = vec![SENTINEL_BLOCK; 4096];
        let mut output_biomes = vec![SENTINEL_BIOME; 64];
        for length in [0, 47, 48, size - 1, size + 1] {
            assert!(
                decode_snapshot(&file[..length], &mut output_blocks, &mut output_biomes).is_err()
            );
            assert!(output_blocks.iter().all(|v| *v == SENTINEL_BLOCK));
            assert!(output_biomes.iter().all(|v| *v == SENTINEL_BIOME));
        }
        file[32..36].copy_from_slice(&4095u32.to_le_bytes());
        assert_eq!(decode_header(&file), Err(SNAP_BAD_FRAME));
    }

    fn make_blocks(sections: u32, fill: u32) -> Vec<u32> {
        vec![fill; sections as usize * 4096]
    }

    fn make_biomes(sections: u32, fill: u32) -> Vec<u32> {
        vec![fill; sections as usize * 64]
    }

    fn verify_header(
        data: &[u8],
        expected_sections: u32,
        expected_blocks: u32,
        expected_biomes: u32,
    ) {
        assert_eq!(&data[MAGIC_OFFSET..MAGIC_OFFSET + 8], MAGIC);
        assert_eq!(
            read_u32_le(&data[PROTOCOL_OFFSET..PROTOCOL_OFFSET + 4]),
            PROTOCOL
        );
        assert_eq!(read_u32_le(&data[FORMAT_OFFSET..FORMAT_OFFSET + 4]), FORMAT);
        assert_eq!(
            read_u32_le(&data[SECTIONS_OFFSET..SECTIONS_OFFSET + 4]),
            expected_sections
        );
        assert_eq!(
            read_u32_le(&data[BLOCKS_WRITTEN_OFFSET..BLOCKS_WRITTEN_OFFSET + 4]),
            expected_blocks
        );
        assert_eq!(
            read_u32_le(&data[BIOMES_WRITTEN_OFFSET..BIOMES_WRITTEN_OFFSET + 4]),
            expected_biomes
        );
        assert!(data[RESERVED_OFFSET..RESERVED_OFFSET + RESERVED_SIZE]
            .iter()
            .all(|&b| b == 0));
    }

    #[test]
    fn encode_decode_roundtrip_single_section() {
        let blocks = make_blocks(1, 0x1234_5678);
        let biomes = make_biomes(1, 0xABCD_EF01);

        let sections = 1u32;
        let blocks_written = 4096u32;
        let biomes_written = 64u32;

        let file_size = header_size(sections);
        let mut out = vec![0u8; file_size];

        let written = encode_snapshot(-3, 7, -16, sections, &blocks, &biomes, &mut out).unwrap();
        assert_eq!(written, file_size);

        verify_header(&out, sections, blocks_written, biomes_written);

        let mut decoded_blocks = vec![SENTINEL_BLOCK; 4096];
        let mut decoded_biomes = vec![SENTINEL_BIOME; 64];
        let (cx, cz, my, sec, bw, biw) =
            decode_snapshot(&out, &mut decoded_blocks, &mut decoded_biomes).unwrap();

        assert_eq!(cx, -3);
        assert_eq!(cz, 7);
        assert_eq!(my, -16);
        assert_eq!(sec, 1);
        assert_eq!(bw, 4096);
        assert_eq!(biw, 64);

        assert_eq!(decoded_blocks, blocks);
        assert_eq!(decoded_biomes, biomes);
    }

    #[test]
    fn encode_decode_roundtrip_multiple_sections() {
        let sections = 3u32;
        let blocks = make_blocks(sections, 0xCAFEBABE);
        let biomes = make_biomes(sections, 0xDEADBEEF);

        let file_size = header_size(sections);
        let mut out = vec![0u8; file_size];

        let written = encode_snapshot(5, -9, -64, sections, &blocks, &biomes, &mut out).unwrap();
        assert_eq!(written, file_size);

        let mut decoded_blocks = vec![SENTINEL_BLOCK; 3 * 4096];
        let mut decoded_biomes = vec![SENTINEL_BIOME; 3 * 64];
        let (cx, cz, my, sec, bw, biw) =
            decode_snapshot(&out, &mut decoded_blocks, &mut decoded_biomes).unwrap();

        assert_eq!(cx, 5);
        assert_eq!(cz, -9);
        assert_eq!(my, -64);
        assert_eq!(sec, 3);
        assert_eq!(bw, 3 * 4096);
        assert_eq!(biw, 3 * 64);

        assert_eq!(decoded_blocks, blocks);
        assert_eq!(decoded_biomes, biomes);
    }

    #[test]
    fn negative_chunk_coordinates_and_min_y() {
        let sections = 2u32;
        let blocks = make_blocks(sections, 0xABCD1234);
        let biomes = make_biomes(sections, 0x5678EFAB);

        let file_size = header_size(sections);
        let mut out = vec![0u8; file_size];

        let written = encode_snapshot(
            i32::MIN,
            i32::MAX,
            i32::MIN,
            sections,
            &blocks,
            &biomes,
            &mut out,
        )
        .unwrap();
        assert_eq!(written, file_size);

        let mut decoded_blocks = vec![SENTINEL_BLOCK; 2 * 4096];
        let mut decoded_biomes = vec![SENTINEL_BIOME; 2 * 64];
        let (cx, cz, my, sec, _, _) =
            decode_snapshot(&out, &mut decoded_blocks, &mut decoded_biomes).unwrap();

        assert_eq!(cx, i32::MIN);
        assert_eq!(cz, i32::MAX);
        assert_eq!(my, i32::MIN);
        assert_eq!(sec, 2);

        assert_eq!(decoded_blocks, blocks);
        assert_eq!(decoded_biomes, biomes);
    }

    #[test]
    fn min_y_must_be_multiple_of_16() {
        let sections = 1u32;
        let blocks = make_blocks(sections, 1);
        let biomes = make_biomes(sections, 1);

        let file_size = header_size(sections);
        let mut out = vec![0u8; file_size];

        // Valid: min_y = 0
        let written = encode_snapshot(0, 0, 0, sections, &blocks, &biomes, &mut out).unwrap();
        assert_eq!(written, file_size);

        // Invalid: min_y = 1
        let mut out2 = vec![0u8; file_size];
        let result = encode_snapshot(0, 0, 1, sections, &blocks, &biomes, &mut out2);
        assert_eq!(result, Err(-3));
    }

    #[test]
    fn min_y_plus_sections_must_fit_i32() {
        let blocks = make_blocks(256, 1);
        let biomes = make_biomes(256, 1);

        let file_size = header_size(256);
        let mut out = vec![0u8; file_size];

        // Valid: max_y = 2048 (256 sections * 16)
        let written = encode_snapshot(0, 0, 0, 256, &blocks, &biomes, &mut out).unwrap();
        assert_eq!(written, file_size);

        // Invalid: min_y + sections*16 overflows
        let mut out2 = vec![0u8; file_size];
        let result = encode_snapshot(0, 0, i32::MAX - 15, 256, &blocks, &biomes, &mut out2);
        assert_eq!(result, Err(-3));
    }

    #[test]
    fn reserved_must_be_zero() {
        let sections = 1u32;
        let blocks = make_blocks(sections, 1);
        let biomes = make_biomes(sections, 1);

        let file_size = header_size(sections);
        let mut out = vec![0u8; file_size];

        let written = encode_snapshot(0, 0, 0, sections, &blocks, &biomes, &mut out).unwrap();
        assert_eq!(written, file_size);

        // Corrupt reserved bytes
        out[RESERVED_OFFSET] = 1;

        let mut decoded_blocks = vec![SENTINEL_BLOCK; 4096];
        let mut decoded_biomes = vec![SENTINEL_BIOME; 64];
        let result = decode_snapshot(&out, &mut decoded_blocks, &mut decoded_biomes);
        assert_eq!(result, Err(-3));
    }

    #[test]
    fn wrong_magic_returns_bad_frame() {
        let sections = 1u32;
        let mut blocks = make_blocks(sections, 1);
        let mut biomes = make_biomes(sections, 1);

        let mut out = vec![0u8; header_size(sections)];
        out[0] = b'X';

        let result = decode_snapshot(&out, &mut blocks, &mut biomes);
        assert_eq!(result, Err(-3));
    }

    #[test]
    fn wrong_protocol_returns_bad_frame() {
        let sections = 1u32;
        let mut blocks = make_blocks(sections, 1);
        let mut biomes = make_biomes(sections, 1);

        let mut out = vec![0u8; header_size(sections)];
        out[MAGIC_OFFSET..MAGIC_OFFSET + 8].copy_from_slice(MAGIC);
        write_u32_le(&mut out[PROTOCOL_OFFSET..PROTOCOL_OFFSET + 4], 12345);

        let result = decode_snapshot(&out, &mut blocks, &mut biomes);
        assert_eq!(result, Err(-3));
    }

    #[test]
    fn wrong_format_returns_bad_frame() {
        let sections = 1u32;
        let mut blocks = make_blocks(sections, 1);
        let mut biomes = make_biomes(sections, 1);

        let mut out = vec![0u8; header_size(sections)];
        out[MAGIC_OFFSET..MAGIC_OFFSET + 8].copy_from_slice(MAGIC);
        write_u32_le(&mut out[PROTOCOL_OFFSET..PROTOCOL_OFFSET + 4], PROTOCOL);
        write_u32_le(&mut out[FORMAT_OFFSET..FORMAT_OFFSET + 4], 999);

        let result = decode_snapshot(&out, &mut blocks, &mut biomes);
        assert_eq!(result, Err(-3));
    }

    #[test]
    fn short_input_returns_bad_frame() {
        let mut blocks = vec![SENTINEL_BLOCK; 4096];
        let mut biomes = vec![SENTINEL_BIOME; 64];

        // Empty data is bad argument (no data provided)
        let result = decode_snapshot(&[], &mut blocks, &mut biomes);
        assert_eq!(result, Err(-1));

        // 47 bytes is too short for header (bad frame)
        let result = decode_snapshot(&[0u8; 47], &mut blocks, &mut biomes);
        assert_eq!(result, Err(-3));
    }

    #[test]
    fn output_capacity_must_suffice() {
        let sections = 2u32;
        let blocks = make_blocks(sections, 1);
        let biomes = make_biomes(sections, 1);

        let file_size = header_size(sections);
        let mut out = vec![0u8; file_size];
        encode_snapshot(0, 0, 0, sections, &blocks, &biomes, &mut out).unwrap();

        // Short block array
        let mut short_blocks = vec![SENTINEL_BLOCK; 4095];
        let mut biomes = make_biomes(sections, 1);
        let result = decode_snapshot(&out, &mut short_blocks, &mut biomes);
        assert_eq!(result, Err(-2));

        // Short biome array
        let mut blocks = make_blocks(sections, 1);
        let mut short_biomes = vec![SENTINEL_BIOME; 63];
        let result = decode_snapshot(&out, &mut blocks, &mut short_biomes);
        assert_eq!(result, Err(-2));
    }

    #[test]
    fn blocks_biomes_tails_untouched() {
        let sections = 1u32;
        let blocks = make_blocks(sections, 0x1111_1111);
        let biomes = make_biomes(sections, 0x2222_2222);

        let file_size = header_size(sections);
        let mut out = vec![0u8; file_size];
        encode_snapshot(0, 0, 0, sections, &blocks, &biomes, &mut out).unwrap();

        let mut decoded_blocks = vec![SENTINEL_BLOCK; 4096 + 100];
        let mut decoded_biomes = vec![SENTINEL_BIOME; 64 + 100];

        let (_, _, _, _, bw, biw) =
            decode_snapshot(&out, &mut decoded_blocks, &mut decoded_biomes).unwrap();

        assert_eq!(bw, 4096);
        assert_eq!(biw, 64);

        // Only first 4096 blocks changed
        assert_eq!(&decoded_blocks[0..4096], &blocks);
        assert_eq!(&decoded_blocks[4096..], &vec![SENTINEL_BLOCK; 100]);

        // Only first 64 biomes changed
        assert_eq!(&decoded_biomes[0..64], &biomes);
        assert_eq!(&decoded_biomes[64..], &vec![SENTINEL_BIOME; 100]);
    }

    #[test]
    fn header_size_calculation() {
        assert_eq!(HEADER_SIZE, 48);
        assert_eq!(header_size(1), 48 + 4096 * 4 + 64 * 4);
        assert_eq!(header_size(256), 48 + 256 * 4096 * 4 + 256 * 64 * 4);
        assert_eq!(header_size(256), MAX_FILE_SIZE);
    }

    #[test]
    fn encode_header_bounds() {
        let mut out = vec![0u8; HEADER_SIZE];

        // Valid: 256 sections
        let result = encode_header(0, 0, 0, 256, 256 * 4096, 256 * 64, &mut out);
        assert_eq!(result, Ok(HEADER_SIZE));

        // Invalid: 0 sections
        let result = encode_header(0, 0, 0, 0, 0, 0, &mut out);
        assert_eq!(result, Err(-3));

        // Invalid: 257 sections (overflow)
        let result = encode_header(0, 0, 0, 257, 257 * 4096, 257 * 64, &mut out);
        assert_eq!(result, Err(-3));

        // Invalid: blocks_written too large
        let result = encode_header(0, 0, 0, 1, 4097, 64, &mut out);
        assert_eq!(result, Err(-3));

        // Invalid: biomes_written too large
        let result = encode_header(0, 0, 0, 1, 4096, 65, &mut out);
        assert_eq!(result, Err(-3));

        // Invalid: min_y not multiple of 16
        let result = encode_header(0, 0, 1, 1, 4096, 64, &mut out);
        assert_eq!(result, Err(-3));
    }

    #[test]
    fn decode_header_returns_all_fields() {
        let sections = 3u32;
        let blocks = make_blocks(sections, 1);
        let biomes = make_biomes(sections, 1);

        let file_size = header_size(sections);
        let mut out = vec![0u8; file_size];
        encode_snapshot(-123, 456, -48, sections, &blocks, &biomes, &mut out).unwrap();

        let (protocol, format, chunk_x, chunk_z, min_y, sec, bw, biw) =
            decode_header(&out).unwrap();

        assert_eq!(protocol, PROTOCOL);
        assert_eq!(format, FORMAT);
        assert_eq!(chunk_x, -123);
        assert_eq!(chunk_z, 456);
        assert_eq!(min_y, -48);
        assert_eq!(sec, 3);
        assert_eq!(bw, 3 * 4096);
        assert_eq!(biw, 3 * 64);
    }

    #[test]
    fn decode_header_validates_bounds() {
        let sections = 1u32;
        let blocks = make_blocks(sections, 1);
        let biomes = make_biomes(sections, 1);

        let file_size = header_size(sections);
        let mut out = vec![0u8; file_size];
        encode_snapshot(0, 0, 0, sections, &blocks, &biomes, &mut out).unwrap();

        // Too many blocks_written
        write_u32_le(
            &mut out[BLOCKS_WRITTEN_OFFSET..BLOCKS_WRITTEN_OFFSET + 4],
            4097,
        );
        let result = decode_header(&out);
        assert_eq!(result, Err(-3));

        // Too many biomes_written
        write_u32_le(
            &mut out[BLOCKS_WRITTEN_OFFSET..BLOCKS_WRITTEN_OFFSET + 4],
            4096,
        );
        write_u32_le(
            &mut out[BIOMES_WRITTEN_OFFSET..BIOMES_WRITTEN_OFFSET + 4],
            65,
        );
        let result = decode_header(&out);
        assert_eq!(result, Err(-3));

        // Zero sections
        write_u32_le(
            &mut out[BIOMES_WRITTEN_OFFSET..BIOMES_WRITTEN_OFFSET + 4],
            64,
        );
        write_u32_le(&mut out[SECTIONS_OFFSET..SECTIONS_OFFSET + 4], 0);
        let result = decode_header(&out);
        assert_eq!(result, Err(-3));
    }
}
