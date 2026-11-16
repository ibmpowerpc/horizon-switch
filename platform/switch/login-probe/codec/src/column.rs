//! Normalized raw column copy for the Switch probe's
//! [`column_codec.h`](../../../include/column_codec.h) ABI.
//!
//! One complete, already-unframed and already-inflated play `LevelChunkWithLight`
//! (id 46) body is validated exactly like [`crate::world::decode_world_body`]
//! does, then its block-state and biome ids are unpacked, bottom-up, into two
//! caller-owned `u32` arrays:
//!
//! - `sections * 4096` block states in section order, then YZX order inside a
//!   section (`(y * 16 + z) * 16 + x`), exactly the container's storage order;
//! - `sections * 64` biome ids in section order, then YZX order over the
//!   4×4×4 biome cells.
//!
//! Each id is the normalized numeric registry id: a single-value container's
//! one id, a local palette's entry for the cell's index, or a global storage's
//! raw id. There are no names, air or opacity semantics.
//!
//! The copy is transactional. A first pass runs the shared chunk decoder with
//! a sink that keeps nothing, so every byte of the body — heightmaps, both
//! containers per section, block entities (NBT included), light masks and
//! layers — is validated, and the output sizes are known. Only then are the
//! caller's capacities compared (a short one is `OutputSmall`, before any
//! write) and a second pass unpack the same bytes into the arrays. The body is
//! borrowed and both capacity slices are only written on the second pass, so a
//! malformed body leaves the outputs untouched. The second pass cannot disagree
//! with the first: it parses the same immutable bytes with the same code.
//!
//! Nothing here allocates, recurses or uses a large stack object: the shared
//! decoder keeps one 256-entry palette and its readers on the stack, and the
//! body is walked in place. `unsafe` lives only in [`crate::column_ffi`].

use crate::play::PlState;
use crate::wire::Reader;
use crate::world::{
    decode_chunk_cells, i32_be, CellSink, CellTarget, ChunkSummary, IgnoreCells, WcState,
    WorldError, WC_MAX_BODY,
};

/// `COL_OK`.
pub const COL_OK: i32 = 0;
/// `COL_BAD_ARGUMENT`.
pub const COL_BAD_ARGUMENT: i32 = -1;
/// `COL_OUTPUT_SMALL`: a caller array cannot hold `sections * 4096` block
/// states or `sections * 64` biome ids.
pub const COL_OUTPUT_SMALL: i32 = -2;
/// `COL_BAD_FRAME`.
pub const COL_BAD_FRAME: i32 = -3;
/// `COL_UNSUPPORTED`.
pub const COL_UNSUPPORTED: i32 = -5;
/// `COL_LIMIT`: an NBT budget was exhausted.
pub const COL_LIMIT: i32 = -6;
/// `COL_MAX_BLOCKS`: 256 sections of 4096 block states.
pub const COL_MAX_BLOCKS: usize = 256 * 4096;
/// `COL_MAX_BIOMES`: 256 sections of 64 biome cells.
pub const COL_MAX_BIOMES: usize = 256 * 64;

/// Play `LevelChunkWithLight`, the only packet this copy reads.
const CHUNK_PACKET: i32 = 46;

/// One `struct col_info`: 24 bytes, alignment 4. Every field is zero on
/// failure.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColInfo {
    pub chunk_x: i32,
    pub chunk_z: i32,
    /// The selected dimension's `min_y`, bottom of the first section.
    pub min_y: i32,
    pub sections: u32,
    pub blocks_written: u32,
    pub biomes_written: u32,
}

impl ColInfo {
    /// An all-zero info, the failure shape.
    pub const fn zeroed() -> Self {
        Self {
            chunk_x: 0,
            chunk_z: 0,
            min_y: 0,
            sections: 0,
            blocks_written: 0,
            biomes_written: 0,
        }
    }
}

/// Why a column copy was rejected. The statuses map exactly like the world
/// decoder's, with [`ColumnError::OutputSmall`] added for a short array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnError {
    /// A pointer, length, capacity or scalar argument is out of contract.
    BadArgument,
    /// A caller array cannot hold the whole column.
    OutputSmall,
    /// The body is not a structurally valid chunk packet.
    BadFrame,
    /// Structurally valid, but not this packet, or a palette/registry cap is
    /// past what is retained.
    Unsupported,
    /// An NBT budget was exhausted.
    Limit,
}

impl From<WorldError> for ColumnError {
    fn from(error: WorldError) -> Self {
        match error {
            WorldError::BadArgument => ColumnError::BadArgument,
            WorldError::BadFrame => ColumnError::BadFrame,
            WorldError::Unsupported => ColumnError::Unsupported,
            WorldError::Limit => ColumnError::Limit,
        }
    }
}

/// The writing pass's cell destination: section-major YZX output slices.
struct WriteCells<'a> {
    blocks: &'a mut [u32],
    biomes: &'a mut [u32],
}

impl CellSink for WriteCells<'_> {
    fn uniform(&mut self, target: CellTarget, value: i32) {
        let id = value as u32;
        if target.blocks {
            let start = target.section * 4096;
            self.blocks[start..start + 4096].fill(id);
        } else {
            let start = target.section * 64;
            self.biomes[start..start + 64].fill(id);
        }
    }

    fn cell(&mut self, target: CellTarget, index: usize, value: i32) {
        let id = value as u32;
        if target.blocks {
            self.blocks[target.section * 4096 + index] = id;
        } else {
            self.biomes[target.section * 64 + index] = id;
        }
    }
}

/// Read the packet header every pass starts with: the id must be this module's
/// chunk packet and the two coordinates must exist.
fn chunk_reader(body: &[u8]) -> Result<(Reader<'_>, i32, i32), ColumnError> {
    let mut reader = Reader::new(body);
    let packet = reader.varint().map_err(|_| ColumnError::BadFrame)?;
    if packet < 0 {
        return Err(ColumnError::BadFrame);
    }
    if packet != CHUNK_PACKET {
        return Err(ColumnError::Unsupported);
    }
    let chunk_x = i32_be(&mut reader)?;
    let chunk_z = i32_be(&mut reader)?;
    Ok((reader, chunk_x, chunk_z))
}

/// Validate one complete play id 46 body and copy its normalized block-state
/// and biome ids into `blocks`/`biomes`.
///
/// `blocks` and `biomes` are the caller's capacities in `u32` elements, at
/// most [`COL_MAX_BLOCKS`]/[`COL_MAX_BIOMES`]. Only the first
/// `sections * 4096`/`sections * 64` elements are written; a larger tail is
/// untouched. A body that fails validation writes neither array.
pub fn decode_column_body(
    body: &[u8],
    state: &WcState,
    play: &PlState,
    blocks: &mut [u32],
    biomes: &mut [u32],
) -> Result<ColInfo, ColumnError> {
    if body.is_empty()
        || body.len() > WC_MAX_BODY
        || blocks.len() > COL_MAX_BLOCKS
        || biomes.len() > COL_MAX_BIOMES
    {
        return Err(ColumnError::BadArgument);
    }

    // First pass: the shared chunk decoder with a sink that keeps nothing.
    // It validates the whole body and leaves neither output touched.
    let (mut first, chunk_x, chunk_z) = chunk_reader(body)?;
    let summary: ChunkSummary = decode_chunk_cells(&mut first, state, play, &mut IgnoreCells)?;

    let blocks_needed = summary.sections * 4096;
    let biomes_needed = summary.sections * 64;
    if blocks.len() < blocks_needed || biomes.len() < biomes_needed {
        return Err(ColumnError::OutputSmall);
    }

    // Second pass: the same bytes, the same decoder, writing this time. The
    // parse is deterministic, so it cannot reject what the first pass accepted.
    let (mut second, second_x, second_z) = chunk_reader(body)?;
    debug_assert_eq!((second_x, second_z), (chunk_x, chunk_z));
    let written = decode_chunk_cells(&mut second, state, play, &mut WriteCells { blocks, biomes })?;
    debug_assert_eq!(written, summary);

    Ok(ColInfo {
        chunk_x,
        chunk_z,
        min_y: summary.min_y,
        sections: summary.sections as u32,
        blocks_written: blocks_needed as u32,
        biomes_written: biomes_needed as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play::PlState;
    use crate::wire::write_varint;
    use crate::world::{WcDimension, WcState};
    use crate::PLAY_DIMENSION_CAP;
    use std::vec;
    use std::vec::Vec;

    const SENTINEL_BLOCK: u32 = 0xDEAD_BEEF;
    const SENTINEL_BIOME: u32 = 0xBEEF_1234;

    fn varint(value: i32) -> Vec<u8> {
        let mut out = [0u8; 5];
        let len = write_varint(value, &mut out).unwrap();
        out[..len].to_vec()
    }

    fn push_varint(out: &mut Vec<u8>, value: i32) {
        out.extend_from_slice(&varint(value));
    }

    fn push_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
        push_varint(out, bytes.len() as i32);
        out.extend_from_slice(bytes);
    }

    fn single_container(id: i32) -> Vec<u8> {
        let mut out = vec![0];
        push_varint(&mut out, id);
        out
    }

    fn local_container(wire: u8, ids: &[i32], words: &[u64]) -> Vec<u8> {
        let mut out = vec![wire];
        push_varint(&mut out, ids.len() as i32);
        for id in ids {
            push_varint(&mut out, *id);
        }
        for word in words {
            out.extend_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn global_container(wire: u8, words: &[u64]) -> Vec<u8> {
        let mut out = vec![wire];
        for word in words {
            out.extend_from_slice(&word.to_be_bytes());
        }
        out
    }

    /// Pack `entries` values the way vanilla `SimpleBitStorage` does: `bits`
    /// wide, low value first inside each big-endian long, never crossing one.
    fn pack(bits: u32, entries: usize, values: impl Fn(usize) -> u64) -> Vec<u64> {
        let per = 64 / bits as usize;
        let mut words = vec![0u64; entries.div_ceil(per)];
        for index in 0..entries {
            let value = values(index);
            assert!(value < (1u64 << bits));
            words[index / per] |= value << ((index % per) * bits as usize);
        }
        words
    }

    fn section(non_empty: i16, fluids: i16, states: &[u8], biomes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&non_empty.to_be_bytes());
        out.extend_from_slice(&fluids.to_be_bytes());
        out.extend_from_slice(states);
        out.extend_from_slice(biomes);
        out
    }

    fn bitset(words: &[u64]) -> Vec<u8> {
        let mut bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        while bytes.last() == Some(&0) {
            bytes.pop();
        }
        let mut out = varint(bytes.len() as i32);
        out.extend_from_slice(&bytes);
        out
    }

    fn layers(count: usize, fill: u8) -> Vec<u8> {
        let mut out = varint(count as i32);
        for _ in 0..count {
            push_bytes(&mut out, &[fill; 2048]);
        }
        out
    }

    fn light_data(masks: [&[u64]; 4], sky: &[u8], block: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for mask in masks {
            out.extend_from_slice(&bitset(mask));
        }
        out.extend_from_slice(sky);
        out.extend_from_slice(block);
        out
    }

    fn empty_light() -> Vec<u8> {
        light_data([&[], &[], &[], &[]], &layers(0, 0), &layers(0, 0))
    }

    fn full_light() -> Vec<u8> {
        // Sky present on light section 0, block present on light section 1.
        light_data([&[1], &[2], &[], &[]], &layers(1, 0xAA), &layers(1, 0x55))
    }

    /// A complete chunk body with no heightmaps and no block entities.
    fn chunk_body(x: i32, z: i32, buffer: &[u8], light: &[u8]) -> Vec<u8> {
        let mut out = varint(46);
        out.extend_from_slice(&x.to_be_bytes());
        out.extend_from_slice(&z.to_be_bytes());
        out.extend_from_slice(&varint(0));
        push_bytes(&mut out, buffer);
        out.extend_from_slice(&varint(0));
        out.extend_from_slice(light);
        out
    }

    fn play_state(dimension_type: i32) -> PlState {
        PlState {
            position_bits: [0; 3],
            yaw_bits: 0,
            pitch_bits: 0,
            player_id: 0,
            dimension_type,
            game_mode: 0,
            teleport_id: 0,
            login_seen: 1,
            position_seen: 0,
            dimension: [0u8; PLAY_DIMENSION_CAP],
        }
    }

    fn selected_state(min_y: i32, height: i32, biomes: u32) -> WcState {
        let mut state = WcState::zeroed();
        state.dimension_count = 1;
        state.dimensions[0] = WcDimension {
            min_y,
            height,
            has_skylight: 1,
            valid: 1,
        };
        state.biome_count = biomes;
        state
    }

    /// Three sections with one single-value, one local and one global container
    /// for each kind: min_y -16, height 48, biome count 5.
    fn mixed_body(x: i32, z: i32) -> Vec<u8> {
        let mut buffer = Vec::new();
        // Section 0: single-value containers.
        buffer.extend_from_slice(&section(0, 0, &single_container(42), &single_container(3)));
        // Section 1: 4-bit local block palette and 1-bit local biome palette.
        buffer.extend_from_slice(&section(
            0,
            0,
            &local_container(1, &[7, 1_000_000], &pack(4, 4096, |i| (i & 1) as u64)),
            &local_container(1, &[0, 4], &pack(1, 64, |i| (i & 1) as u64)),
        ));
        // Section 2: global 12-bit block storage and global 4-bit biomes.
        buffer.extend_from_slice(&section(
            0,
            0,
            &global_container(12, &pack(12, 4096, |i| 3000 + (i % 1000) as u64)),
            &global_container(4, &pack(4, 64, |i| (i % 5) as u64)),
        ));
        chunk_body(x, z, &buffer, &empty_light())
    }

    fn expected_block(section: usize, index: usize) -> u32 {
        match section {
            0 => 42,
            1 if index & 1 == 0 => 7,
            1 => 1_000_000,
            2 => 3000 + (index % 1000) as u32,
            _ => panic!("unexpected section {section}"),
        }
    }

    fn expected_biome(section: usize, index: usize) -> u32 {
        match section {
            0 => 3,
            1 if index & 1 == 0 => 0,
            1 => 4,
            2 => (index % 5) as u32,
            _ => panic!("unexpected section {section}"),
        }
    }

    #[test]
    fn every_palette_mode_copies_bottom_up_with_negative_columns() {
        let state = selected_state(-16, 48, 5);
        let play = play_state(0);
        let body = mixed_body(-3, 7);
        let mut blocks = vec![SENTINEL_BLOCK; 3 * 4096 + 7];
        let mut biomes = vec![SENTINEL_BIOME; 3 * 64 + 5];
        let mut info = ColInfo::zeroed();

        let status = unsafe {
            crate::column_ffi::craftrustnx_column_copy(
                body.as_ptr(),
                body.len() as u32,
                &state,
                &play,
                blocks.as_mut_ptr(),
                blocks.len() as u32,
                biomes.as_mut_ptr(),
                biomes.len() as u32,
                &mut info,
            )
        };
        assert_eq!(status, COL_OK);

        for section in 0..3 {
            for index in 0..4096 {
                assert_eq!(
                    blocks[section * 4096 + index],
                    expected_block(section, index),
                    "block section {section} index {index}"
                );
            }
            for index in 0..64 {
                assert_eq!(
                    biomes[section * 64 + index],
                    expected_biome(section, index),
                    "biome section {section} index {index}"
                );
            }
        }
        // Only the written prefix changes; both tails stay exactly as they were.
        for cell in &blocks[3 * 4096..] {
            assert_eq!(*cell, SENTINEL_BLOCK);
        }
        for cell in &biomes[3 * 64..] {
            assert_eq!(*cell, SENTINEL_BIOME);
        }
        assert_eq!(
            info,
            ColInfo {
                chunk_x: -3,
                chunk_z: 7,
                min_y: -16,
                sections: 3,
                blocks_written: 3 * 4096,
                biomes_written: 3 * 64,
            }
        );
    }

    #[test]
    fn block_and_biome_cells_are_stored_in_yzx_order() {
        // One section, min_y 0. The only 4-bit block value 1 is at cell 291 =
        // (y 1, z 2, x 3) and the only biome value 1 is at cell 37.
        // The block palette maps index 1 to id 55, and the biome palette maps
        // index 1 to id 2. Biome count must be > 2 to accept id 2.
        let state = selected_state(0, 16, 3); // biome_count > 2 for id 2
        let play = play_state(0);
        // Generate exactly 256 words for 4-bit storage (4096 values / 16 values per long).
        let block_words = pack(4, 4096, |i| if i == 291 { 1 } else { 0 });
        // Generate exactly 1 word for 1-bit storage of 64 entries (64 values / 64 per long).
        let biome_words = pack(1, 64, |i| if i == 37 { 1 } else { 0 });
        let buffer = section(
            0,
            0,
            &local_container(4, &[0, 55], &block_words),
            &local_container(1, &[0, 2], &biome_words),
        );
        let body = chunk_body(0, 0, &buffer, &empty_light());

        let mut blocks = vec![SENTINEL_BLOCK; 4096];
        let mut biomes = vec![SENTINEL_BIOME; 64];
        let mut info = ColInfo::zeroed();
        let status = unsafe {
            crate::column_ffi::craftrustnx_column_copy(
                body.as_ptr(),
                body.len() as u32,
                &state,
                &play,
                blocks.as_mut_ptr(),
                blocks.len() as u32,
                biomes.as_mut_ptr(),
                biomes.len() as u32,
                &mut info,
            )
        };
        assert_eq!(status, COL_OK);

        // (y * 16 + z) * 16 + x for the block, and the 4×4×4 biome index.
        assert_eq!(blocks[291], 55);
        for index in [0, 1, 16, 256, 290, 292, 4095] {
            assert_eq!(blocks[index], 0, "block index {index}");
        }
        assert_eq!(biomes[37], 2);
        for index in [0, 1, 36, 38, 63] {
            assert_eq!(biomes[index], 0, "biome index {index}");
        }
        assert_eq!(
            info,
            ColInfo {
                chunk_x: 0,
                chunk_z: 0,
                min_y: 0,
                sections: 1,
                blocks_written: 4096,
                biomes_written: 64,
            }
        );
    }

    #[test]
    fn a_late_malformed_light_layer_leaves_outputs_and_inputs_untouched() {
        let state = selected_state(-16, 48, 5);
        let play = play_state(0);
        let mut body = mixed_body(5, -9);
        // Replace the empty light block with one sky and one block layer, then
        // cut the last byte: the failure is inside the final 2048-byte block
        // layer, after sections, block entities and every earlier light field
        // validated.
        let empty = empty_light();
        assert_eq!(empty.len(), 6);
        body.truncate(body.len() - empty.len());
        body.extend_from_slice(&full_light());
        let broken = &body[..body.len() - 1];

        let mut blocks = vec![SENTINEL_BLOCK; 3 * 4096];
        let mut biomes = vec![SENTINEL_BIOME; 3 * 64];
        let mut info = ColInfo {
            chunk_x: 11,
            chunk_z: 22,
            min_y: 33,
            sections: 4,
            blocks_written: 5,
            biomes_written: 6,
        };
        let state_before = state;
        let play_before = play;
        let status = unsafe {
            crate::column_ffi::craftrustnx_column_copy(
                broken.as_ptr(),
                broken.len() as u32,
                &state,
                &play,
                blocks.as_mut_ptr(),
                blocks.len() as u32,
                biomes.as_mut_ptr(),
                biomes.len() as u32,
                &mut info,
            )
        };
        assert_eq!(status, COL_BAD_FRAME);
        assert!(blocks.iter().all(|&cell| cell == SENTINEL_BLOCK));
        assert!(biomes.iter().all(|&cell| cell == SENTINEL_BIOME));
        assert_eq!(info, ColInfo::zeroed());
        assert_eq!(state, state_before);
        assert_eq!(play, play_before);
    }

    #[test]
    fn every_short_capacity_is_reported_before_any_write() {
        let state = selected_state(-16, 48, 5);
        let play = play_state(0);
        let body = mixed_body(0, 0);
        let blocks_needed = 3 * 4096;
        let biomes_needed = 3 * 64;

        let block_caps = [0usize, 1, 63, 1024, 4095, 8191, blocks_needed - 1];
        let biome_caps = [0usize, 1, 63, biomes_needed - 1];
        for &block_cap in &block_caps {
            for &biome_cap in &biome_caps {
                let mut blocks = vec![SENTINEL_BLOCK; block_cap];
                let mut biomes = vec![SENTINEL_BIOME; biome_cap];
                let mut info = ColInfo::zeroed();
                let status = unsafe {
                    crate::column_ffi::craftrustnx_column_copy(
                        body.as_ptr(),
                        body.len() as u32,
                        &state,
                        &play,
                        blocks.as_mut_ptr(),
                        blocks.len() as u32,
                        biomes.as_mut_ptr(),
                        biomes.len() as u32,
                        &mut info,
                    )
                };
                assert_eq!(status, COL_OUTPUT_SMALL, "{block_cap}/{biome_cap}");
                assert!(blocks.iter().all(|&cell| cell == SENTINEL_BLOCK));
                assert!(biomes.iter().all(|&cell| cell == SENTINEL_BIOME));
                assert_eq!(info, ColInfo::zeroed());
            }
        }

        // An adequate block array with a short biome array fails the same way.
        let cases = [
            (blocks_needed, biomes_needed - 1),
            (blocks_needed - 1, biomes_needed),
        ];
        for (block_cap, biome_cap) in cases {
            let mut blocks = vec![SENTINEL_BLOCK; block_cap];
            let mut biomes = vec![SENTINEL_BIOME; biome_cap];
            let mut info = ColInfo::zeroed();
            let status = unsafe {
                crate::column_ffi::craftrustnx_column_copy(
                    body.as_ptr(),
                    body.len() as u32,
                    &state,
                    &play,
                    blocks.as_mut_ptr(),
                    blocks.len() as u32,
                    biomes.as_mut_ptr(),
                    biomes.len() as u32,
                    &mut info,
                )
            };
            assert_eq!(status, COL_OUTPUT_SMALL, "{block_cap}/{biome_cap}");
            assert!(blocks.iter().all(|&cell| cell == SENTINEL_BLOCK));
            assert!(biomes.iter().all(|&cell| cell == SENTINEL_BIOME));
            assert_eq!(info, ColInfo::zeroed());
        }

        // Exactly the needed capacities succeed and fill every element.
        let mut blocks = vec![SENTINEL_BLOCK; blocks_needed];
        let mut biomes = vec![SENTINEL_BIOME; biomes_needed];
        let mut info = ColInfo::zeroed();
        let status = unsafe {
            crate::column_ffi::craftrustnx_column_copy(
                body.as_ptr(),
                body.len() as u32,
                &state,
                &play,
                blocks.as_mut_ptr(),
                blocks.len() as u32,
                biomes.as_mut_ptr(),
                biomes.len() as u32,
                &mut info,
            )
        };
        assert_eq!(status, COL_OK);
        assert_eq!(info.blocks_written, blocks_needed as u32);
        assert_eq!(info.biomes_written, biomes_needed as u32);
        assert!(blocks
            .iter()
            .enumerate()
            .all(|(index, &cell)| cell == expected_block(index / 4096, index % 4096)));
        assert!(biomes
            .iter()
            .enumerate()
            .all(|(index, &cell)| cell == expected_biome(index / 64, index % 64)));
    }

    #[test]
    fn info_layout_and_argument_bounds_match_the_header() {
        assert_eq!(core::mem::size_of::<ColInfo>(), 24);
        assert_eq!(core::mem::align_of::<ColInfo>(), 4);
        assert_eq!(COL_MAX_BLOCKS, 1_048_576);
        assert_eq!(COL_MAX_BIOMES, 16_384);
        assert_eq!(
            core::mem::size_of::<crate::world::WcState>(),
            528,
            "the world state is unchanged"
        );

        let state = selected_state(-16, 48, 5);
        let play = play_state(0);
        let body = mixed_body(0, 0);
        let mut blocks = vec![SENTINEL_BLOCK; 3 * 4096];
        let mut biomes = vec![SENTINEL_BIOME; 3 * 64];
        // info is only used later in the test, so we can initialize it later.

        #[allow(clippy::too_many_arguments)]
        unsafe fn copy(
            body: *const u8,
            length: u32,
            world: *const WcState,
            play: *const PlState,
            blocks: *mut u32,
            block_capacity: u32,
            biomes: *mut u32,
            biome_capacity: u32,
            info: *mut ColInfo,
        ) -> i32 {
            crate::column_ffi::craftrustnx_column_copy(
                body,
                length,
                world,
                play,
                blocks,
                block_capacity,
                biomes,
                biome_capacity,
                info,
            )
        }

        let ok_body = body.as_ptr();
        let len = body.len() as u32;
        let ok_world = &state;
        let ok_play = &play;
        let ok_blocks = blocks.as_mut_ptr();
        let ok_biomes = biomes.as_mut_ptr();
        let block_cap = blocks.len() as u32;
        let biome_cap = biomes.len() as u32;

        // A null info cannot be zeroed: it is the one argument checked first.
        assert_eq!(
            unsafe {
                copy(
                    ok_body,
                    len,
                    ok_world,
                    ok_play,
                    ok_blocks,
                    block_cap,
                    ok_biomes,
                    biome_cap,
                    core::ptr::null_mut(),
                )
            },
            COL_BAD_ARGUMENT
        );

        // Every other bad pointer leaves the info all-zero and arrays untouched.
        let mut info = ColInfo {
            chunk_x: 1,
            ..ColInfo::zeroed()
        };
        assert_eq!(
            unsafe {
                copy(
                    ok_body,
                    len,
                    core::ptr::null(),
                    ok_play,
                    ok_blocks,
                    block_cap,
                    ok_biomes,
                    biome_cap,
                    &mut info,
                )
            },
            COL_BAD_ARGUMENT
        );
        assert_eq!(info, ColInfo::zeroed());
        assert_eq!(
            unsafe {
                copy(
                    ok_body,
                    len,
                    ok_world,
                    core::ptr::null(),
                    ok_blocks,
                    block_cap,
                    ok_biomes,
                    biome_cap,
                    &mut info,
                )
            },
            COL_BAD_ARGUMENT
        );
        assert_eq!(info, ColInfo::zeroed());
        assert_eq!(
            unsafe {
                copy(
                    core::ptr::null(),
                    5,
                    ok_world,
                    ok_play,
                    ok_blocks,
                    block_cap,
                    ok_biomes,
                    biome_cap,
                    &mut info,
                )
            },
            COL_BAD_ARGUMENT
        );
        assert_eq!(info, ColInfo::zeroed());
        assert_eq!(
            unsafe {
                copy(
                    ok_body,
                    len,
                    ok_world,
                    ok_play,
                    core::ptr::null_mut(),
                    block_cap,
                    ok_biomes,
                    biome_cap,
                    &mut info,
                )
            },
            COL_BAD_ARGUMENT
        );
        assert_eq!(info, ColInfo::zeroed());
        assert_eq!(
            unsafe {
                copy(
                    ok_body,
                    len,
                    ok_world,
                    ok_play,
                    ok_blocks,
                    block_cap,
                    core::ptr::null_mut(),
                    biome_cap,
                    &mut info,
                )
            },
            COL_BAD_ARGUMENT
        );
        assert_eq!(info, ColInfo::zeroed());

        // Body lengths outside 1..=WC_MAX_BODY.
        for length in [0, (crate::world::WC_MAX_BODY + 1) as u32] {
            assert_eq!(
                unsafe {
                    copy(
                        ok_body, length, ok_world, ok_play, ok_blocks, block_cap, ok_biomes,
                        biome_cap, &mut info,
                    )
                },
                COL_BAD_ARGUMENT
            );
            assert_eq!(info, ColInfo::zeroed());
        }

        // Capacities above the header maximum are bad arguments, not writes.
        let mut block_cap_plus = vec![SENTINEL_BLOCK; COL_MAX_BLOCKS + 1];
        assert_eq!(
            unsafe {
                copy(
                    ok_body,
                    len,
                    ok_world,
                    ok_play,
                    block_cap_plus.as_mut_ptr(),
                    block_cap_plus.len() as u32,
                    ok_biomes,
                    biome_cap,
                    &mut info,
                )
            },
            COL_BAD_ARGUMENT
        );
        assert_eq!(info, ColInfo::zeroed());
        let mut biome_cap_plus = vec![SENTINEL_BIOME; COL_MAX_BIOMES + 1];
        assert_eq!(
            unsafe {
                copy(
                    ok_body,
                    len,
                    ok_world,
                    ok_play,
                    ok_blocks,
                    block_cap,
                    biome_cap_plus.as_mut_ptr(),
                    biome_cap_plus.len() as u32,
                    &mut info,
                )
            },
            COL_BAD_ARGUMENT
        );
        assert_eq!(info, ColInfo::zeroed());
        assert!(block_cap_plus.iter().all(|&cell| cell == SENTINEL_BLOCK));
        assert!(biome_cap_plus.iter().all(|&cell| cell == SENTINEL_BIOME));

        // Only play id 46 is copied: 49 (LightUpdate) and 45 (KeepAlive) are
        // structurally complete bodies of another kind.
        for packet in [49i32, 45] {
            let other = varint(packet);
            assert_eq!(
                unsafe {
                    copy(
                        other.as_ptr(),
                        other.len() as u32,
                        ok_world,
                        ok_play,
                        ok_blocks,
                        block_cap,
                        ok_biomes,
                        biome_cap,
                        &mut info,
                    )
                },
                COL_UNSUPPORTED,
                "packet {packet}"
            );
            assert_eq!(info, ColInfo::zeroed());
            assert!(blocks.iter().all(|&cell| cell == SENTINEL_BLOCK));
        }
        // A negative packet id and an empty body are malformed.
        let negative = [0xff, 0xff, 0xff, 0xff, 0x0f];
        assert_eq!(
            unsafe {
                copy(
                    negative.as_ptr(),
                    negative.len() as u32,
                    ok_world,
                    ok_play,
                    ok_blocks,
                    block_cap,
                    ok_biomes,
                    biome_cap,
                    &mut info,
                )
            },
            COL_BAD_FRAME
        );
        assert_eq!(info, ColInfo::zeroed());

        // Without a retained biome registry the chunk cannot be validated.
        let unsupported_state = selected_state(-16, 48, 0);
        assert_eq!(
            unsafe {
                copy(
                    ok_body,
                    len,
                    &unsupported_state,
                    ok_play,
                    ok_blocks,
                    block_cap,
                    ok_biomes,
                    biome_cap,
                    &mut info,
                )
            },
            COL_UNSUPPORTED
        );
        assert_eq!(info, ColInfo::zeroed());
        assert!(blocks.iter().all(|&cell| cell == SENTINEL_BLOCK));
        assert!(biomes.iter().all(|&cell| cell == SENTINEL_BIOME));
    }

    #[test]
    fn a_bad_palette_or_trailing_byte_is_a_bad_frame_with_no_writes() {
        let state = selected_state(0, 16, 3);
        let play = play_state(0);
        let good = {
            let buffer = section(
                0,
                0,
                &local_container(4, &[0, 55], &vec![0u64; 256]),
                &single_container(0),
            );
            chunk_body(0, 0, &buffer, &empty_light())
        };
        // A palette index past the two entries is a malformed container.
        let buffer = section(
            0,
            0,
            &local_container(4, &[0, 55], &vec![2u64; 256]),
            &single_container(0),
        );
        let bad_palette = chunk_body(0, 0, &buffer, &empty_light());
        let mut trailing = good.clone();
        trailing.push(0);

        for (body, expected) in [(&bad_palette, COL_BAD_FRAME), (&trailing, COL_BAD_FRAME)] {
            let mut blocks = vec![SENTINEL_BLOCK; 4096];
            let mut biomes = vec![SENTINEL_BIOME; 64];
            let mut info = ColInfo::zeroed();
            let status = unsafe {
                crate::column_ffi::craftrustnx_column_copy(
                    body.as_ptr(),
                    body.len() as u32,
                    &state,
                    &play,
                    blocks.as_mut_ptr(),
                    blocks.len() as u32,
                    biomes.as_mut_ptr(),
                    biomes.len() as u32,
                    &mut info,
                )
            };
            assert_eq!(status, expected);
            assert!(blocks.iter().all(|&cell| cell == SENTINEL_BLOCK));
            assert!(biomes.iter().all(|&cell| cell == SENTINEL_BIOME));
            assert_eq!(info, ColInfo::zeroed());
        }
    }
}
