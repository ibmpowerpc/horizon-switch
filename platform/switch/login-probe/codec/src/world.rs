//! Bounded world-data decoding for the Switch probe's
//! [`world_codec.h`](../../../include/world_codec.h) ABI.
//!
//! Two packet families are read:
//!
//! - configuration `RegistryData` (id 7): every entry's identifier and
//!   optional unnamed NBT value is walked, and only
//!   `minecraft:dimension_type` and `minecraft:worldgen/biome` are retained
//!   (as the fixed dimension table and a biome count). Other registries stay
//!   opaque metadata. Names are borrowed, never copied.
//! - play `LevelChunkWithLight` (46) and `LightUpdate` (49): the chunk buffer,
//!   sections, paletted containers, block entities, heightmaps and light data
//!   are structurally validated. Nothing is loaded, stored or simulated.
//!
//! ## 26.3 details this module follows
//!
//! - A section starts with **two** big-endian signed shorts, `non_empty` and
//!   `fluids`, each `0..=4096`.
//! - A paletted container's storage has **no** prefixed long count (the
//!   pre-1.21.5 form did). The container is a one-byte wire bit width, then the
//!   local palette (when the strategy uses one), then exactly
//!   `storage_len(bits, entries)` big-endian longs.
//! - Heightmaps are a `VarInt` count (`<= 16`) of (`VarInt` kind, long array),
//!   **not** NBT. A long array is a `VarInt` long count followed by that many
//!   big-endian longs.
//! - Java 26.3 light masks use `ByteBufCodecs.BIT_SET`: a VarInt byte length
//!   followed by BitSet.toByteArray bytes, low bit/byte first. Internal u64
//!   words below are only bounded working storage, never the wire layout.
//! - Block-state names are not installed, so a global block palette uses the
//!   wire's own bit width (`<= 32`) and cannot be cross-checked against the
//!   registry's state count. Biome ids are checked against the retained biome
//!   count.
//!
//! Every buffer is caller-owned and borrowed; the module allocates nothing,
//! recurses nowhere, and keeps every loop bounded by the body length and the
//! header caps.

use crate::nbt::{self, Limits, RootView};
use crate::play::PlState;
use crate::wire::Reader;
use crate::Error as WireError;

/// `WC_OK`.
pub const WC_OK: i32 = 0;
/// `WC_BAD_ARGUMENT`.
pub const WC_BAD_ARGUMENT: i32 = -1;
/// `WC_BAD_FRAME`.
pub const WC_BAD_FRAME: i32 = -3;
/// `WC_UNSUPPORTED`.
pub const WC_UNSUPPORTED: i32 = -5;
/// `WC_LIMIT`: an NBT budget was exhausted.
pub const WC_LIMIT: i32 = -6;
/// `WC_MAX_BODY`: largest complete body, packet id included.
pub const WC_MAX_BODY: usize = 8_388_608;
/// `WC_MAX_DIMENSIONS`: the fixed dimension table.
pub const WC_MAX_DIMENSIONS: usize = 32;
/// `WC_MAX_SECTIONS`: 4096-height columns have at most 256 sections.
pub const WC_MAX_SECTIONS: usize = 256;

/// `WC_CONFIGURATION`.
pub const WC_CONFIGURATION: u32 = 0;
/// `WC_PLAY`.
pub const WC_PLAY: u32 = 1;

/// `wc_kind` values, in header order.
pub const WC_METADATA: u32 = 1;
pub const WC_DIMENSIONS: u32 = 2;
pub const WC_BIOMES: u32 = 3;
pub const WC_CHUNK: u32 = 4;
pub const WC_LIGHT: u32 = 5;

/// Biome registries may be much larger than the fixed dimension table.
const MAX_BIOMES: usize = 65_536;
/// Vanilla's `Heightmaps` count limit.
const MAX_HEIGHTMAPS: usize = 16;
/// Block-entity count cap for this diagnostic (vanilla has none).
const MAX_BLOCK_ENTITIES: usize = 4096;
/// One light data layer is exactly 2048 bytes.
const DATA_LAYER_BYTES: usize = 2048;
/// Light sections are block sections plus two; 256 + 2 fits five 64-bit words.
const MAX_LIGHT_WORDS: usize = 5;

/// One `struct wc_dimension`: 16 bytes, alignment 4.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WcDimension {
    pub min_y: i32,
    pub height: i32,
    /// The retained `has_skylight` byte, zero-extended.
    pub has_skylight: u32,
    pub valid: u32,
}

impl WcDimension {
    const ZERO: Self = Self {
        min_y: 0,
        height: 0,
        has_skylight: 0,
        valid: 0,
    };
}

/// `struct wc_state`: 528 bytes, alignment 4. Only the selected registries of
/// a successful configuration registry packet change it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WcState {
    pub dimension_count: u32,
    pub biome_count: u32,
    /// Cumulative dimension entries validated since the state was zeroed.
    pub dimensions_seen: u32,
    /// Cumulative biome entries validated since the state was zeroed.
    pub biomes_seen: u32,
    pub dimensions: [WcDimension; WC_MAX_DIMENSIONS],
}

impl WcState {
    /// A fresh connection's state: everything zero, no dimension valid.
    pub const fn zeroed() -> Self {
        Self {
            dimension_count: 0,
            biome_count: 0,
            dimensions_seen: 0,
            biomes_seen: 0,
            dimensions: [WcDimension::ZERO; WC_MAX_DIMENSIONS],
        }
    }
}

/// `struct wc_event`: 72 bytes, alignment 4. Every field is zero on failure.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WcEvent {
    pub kind: u32,
    pub packet_id: i32,
    pub entries: u32,
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub sections: u32,
    pub heightmaps: u32,
    pub block_entities: u32,
    pub sky_layers: u32,
    pub block_layers: u32,
    pub nonempty_blocks: u32,
    pub fluid_blocks: u32,
    pub sample_valid: u32,
    pub sample_x: i32,
    pub sample_y: i32,
    pub sample_z: i32,
    pub state_id: i32,
    pub biome_id: i32,
}

impl WcEvent {
    /// An all-zero event, the failure shape.
    pub const fn zeroed() -> Self {
        Self {
            kind: 0,
            packet_id: 0,
            entries: 0,
            chunk_x: 0,
            chunk_z: 0,
            sections: 0,
            heightmaps: 0,
            block_entities: 0,
            sky_layers: 0,
            block_layers: 0,
            nonempty_blocks: 0,
            fluid_blocks: 0,
            sample_valid: 0,
            sample_x: 0,
            sample_y: 0,
            sample_z: 0,
            state_id: 0,
            biome_id: 0,
        }
    }
}

/// Why a body was rejected. A separate type keeps the shared login/play
/// `Error` (and its ABI mappings) untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldError {
    /// A pointer, length, phase or scalar argument is out of contract.
    BadArgument,
    /// The body is not a structurally valid packet.
    BadFrame,
    /// Structurally valid, but a registry/palette cap is past what is retained.
    Unsupported,
    /// An NBT budget was exhausted.
    Limit,
}

/// One decoded body: the event to report and the state to commit. `state` is
/// always a complete copy; `state_changed` says whether the caller should
/// write it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldDecoded {
    pub event: WcEvent,
    pub state: WcState,
    pub state_changed: bool,
}

fn wire<T>(result: Result<T, WireError>) -> Result<T, WorldError> {
    result.map_err(|_| WorldError::BadFrame)
}

fn nbt_error(error: nbt::NbtError) -> WorldError {
    match error {
        nbt::NbtError::Limit { .. } => WorldError::Limit,
        nbt::NbtError::Truncated { .. } | nbt::NbtError::Invalid { .. } => WorldError::BadFrame,
    }
}

fn nbt_limits(input: &[u8]) -> Limits {
    Limits {
        max_bytes: input.len(),
        ..Limits::default()
    }
}

/// Consume exactly one network NBT value from `reader`, leaving the bytes that
/// follow it. The returned slice borrows the caller's body.
fn take_nbt<'a>(reader: &mut Reader<'a>) -> Result<&'a [u8], WorldError> {
    let rest = reader.rest();
    let used = nbt::skip_unnamed(rest, nbt_limits(rest)).map_err(nbt_error)?;
    *reader = Reader::new(&rest[used..]);
    Ok(&rest[..used])
}

fn i16_be(reader: &mut Reader<'_>) -> Result<i16, WorldError> {
    let bytes = wire(reader.bytes(2))?;
    Ok(i16::from_be_bytes([bytes[0], bytes[1]]))
}

pub(crate) fn i32_be(reader: &mut Reader<'_>) -> Result<i32, WorldError> {
    let bytes = wire(reader.bytes(4))?;
    Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn u64_be(reader: &mut Reader<'_>) -> Result<u64, WorldError> {
    let bytes = wire(reader.bytes(8))?;
    let mut value = [0u8; 8];
    value.copy_from_slice(bytes);
    Ok(u64::from_be_bytes(value))
}

/// Smallest `bits` with `2^bits >= value`, for `value >= 1`.
fn ceil_log2(value: u32) -> u32 {
    let mut bits = 0u32;
    while (1u64 << bits) < u64::from(value) {
        bits += 1;
    }
    bits
}

/// Vanilla `SimpleBitStorage` length: values never cross a long.
fn storage_len(bits: u32, entries: usize) -> usize {
    if bits == 0 || bits > 32 {
        0
    } else {
        entries.div_ceil(64 / bits as usize)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RegistryKind {
    Dimensions,
    Biomes,
    Other,
}

/// Compare an already-validated identifier against a `minecraft:` name, with
/// the default namespace when the wire omits one.
fn registry_kind(name: &[u8]) -> RegistryKind {
    let (namespace, path): (&[u8], &[u8]) = match name.iter().position(|&byte| byte == b':') {
        Some(colon) if colon > 0 => (&name[..colon], &name[colon + 1..]),
        _ => (b"minecraft", name),
    };
    if namespace != b"minecraft" {
        return RegistryKind::Other;
    }
    if path == b"dimension_type" {
        RegistryKind::Dimensions
    } else if path == b"worldgen/biome" {
        RegistryKind::Biomes
    } else {
        RegistryKind::Other
    }
}

/// Read one `PackedRegistryEntry` header and walk its optional NBT value.
fn read_registry_entry<'a>(reader: &mut Reader<'a>) -> Result<Option<&'a [u8]>, WorldError> {
    wire(reader.identifier())?;
    let has_data = wire(reader.u8())? != 0;
    if has_data {
        Ok(Some(take_nbt(reader)?))
    } else {
        Ok(None)
    }
}

fn parse_dimension(raw: &[u8]) -> Result<WcDimension, WorldError> {
    let view = RootView::parse(raw, nbt_limits(raw)).map_err(nbt_error)?;
    if !view.is_compound() {
        return Err(WorldError::BadFrame);
    }
    let min_y = view.find_i32(b"min_y").ok_or(WorldError::BadFrame)?;
    let height = view.find_i32(b"height").ok_or(WorldError::BadFrame)?;
    let has_skylight = view
        .find_byte(b"has_skylight")
        .ok_or(WorldError::BadFrame)?;
    if min_y % 16 != 0 {
        return Err(WorldError::BadFrame);
    }
    if height <= 0 || height % 16 != 0 || height > 4096 {
        return Err(WorldError::BadFrame);
    }
    let _ = min_y.checked_add(height).ok_or(WorldError::BadFrame)?;
    Ok(WcDimension {
        min_y,
        height,
        has_skylight: u32::from(has_skylight as u8),
        valid: 1,
    })
}

/// `RegistryData` (configuration id 7).
fn decode_registry(reader: &mut Reader<'_>, decoded: &mut WorldDecoded) -> Result<(), WorldError> {
    let name = wire(reader.identifier())?;
    let count_raw = wire(reader.varint())?;
    if count_raw < 0 {
        return Err(WorldError::BadFrame);
    }
    let count = count_raw as usize;
    // Every entry carries at least an identifier length byte and the data flag.
    if count > reader.remaining() / 2 {
        return Err(WorldError::BadFrame);
    }
    decoded.event.kind = WC_METADATA;
    decoded.event.entries = count as u32;

    match registry_kind(name) {
        RegistryKind::Dimensions => {
            if count > WC_MAX_DIMENSIONS {
                return Err(WorldError::Unsupported);
            }
            let mut state = decoded.state;
            for slot in state.dimensions.iter_mut().take(count) {
                let raw = read_registry_entry(reader)?.ok_or(WorldError::Unsupported)?;
                *slot = parse_dimension(raw)?;
            }
            state.dimension_count = count as u32;
            state.dimensions_seen = state.dimensions_seen.saturating_add(count as u32);
            decoded.state = state;
            decoded.state_changed = true;
            decoded.event.kind = WC_DIMENSIONS;
        }
        RegistryKind::Biomes => {
            if count > MAX_BIOMES {
                return Err(WorldError::Unsupported);
            }
            let mut state = decoded.state;
            for _ in 0..count {
                // The value's bytes are walked (validated) by `read_registry_entry`.
                read_registry_entry(reader)?.ok_or(WorldError::Unsupported)?;
            }
            state.biome_count = count as u32;
            state.biomes_seen = state.biomes_seen.saturating_add(count as u32);
            decoded.state = state;
            decoded.state_changed = true;
            decoded.event.kind = WC_BIOMES;
        }
        RegistryKind::Other => {
            for _ in 0..count {
                // Opaque registries are still walked, never retained.
                read_registry_entry(reader)?;
            }
        }
    }
    if reader.remaining() != 0 {
        return Err(WorldError::BadFrame);
    }
    Ok(())
}

fn selected_dimension(state: &WcState, dimension_type: i32) -> Result<WcDimension, WorldError> {
    if dimension_type < 0 {
        return Err(WorldError::Unsupported);
    }
    let index = dimension_type as usize;
    let count = (state.dimension_count as usize).min(WC_MAX_DIMENSIONS);
    if index >= count {
        return Err(WorldError::Unsupported);
    }
    let dimension = state.dimensions[index];
    if dimension.valid == 0
        || dimension.height <= 0
        || dimension.height % 16 != 0
        || dimension.height > 4096
        || dimension.min_y.checked_add(dimension.height).is_none()
    {
        return Err(WorldError::Unsupported);
    }
    Ok(dimension)
}

/// One heightmap: `count` longs of `bits`-wide values, all `<= height`.
fn read_heightmap(
    reader: &mut Reader<'_>,
    bits: u32,
    height: u32,
    longs: usize,
) -> Result<(), WorldError> {
    let per = 64 / bits as usize;
    let mask = (1u64 << bits) - 1;
    for word_index in 0..longs {
        let word = u64_be(reader)?;
        let base = word_index * per;
        for cell in 0..per {
            let index = base + cell;
            if index >= 256 {
                // Bits past the 256 packed values are storage padding.
                break;
            }
            let value = (word >> (cell * bits as usize)) & mask;
            if value > u64::from(height) {
                return Err(WorldError::BadFrame);
            }
        }
    }
    Ok(())
}

/// Where one paletted container's cells belong: the section they describe,
/// bottom-up, and whether they are block states or biomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CellTarget {
    pub section: usize,
    pub blocks: bool,
}

/// A destination for the normalized cells of a chunk's containers, in storage
/// order. A single-value container reports one `uniform` call instead of 4096
/// or 64 per-cell calls; both shapes describe exactly the same cells.
pub(crate) trait CellSink {
    /// Every cell of `target` carries `value`.
    fn uniform(&mut self, target: CellTarget, value: i32);
    /// Cell `index` of `target` (YZX block order or YZX 4×4×4 biome order).
    fn cell(&mut self, target: CellTarget, index: usize, value: i32);
}

/// A sink that keeps nothing: the validating first pass of a column copy.
pub(crate) struct IgnoreCells;

impl CellSink for IgnoreCells {
    fn uniform(&mut self, _target: CellTarget, _value: i32) {}
    fn cell(&mut self, _target: CellTarget, _index: usize, _value: i32) {}
}

/// Read one paletted container and report its normalized ids to `sink`.
///
/// The whole storage is checked: every cell's palette index must be in the
/// local palette, every global block id at most `i32::MAX`, and every biome id
/// below the retained biome count.
fn read_container<S: CellSink>(
    reader: &mut Reader<'_>,
    blocks: bool,
    biome_count: u32,
    entries: usize,
    target: CellTarget,
    sink: &mut S,
) -> Result<(), WorldError> {
    let wire_bits = wire(reader.u8())?;
    let (has_palette, mut bits) = if blocks {
        match wire_bits {
            0 => (true, 0),
            1..=4 => (true, 4),
            5..=8 => (true, u32::from(wire_bits)),
            _ => (false, u32::from(wire_bits)),
        }
    } else {
        match wire_bits {
            0 => (true, 0),
            1..=3 => (true, u32::from(wire_bits)),
            _ => (false, u32::from(wire_bits)),
        }
    };
    if !has_palette && !blocks {
        bits = bits.max(ceil_log2(biome_count.max(1)));
    }
    // SimpleBitStorage holds 1..=32 bits per value.
    if bits > 32 {
        return Err(WorldError::BadFrame);
    }

    let mut palette = [0i32; 256];
    let mut palette_len = 0usize;
    if has_palette {
        if bits == 0 {
            let id = wire(reader.varint())?;
            if id < 0 || (!blocks && id as u32 >= biome_count) {
                return Err(WorldError::BadFrame);
            }
            palette[0] = id;
            palette_len = 1;
        } else {
            let limit = 1usize << bits;
            let count_raw = wire(reader.varint())?;
            if count_raw < 1 || count_raw as usize > limit {
                return Err(WorldError::BadFrame);
            }
            let count = count_raw as usize;
            for slot in palette.iter_mut().take(count) {
                let id = wire(reader.varint())?;
                if id < 0 || (!blocks && id as u32 >= biome_count) {
                    return Err(WorldError::BadFrame);
                }
                *slot = id;
            }
            palette_len = count;
        }
    }

    if bits == 0 {
        // The one-value strategy stores no per-cell storage.
        sink.uniform(target, palette[0]);
        return Ok(());
    }
    let per = 64 / bits as usize;
    let mask = (1u64 << bits) - 1;
    let words = storage_len(bits, entries);
    for word_index in 0..words {
        let word = u64_be(reader)?;
        let base = word_index * per;
        for cell in 0..per {
            let index = base + cell;
            if index >= entries {
                break;
            }
            let value = (word >> (cell * bits as usize)) & mask;
            let resolved = if has_palette {
                if value as usize >= palette_len {
                    return Err(WorldError::BadFrame);
                }
                palette[value as usize]
            } else {
                if blocks {
                    if value > i32::MAX as u64 {
                        return Err(WorldError::BadFrame);
                    }
                } else if value >= u64::from(biome_count) {
                    return Err(WorldError::BadFrame);
                }
                value as i32
            };
            sink.cell(target, index, resolved);
        }
    }
    Ok(())
}

/// A block below the acknowledged pose, when it lies in this chunk column and
/// dimension.
#[derive(Clone, Copy)]
struct Sample {
    section: usize,
    block_index: usize,
    biome_index: usize,
    x: i32,
    y: i32,
    z: i32,
}

/// `floor` of a finite `f64` that must land inside `i32`, without `std`/`libm`.
fn floor_to_i32(value: f64) -> Option<i32> {
    if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
        return None;
    }
    let truncated = value as i64;
    let floor = if value < truncated as f64 {
        truncated - 1
    } else {
        truncated
    };
    if floor < i64::from(i32::MIN) || floor > i64::from(i32::MAX) {
        None
    } else {
        Some(floor as i32)
    }
}

fn sample_target(
    play: &PlState,
    dimension: WcDimension,
    chunk_x: i32,
    chunk_z: i32,
) -> Option<Sample> {
    if play.position_seen == 0 {
        return None;
    }
    let x = floor_to_i32(f64::from_bits(play.position_bits[0]))?;
    let y = floor_to_i32(f64::from_bits(play.position_bits[1]))?;
    let z = floor_to_i32(f64::from_bits(play.position_bits[2]))?;
    if (x >> 4) != chunk_x || (z >> 4) != chunk_z {
        return None;
    }
    let below = y.checked_sub(1)?;
    let end = dimension.min_y.checked_add(dimension.height)?;
    if below < dimension.min_y || below >= end {
        return None;
    }
    let local_y = (below - dimension.min_y) as usize;
    let section = local_y >> 4;
    if section >= (dimension.height as usize) / 16 {
        return None;
    }
    let local_x = (x & 15) as usize;
    let local_z = (z & 15) as usize;
    let section_y = local_y & 15;
    Some(Sample {
        section,
        block_index: ((section_y * 16) + local_z) * 16 + local_x,
        biome_index: (((section_y >> 2) * 4) + (local_z >> 2)) * 4 + (local_x >> 2),
        x,
        y: below,
        z,
    })
}

/// One `LightData`: four masks and the data layers they promise.
#[derive(Clone, Copy)]
struct LightInfo {
    sky_layers: u32,
    block_layers: u32,
}

fn read_bitset(reader: &mut Reader<'_>, bits: usize) -> Result<[u64; MAX_LIGHT_WORDS], WorldError> {
    let count_raw = wire(reader.varint())?;
    if count_raw < 0 {
        return Err(WorldError::BadFrame);
    }
    let count = count_raw as usize;
    if count > bits.div_ceil(8) {
        return Err(WorldError::BadFrame);
    }
    let bytes = wire(reader.bytes(count))?;
    let mut words = [0u64; MAX_LIGHT_WORDS];
    for (index, byte) in bytes.iter().enumerate() {
        words[index / 8] |= u64::from(*byte) << (8 * (index % 8));
    }
    for index in bits..(count * 8) {
        if words[index / 64] >> (index % 64) & 1 != 0 {
            return Err(WorldError::BadFrame);
        }
    }
    Ok(words)
}

fn popcount(words: &[u64; MAX_LIGHT_WORDS]) -> u32 {
    words.iter().map(|word| word.count_ones()).sum()
}

fn read_layers(reader: &mut Reader<'_>, expected: u32) -> Result<u32, WorldError> {
    let count_raw = wire(reader.varint())?;
    if count_raw < 0 || count_raw as u32 != expected {
        return Err(WorldError::BadFrame);
    }
    for _ in 0..count_raw {
        let length = wire(reader.varint())?;
        if length != DATA_LAYER_BYTES as i32 {
            return Err(WorldError::BadFrame);
        }
        wire(reader.bytes(DATA_LAYER_BYTES))?;
    }
    Ok(count_raw as u32)
}

fn read_light(reader: &mut Reader<'_>, sections: usize) -> Result<LightInfo, WorldError> {
    let light_sections = sections + 2;
    let max_words = light_sections.div_ceil(64);
    let sky_mask = read_bitset(reader, light_sections)?;
    let block_mask = read_bitset(reader, light_sections)?;
    let empty_sky_mask = read_bitset(reader, light_sections)?;
    let empty_block_mask = read_bitset(reader, light_sections)?;
    for index in 0..max_words {
        // A section cannot be both present and empty for the same kind.
        if sky_mask[index] & empty_sky_mask[index] != 0
            || block_mask[index] & empty_block_mask[index] != 0
        {
            return Err(WorldError::BadFrame);
        }
    }
    let sky_layers = read_layers(reader, popcount(&sky_mask))?;
    let block_layers = read_layers(reader, popcount(&block_mask))?;
    Ok(LightInfo {
        sky_layers,
        block_layers,
    })
}

/// The structural facts one chunk packet reports to both decoders: the world
/// event and the column copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChunkSummary {
    pub sections: usize,
    pub min_y: i32,
    pub heightmaps: u32,
    pub block_entities: u32,
    pub sky_layers: u32,
    pub block_layers: u32,
    pub nonempty_blocks: u32,
    pub fluid_blocks: u32,
}

/// Decode one chunk packet's payload after its x/z, reporting every normalized
/// container cell to `sink` in bottom-up section order. The whole payload —
/// heightmaps, containers, block entities (NBT included) and light — is
/// validated and must end exactly.
///
/// This is the shared body of the world decoder and both column-copy passes, so
/// a caller that validates twice and writes once sees byte-identical parses.
pub(crate) fn decode_chunk_cells<S: CellSink>(
    reader: &mut Reader<'_>,
    state: &WcState,
    play: &PlState,
    sink: &mut S,
) -> Result<ChunkSummary, WorldError> {
    let dimension = selected_dimension(state, play.dimension_type)?;
    if state.biome_count == 0 {
        return Err(WorldError::Unsupported);
    }
    let sections = (dimension.height as usize) / 16;
    if sections == 0 || sections > WC_MAX_SECTIONS {
        return Err(WorldError::Unsupported);
    }

    let heightmaps_raw = wire(reader.varint())?;
    if heightmaps_raw < 0 || heightmaps_raw as usize > MAX_HEIGHTMAPS {
        return Err(WorldError::BadFrame);
    }
    let heightmaps = heightmaps_raw as usize;
    let height_bits = ceil_log2(dimension.height as u32 + 1);
    let expected_longs = storage_len(height_bits, 256);
    for _ in 0..heightmaps {
        let kind = wire(reader.varint())?;
        if kind < 0 {
            return Err(WorldError::BadFrame);
        }
        let longs_raw = wire(reader.varint())?;
        if longs_raw < 0 || longs_raw as usize != expected_longs {
            return Err(WorldError::BadFrame);
        }
        read_heightmap(reader, height_bits, dimension.height as u32, expected_longs)?;
    }

    // The chunk buffer is a length-prefixed byte array; sections are decoded
    // from it and it must end exactly where its own length says.
    let buffer = wire(reader.byte_array())?;
    let mut sections_reader = Reader::new(buffer);
    let mut nonempty_blocks = 0u32;
    let mut fluid_blocks = 0u32;
    for section in 0..sections {
        let non_empty = i16_be(&mut sections_reader)?;
        let fluids = i16_be(&mut sections_reader)?;
        if !(0..=4096).contains(&i32::from(non_empty)) || !(0..=4096).contains(&i32::from(fluids)) {
            return Err(WorldError::BadFrame);
        }
        nonempty_blocks = nonempty_blocks.saturating_add(u32::from(non_empty as u16));
        fluid_blocks = fluid_blocks.saturating_add(u32::from(fluids as u16));

        read_container(
            &mut sections_reader,
            true,
            state.biome_count,
            4096,
            CellTarget {
                section,
                blocks: true,
            },
            sink,
        )?;
        read_container(
            &mut sections_reader,
            false,
            state.biome_count,
            64,
            CellTarget {
                section,
                blocks: false,
            },
            sink,
        )?;
    }
    if sections_reader.remaining() != 0 {
        return Err(WorldError::BadFrame);
    }

    let block_entities_raw = wire(reader.varint())?;
    if block_entities_raw < 0 {
        return Err(WorldError::BadFrame);
    }
    let block_entities = block_entities_raw as usize;
    if block_entities > MAX_BLOCK_ENTITIES {
        return Err(WorldError::Unsupported);
    }
    let dimension_end = dimension
        .min_y
        .checked_add(dimension.height)
        .ok_or(WorldError::BadFrame)?;
    for _ in 0..block_entities {
        wire(reader.u8())?; // packed xz: consumed, never mapped
        let y = i32::from(i16_be(reader)?);
        let kind = wire(reader.varint())?;
        if kind < 0 {
            return Err(WorldError::BadFrame);
        }
        let _ = take_nbt(reader)?;
        if y < dimension.min_y || y >= dimension_end {
            return Err(WorldError::BadFrame);
        }
    }

    let light = read_light(reader, sections)?;
    if reader.remaining() != 0 {
        return Err(WorldError::BadFrame);
    }
    Ok(ChunkSummary {
        sections,
        min_y: dimension.min_y,
        heightmaps: heightmaps as u32,
        block_entities: block_entities as u32,
        sky_layers: light.sky_layers,
        block_layers: light.block_layers,
        nonempty_blocks,
        fluid_blocks,
    })
}

/// The world decoder's cell destination: only the block under the
/// acknowledged pose is retained.
struct SampleCells {
    sample: Option<Sample>,
    state_id: Option<i32>,
    biome_id: Option<i32>,
}

impl CellSink for SampleCells {
    fn uniform(&mut self, target: CellTarget, value: i32) {
        if self
            .sample
            .is_some_and(|sample| sample.section == target.section)
        {
            if target.blocks {
                self.state_id = Some(value);
            } else {
                self.biome_id = Some(value);
            }
        }
    }

    fn cell(&mut self, target: CellTarget, index: usize, value: i32) {
        let Some(sample) = self
            .sample
            .filter(|sample| sample.section == target.section)
        else {
            return;
        };
        if target.blocks {
            if sample.block_index == index {
                self.state_id = Some(value);
            }
        } else if sample.biome_index == index {
            self.biome_id = Some(value);
        }
    }
}

/// `LevelChunkWithLight` (play id 46).
fn decode_chunk(
    reader: &mut Reader<'_>,
    state: &WcState,
    play: &PlState,
    decoded: &mut WorldDecoded,
) -> Result<(), WorldError> {
    let chunk_x = i32_be(reader)?;
    let chunk_z = i32_be(reader)?;
    let dimension = selected_dimension(state, play.dimension_type)?;
    let sample = sample_target(play, dimension, chunk_x, chunk_z);
    let mut cells = SampleCells {
        sample,
        state_id: None,
        biome_id: None,
    };
    let summary = decode_chunk_cells(reader, state, play, &mut cells)?;

    decoded.event.kind = WC_CHUNK;
    decoded.event.chunk_x = chunk_x;
    decoded.event.chunk_z = chunk_z;
    decoded.event.sections = summary.sections as u32;
    decoded.event.heightmaps = summary.heightmaps;
    decoded.event.block_entities = summary.block_entities;
    decoded.event.sky_layers = summary.sky_layers;
    decoded.event.block_layers = summary.block_layers;
    decoded.event.nonempty_blocks = summary.nonempty_blocks;
    decoded.event.fluid_blocks = summary.fluid_blocks;
    if let (Some(sample), Some(state_id), Some(biome_id)) = (sample, cells.state_id, cells.biome_id)
    {
        decoded.event.sample_valid = 1;
        decoded.event.sample_x = sample.x;
        decoded.event.sample_y = sample.y;
        decoded.event.sample_z = sample.z;
        decoded.event.state_id = state_id;
        decoded.event.biome_id = biome_id;
    }
    Ok(())
}

/// `LightUpdate` (play id 49).
fn decode_light(
    reader: &mut Reader<'_>,
    state: &WcState,
    play: &PlState,
    decoded: &mut WorldDecoded,
) -> Result<(), WorldError> {
    let chunk_x = wire(reader.varint())?;
    let chunk_z = wire(reader.varint())?;
    let dimension = selected_dimension(state, play.dimension_type)?;
    let sections = (dimension.height as usize) / 16;
    if sections == 0 || sections > WC_MAX_SECTIONS {
        return Err(WorldError::Unsupported);
    }
    let light = read_light(reader, sections)?;
    if reader.remaining() != 0 {
        return Err(WorldError::BadFrame);
    }
    decoded.event.kind = WC_LIGHT;
    decoded.event.chunk_x = chunk_x;
    decoded.event.chunk_z = chunk_z;
    decoded.event.sections = sections as u32;
    decoded.event.sky_layers = light.sky_layers;
    decoded.event.block_layers = light.block_layers;
    Ok(())
}

/// Decode **one** complete, already-unframed and already-inflated body.
///
/// `phase` is [`WC_CONFIGURATION`] or [`WC_PLAY`]; `state` is the caller's
/// retained registry state and `play` the play decoder's state (its dimension
/// type and acknowledged pose are all this module reads).
///
/// The returned state is a full copy built aside from the caller's, so a
/// failure changes nothing: the caller only writes it back when
/// [`WorldDecoded::state_changed`] is set. Every other id is opaque metadata:
/// its packet id and, for registry data, its entry count are reported, and its
/// payload is never decoded.
pub fn decode_world_body(
    body: &[u8],
    phase: u32,
    state: &WcState,
    play: &PlState,
) -> Result<WorldDecoded, WorldError> {
    if body.is_empty() || body.len() > WC_MAX_BODY || phase > WC_PLAY {
        return Err(WorldError::BadArgument);
    }
    let mut reader = Reader::new(body);
    let packet_id = wire(reader.varint())?;
    if packet_id < 0 {
        return Err(WorldError::BadFrame);
    }

    let mut decoded = WorldDecoded {
        event: WcEvent::zeroed(),
        state: *state,
        state_changed: false,
    };
    decoded.event.kind = WC_METADATA;
    decoded.event.packet_id = packet_id;

    if phase == WC_CONFIGURATION {
        if packet_id == 7 {
            decode_registry(&mut reader, &mut decoded)?;
        }
    } else {
        match packet_id {
            46 => decode_chunk(&mut reader, state, play, &mut decoded)?,
            49 => decode_light(&mut reader, state, play, &mut decoded)?,
            _ => {}
        }
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play::PlState;
    use crate::wire::write_varint;
    use crate::world_ffi::craftrustnx_world_decode_body;
    use crate::PLAY_DIMENSION_CAP;
    use std::vec;
    use std::vec::Vec;

    fn varint(value: i32) -> Vec<u8> {
        let mut out = [0u8; 5];
        let len = write_varint(value, &mut out).unwrap();
        out[..len].to_vec()
    }

    fn push_varint(out: &mut Vec<u8>, value: i32) {
        out.extend_from_slice(&varint(value));
    }

    fn push_string(out: &mut Vec<u8>, bytes: &[u8]) {
        push_varint(out, bytes.len() as i32);
        out.extend_from_slice(bytes);
    }

    fn named(out: &mut Vec<u8>, kind: u8, name: &[u8], payload: &[u8]) {
        out.push(kind);
        out.extend_from_slice(&(name.len() as u16).to_be_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(payload);
    }

    fn dimension_nbt_opt(min_y: i32, height: i32, skylight: Option<i8>) -> Vec<u8> {
        let mut out = vec![0x0a];
        named(&mut out, 0x03, b"min_y", &min_y.to_be_bytes());
        named(&mut out, 0x03, b"height", &height.to_be_bytes());
        if let Some(value) = skylight {
            named(&mut out, 0x01, b"has_skylight", &[value as u8]);
        }
        out.push(0x00);
        out
    }

    fn dimension_nbt(min_y: i32, height: i32) -> Vec<u8> {
        dimension_nbt_opt(min_y, height, Some(1))
    }

    fn biome_nbt() -> Vec<u8> {
        vec![0x0a, 0x00]
    }

    fn registry_body(registry: &str, entries: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
        let mut out = varint(7);
        push_string(&mut out, registry.as_bytes());
        push_varint(&mut out, entries.len() as i32);
        for (name, data) in entries {
            push_string(&mut out, name.as_bytes());
            match data {
                Some(nbt) => {
                    out.push(1);
                    out.extend_from_slice(nbt);
                }
                None => out.push(0),
            }
        }
        out
    }

    fn play_state(dimension_type: i32, x: f64, y: f64, z: f64) -> PlState {
        PlState {
            position_bits: [x.to_bits(), y.to_bits(), z.to_bits()],
            yaw_bits: 0,
            pitch_bits: 0,
            player_id: 0,
            dimension_type,
            game_mode: 0,
            teleport_id: 0,
            login_seen: 1,
            position_seen: 1,
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
        state.dimensions_seen = 1;
        state.biomes_seen = biomes;
        state
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

    fn section(non_empty: i16, fluids: i16, states: &[u8], biomes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&non_empty.to_be_bytes());
        out.extend_from_slice(&fluids.to_be_bytes());
        out.extend_from_slice(states);
        out.extend_from_slice(biomes);
        out
    }

    fn simple_section(state_id: i32, biome_id: i32) -> Vec<u8> {
        section(
            0,
            0,
            &single_container(state_id),
            &single_container(biome_id),
        )
    }

    fn chunk_body(
        x: i32,
        z: i32,
        heightmaps: &[u8],
        buffer: &[u8],
        block_entities: &[u8],
        light: &[u8],
    ) -> Vec<u8> {
        let mut out = varint(46);
        out.extend_from_slice(&x.to_be_bytes());
        out.extend_from_slice(&z.to_be_bytes());
        out.extend_from_slice(heightmaps);
        push_string(&mut out, buffer);
        out.extend_from_slice(block_entities);
        out.extend_from_slice(light);
        out
    }

    fn no_heightmaps() -> Vec<u8> {
        varint(0)
    }

    fn no_block_entities() -> Vec<u8> {
        varint(0)
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
            push_string(&mut out, &[fill; DATA_LAYER_BYTES]);
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

    fn valid_chunk(height: i32) -> Vec<u8> {
        assert_eq!(height, 16, "the one-section helper");
        let buffer = simple_section(5, 0);
        chunk_body(
            0,
            0,
            &no_heightmaps(),
            &buffer,
            &no_block_entities(),
            &empty_light(),
        )
    }

    #[test]
    fn abi_layout_matches_the_header() {
        assert_eq!(core::mem::size_of::<WcState>(), 528);
        assert_eq!(core::mem::align_of::<WcState>(), 4);
        assert_eq!(core::mem::size_of::<WcEvent>(), 72);
        assert_eq!(core::mem::align_of::<WcEvent>(), 4);
        assert_eq!(core::mem::size_of::<WcDimension>(), 16);
        assert_eq!(WC_MAX_BODY, 8_388_608);
    }

    #[test]
    fn registry_retains_only_selected_kinds_and_walks_the_rest() {
        let dimension = dimension_nbt(-64, 384);
        let body = registry_body(
            "minecraft:dimension_type",
            &[("minecraft:overworld", Some(&dimension))],
        );
        let state = WcState::zeroed();
        let play = play_state(0, 0.0, 0.0, 0.0);
        let decoded = decode_world_body(&body, WC_CONFIGURATION, &state, &play).unwrap();
        assert_eq!(decoded.event.kind, WC_DIMENSIONS);
        assert_eq!(decoded.event.packet_id, 7);
        assert_eq!(decoded.event.entries, 1);
        assert!(decoded.state_changed);
        assert_eq!(decoded.state.dimension_count, 1);
        assert_eq!(decoded.state.dimensions_seen, 1);
        assert_eq!(
            decoded.state.dimensions[0],
            WcDimension {
                min_y: -64,
                height: 384,
                has_skylight: 1,
                valid: 1
            }
        );

        let biome = biome_nbt();
        let biomes = registry_body(
            "minecraft:worldgen/biome",
            &[
                ("minecraft:plains", Some(&biome)),
                ("minecraft:desert", Some(&biome)),
            ],
        );
        let decoded = decode_world_body(&biomes, WC_CONFIGURATION, &decoded.state, &play).unwrap();
        assert_eq!(decoded.event.kind, WC_BIOMES);
        assert_eq!(decoded.event.entries, 2);
        assert_eq!(decoded.state.biome_count, 2);
        assert_eq!(decoded.state.biomes_seen, 2);
        // The dimension table is preserved across the second registry.
        assert_eq!(decoded.state.dimension_count, 1);
        assert_eq!(decoded.state.dimensions[0].height, 384);

        // Another registry is metadata; data may be absent, present NBT is walked.
        let blocks = registry_body(
            "minecraft:block",
            &[("minecraft:stone", None), ("minecraft:dirt", Some(&biome))],
        );
        let before = decoded.state;
        let decoded = decode_world_body(&blocks, WC_CONFIGURATION, &decoded.state, &play).unwrap();
        assert_eq!(decoded.event.kind, WC_METADATA);
        assert_eq!(decoded.event.entries, 2);
        assert!(!decoded.state_changed);
        assert_eq!(decoded.state, before);

        // Even opaque registry NBT is validated.
        let broken = vec![0x0c, 0x7f, 0xff, 0xff, 0xff];
        let body = registry_body("minecraft:block", &[("minecraft:stone", Some(&broken))]);
        assert_eq!(
            decode_world_body(&body, WC_CONFIGURATION, &decoded.state, &play),
            Err(WorldError::BadFrame)
        );
    }

    #[test]
    fn dimension_entries_are_validated_and_required() {
        let play = play_state(0, 0.0, 0.0, 0.0);
        let state = WcState::zeroed();
        let cases: &[(Vec<u8>, WorldError)] = &[
            (dimension_nbt_opt(-64, 384, None), WorldError::BadFrame),
            (dimension_nbt(1, 384), WorldError::BadFrame), // min_y not /16
            (dimension_nbt(0, 0), WorldError::BadFrame),
            (dimension_nbt(0, 15), WorldError::BadFrame),
            (dimension_nbt(0, 4112), WorldError::BadFrame),
            (dimension_nbt(i32::MAX & !15, 16), WorldError::BadFrame),
        ];
        for (nbt, expected) in cases {
            let body = registry_body(
                "minecraft:dimension_type",
                &[("minecraft:overworld", Some(nbt))],
            );
            assert_eq!(
                decode_world_body(&body, WC_CONFIGURATION, &state, &play),
                Err(*expected),
                "{nbt:02x?}"
            );
        }

        // A selected entry without its data cannot be validated.
        let body = registry_body("minecraft:dimension_type", &[("minecraft:overworld", None)]);
        assert_eq!(
            decode_world_body(&body, WC_CONFIGURATION, &state, &play),
            Err(WorldError::Unsupported)
        );

        // A valid selected registry too large for the table is unsupported.
        let nbt = dimension_nbt(-64, 384);
        let mut entries = Vec::new();
        for _ in 0..=WC_MAX_DIMENSIONS {
            entries.push(("minecraft:overworld", Some(nbt.as_slice())));
        }
        let body = registry_body("minecraft:dimension_type", &entries);
        assert_eq!(
            decode_world_body(&body, WC_CONFIGURATION, &state, &play),
            Err(WorldError::Unsupported)
        );
    }

    #[test]
    fn registry_side_nbt_limit_is_a_limit() {
        let play = play_state(0, 0.0, 0.0, 0.0);
        // A root list of 65,536 ints is one tag over the 65,536-tag budget.
        let mut huge = vec![0x09, 0x03, 0x00, 0x01, 0x00, 0x00];
        huge.resize(huge.len() + 65_536 * 4, 0);
        let body = registry_body("minecraft:block", &[("minecraft:stone", Some(&huge))]);
        assert_eq!(
            decode_world_body(&body, WC_CONFIGURATION, &WcState::zeroed(), &play),
            Err(WorldError::Limit)
        );
    }

    #[test]
    fn one_section_chunk_reports_the_header_counts() {
        let state = selected_state(-64, 16, 1);
        let play = play_state(0, 0.0, 0.0, 0.0);
        let body = valid_chunk(16);
        let decoded = decode_world_body(&body, WC_PLAY, &state, &play).unwrap();
        assert_eq!(decoded.event.kind, WC_CHUNK);
        assert_eq!(decoded.event.packet_id, 46);
        assert_eq!((decoded.event.chunk_x, decoded.event.chunk_z), (0, 0));
        assert_eq!(decoded.event.sections, 1);
        assert_eq!(decoded.event.heightmaps, 0);
        assert_eq!(decoded.event.block_entities, 0);
        assert_eq!(decoded.event.sky_layers, 0);
        assert_eq!(decoded.event.block_layers, 0);
        assert_eq!(decoded.event.sample_valid, 0);
        assert!(!decoded.state_changed);
    }

    #[test]
    fn section_counts_must_be_two_bounded_shorts() {
        let state = selected_state(-64, 16, 1);
        let play = play_state(0, 0.0, 0.0, 0.0);
        for value in [4097i16, -1, i16::MIN] {
            let buffer = section(value, 0, &single_container(0), &single_container(0));
            let body = chunk_body(
                0,
                0,
                &no_heightmaps(),
                &buffer,
                &no_block_entities(),
                &empty_light(),
            );
            assert_eq!(
                decode_world_body(&body, WC_PLAY, &state, &play),
                Err(WorldError::BadFrame),
                "non_empty {value}"
            );
            let buffer = section(0, value, &single_container(0), &single_container(0));
            let body = chunk_body(
                0,
                0,
                &no_heightmaps(),
                &buffer,
                &no_block_entities(),
                &empty_light(),
            );
            assert_eq!(
                decode_world_body(&body, WC_PLAY, &state, &play),
                Err(WorldError::BadFrame),
                "fluids {value}"
            );
        }
        // 4096 is the accepted maximum and is summed.
        let buffer = section(4096, 4096, &single_container(0), &single_container(0));
        let body = chunk_body(
            0,
            0,
            &no_heightmaps(),
            &buffer,
            &no_block_entities(),
            &empty_light(),
        );
        let decoded = decode_world_body(&body, WC_PLAY, &state, &play).unwrap();
        assert_eq!(decoded.event.nonempty_blocks, 4096);
        assert_eq!(decoded.event.fluid_blocks, 4096);
    }

    #[test]
    fn every_block_palette_mode_reads_without_a_long_prefix() {
        let state = selected_state(-64, 16, 4);
        let play = play_state(0, 0.0, 0.0, 0.0);
        let biome = single_container(0);
        for wire in [0u8, 1, 3, 5, 8, 9, 10, 32] {
            let (states, bits) = if wire == 0 {
                (single_container(7), 0)
            } else if wire <= 8 {
                let bits = if wire <= 4 { 4 } else { u32::from(wire) };
                let words = vec![0u64; storage_len(bits, 4096)];
                (local_container(wire, &[7, 9], &words), bits)
            } else {
                let bits = u32::from(wire);
                let words = vec![0u64; storage_len(bits, 4096)];
                (global_container(wire, &words), bits)
            };
            assert_eq!(bits, states_wire_bits(&states));
            let buffer = section(0, 0, &states, &biome);
            let body = chunk_body(
                0,
                0,
                &no_heightmaps(),
                &buffer,
                &no_block_entities(),
                &empty_light(),
            );
            assert!(
                decode_world_body(&body, WC_PLAY, &state, &play).is_ok(),
                "states {states:02x?}"
            );
        }

        // A wire width past 32 bits per value is not a storage this reads.
        let states = global_container(33, &[]);
        let buffer = section(0, 0, &states, &biome);
        let body = chunk_body(
            0,
            0,
            &no_heightmaps(),
            &buffer,
            &no_block_entities(),
            &empty_light(),
        );
        assert_eq!(
            decode_world_body(&body, WC_PLAY, &state, &play),
            Err(WorldError::BadFrame)
        );

        // An extra long-count prefix makes the section buffer end early.
        let mut prefixed = local_container(4, &[7, 9], &[0; 1024]);
        prefixed.insert(3, 0x00);
        let buffer = section(0, 0, &prefixed, &biome);
        let body = chunk_body(
            0,
            0,
            &no_heightmaps(),
            &buffer,
            &no_block_entities(),
            &empty_light(),
        );
        assert_eq!(
            decode_world_body(&body, WC_PLAY, &state, &play),
            Err(WorldError::BadFrame)
        );
    }

    fn states_wire_bits(states: &[u8]) -> u32 {
        match states[0] {
            0 => 0,
            1..=4 => 4,
            other => u32::from(other),
        }
    }

    #[test]
    fn every_biome_palette_mode_checks_the_retained_count() {
        let state = selected_state(-64, 16, 4);
        let play = play_state(0, 0.0, 0.0, 0.0);
        let block = single_container(0);
        let mut modes = Vec::new();
        for wire in [0u8, 1, 2, 3, 4, 5] {
            let biomes = if wire == 0 {
                single_container(3)
            } else if wire <= 3 {
                let bits = u32::from(wire);
                local_container(wire, &[0, 2], &vec![0u64; storage_len(bits, 64)])
            } else {
                let bits = u32::from(wire).max(ceil_log2(4));
                global_container(wire, &vec![0u64; storage_len(bits, 64)])
            };
            modes.push(biomes);
        }
        for biomes in modes {
            let buffer = section(0, 0, &block, &biomes);
            let body = chunk_body(
                0,
                0,
                &no_heightmaps(),
                &buffer,
                &no_block_entities(),
                &empty_light(),
            );
            assert!(
                decode_world_body(&body, WC_PLAY, &state, &play).is_ok(),
                "biomes {biomes:02x?}"
            );
        }

        // A local or single biome id at or past the retained count is refused.
        for biomes in [
            local_container(0, &[4], &[]),
            local_container(1, &[0, 4], &[0; 1]),
            global_container(4, &[4; 16]),
        ] {
            let buffer = section(0, 0, &block, &biomes);
            let body = chunk_body(
                0,
                0,
                &no_heightmaps(),
                &buffer,
                &no_block_entities(),
                &empty_light(),
            );
            assert_eq!(
                decode_world_body(&body, WC_PLAY, &state, &play),
                Err(WorldError::BadFrame),
                "biomes {biomes:02x?}"
            );
        }

        // Without a retained biome registry the chunk cannot be validated.
        let state = selected_state(-64, 16, 0);
        let body = valid_chunk(16);
        assert_eq!(
            decode_world_body(&body, WC_PLAY, &state, &play),
            Err(WorldError::Unsupported)
        );
    }

    #[test]
    fn local_palette_indices_must_stay_inside_the_palette() {
        let state = selected_state(-64, 16, 1);
        let play = play_state(0, 0.0, 0.0, 0.0);
        // 4-bit palette of two entries, first value 2.
        let states = local_container(4, &[7, 9], &[0x2; 1024]);
        let buffer = section(0, 0, &states, &single_container(0));
        let body = chunk_body(
            0,
            0,
            &no_heightmaps(),
            &buffer,
            &no_block_entities(),
            &empty_light(),
        );
        assert_eq!(
            decode_world_body(&body, WC_PLAY, &state, &play),
            Err(WorldError::BadFrame)
        );
    }

    #[test]
    fn heightmaps_have_kind_bit_width_and_exactly_the_expected_longs() {
        let state = selected_state(-64, 16, 1);
        let play = play_state(0, 0.0, 0.0, 0.0);
        let container = simple_section(0, 0);
        let buffer = container;
        let long_words = |last: u64| {
            let mut words = [0u64; 22];
            words[21] = last;
            words
        };
        let heightmap = |kind: i32, words: &[u64]| {
            let mut out = varint(1); // one heightmap
            push_varint(&mut out, kind);
            out.extend_from_slice(&varint(words.len() as i32));
            for word in words {
                out.extend_from_slice(&word.to_be_bytes());
            }
            out
        };
        let body =
            |maps: &[u8]| chunk_body(0, 0, maps, &buffer, &no_block_entities(), &empty_light());

        let decoded =
            decode_world_body(&body(&heightmap(3, &long_words(0))), WC_PLAY, &state, &play)
                .unwrap();
        assert_eq!(decoded.event.heightmaps, 1);

        // A value above the dimension height.
        assert_eq!(
            decode_world_body(
                &body(&heightmap(0, &long_words(17))),
                WC_PLAY,
                &state,
                &play
            ),
            Err(WorldError::BadFrame)
        );
        // Bits past the 256 packed values are padding and are ignored.
        let mut padding = 0u64;
        for cell in 4..12 {
            padding |= 31u64 << (cell * 5);
        }
        assert!(decode_world_body(
            &body(&heightmap(0, &long_words(padding))),
            WC_PLAY,
            &state,
            &play
        )
        .is_ok());
        // Negative kind and a wrong long count.
        for maps in [
            heightmap(-1, &long_words(0)),
            heightmap(0, &long_words(0)[..21]),
        ] {
            assert_eq!(
                decode_world_body(&body(&maps), WC_PLAY, &state, &play),
                Err(WorldError::BadFrame)
            );
        }
        // Seventeen heightmaps are past vanilla's cap.
        let mut too_many = varint(17);
        for _ in 0..17 {
            too_many.extend_from_slice(&heightmap(0, &long_words(0))[1..]);
        }
        assert_eq!(
            decode_world_body(&body(&too_many), WC_PLAY, &state, &play),
            Err(WorldError::BadFrame)
        );
    }

    #[test]
    fn light_masks_layers_and_2048_byte_lengths_are_checked() {
        let state = selected_state(-64, 16, 1);
        let play = play_state(0, 0.0, 0.0, 0.0);
        let buffer = simple_section(0, 0);
        let body =
            |light: &[u8]| chunk_body(0, 0, &no_heightmaps(), &buffer, &no_block_entities(), light);

        // A bit at light section 3 does not exist when there is one block section.
        let bad = light_data([&[8], &[], &[], &[]], &layers(1, 0xFF), &layers(0, 0));
        assert_eq!(
            decode_world_body(&body(&bad), WC_PLAY, &state, &play),
            Err(WorldError::BadFrame)
        );
        // Ignored high bits of a mask word are still rejected.
        let bad = light_data([&[1 << 63], &[], &[], &[]], &layers(0, 0), &layers(0, 0));
        assert_eq!(
            decode_world_body(&body(&bad), WC_PLAY, &state, &play),
            Err(WorldError::BadFrame)
        );
        // Present and empty for the same section and kind.
        let bad = light_data([&[1], &[], &[1], &[]], &layers(1, 0xFF), &layers(0, 0));
        assert_eq!(
            decode_world_body(&body(&bad), WC_PLAY, &state, &play),
            Err(WorldError::BadFrame)
        );
        // The data layer count must equal the mask's population.
        let bad = light_data([&[1], &[], &[], &[]], &layers(0, 0), &layers(0, 0));
        assert_eq!(
            decode_world_body(&body(&bad), WC_PLAY, &state, &play),
            Err(WorldError::BadFrame)
        );
        // A layer must be exactly 2048 bytes.
        let mut short = varint(1);
        short.extend_from_slice(&varint(2047));
        short.resize(short.len() + 2047, 0);
        let bad = light_data([&[1], &[], &[], &[]], &short, &layers(0, 0));
        assert_eq!(
            decode_world_body(&body(&bad), WC_PLAY, &state, &play),
            Err(WorldError::BadFrame)
        );
        // A byte length beyond this dimension's mask bound is refused.
        let bad = vec![2, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            decode_world_body(&body(&bad), WC_PLAY, &state, &play),
            Err(WorldError::BadFrame)
        );

        // The valid shape reports the layer counts.
        let light = light_data([&[1], &[1], &[2], &[]], &layers(1, 0xF0), &layers(1, 0x0F));
        let decoded = decode_world_body(&body(&light), WC_PLAY, &state, &play).unwrap();
        assert_eq!(decoded.event.sky_layers, 1);
        assert_eq!(decoded.event.block_layers, 1);
    }

    #[test]
    fn light_mask_byte_length_and_low_byte_order_match_26_3() {
        // LightUpdate(49), x=0,z=0; sky mask has byte length 2 and bit 8 set.
        // Other masks empty; one 2048-byte sky layer, no block layers.
        // Literal wire vector independent of bitset()/light_data() helpers.
        let mut body = vec![49, 0, 0, 2, 0, 1, 0, 0, 0, 1, 0x80, 0x10];
        body.extend_from_slice(&[0xf0; 2048]);
        body.push(0);
        let state = selected_state(0, 128, 1);
        let play = play_state(0, 0.0, 0.0, 0.0);
        let decoded = decode_world_body(&body, WC_PLAY, &state, &play).unwrap();
        assert_eq!(decoded.event.kind, WC_LIGHT);
        assert_eq!(
            (decoded.event.sky_layers, decoded.event.block_layers),
            (1, 0)
        );
    }

    #[test]
    fn samples_the_block_below_the_acknowledged_pose() {
        let state = selected_state(-64, 384, 4);
        let play = play_state(0, 1.5, -62.5, 2.5);
        let mut buffer = Vec::new();
        // Section 0: 4-bit palette [11, 22], cell 33 selects 22.
        let mut words = vec![0u64; storage_len(4, 4096)];
        words[2] = 1 << 4; // cell 1 of word 2
        buffer.extend_from_slice(&section(
            0,
            0,
            &local_container(4, &[11, 22], &words),
            &local_container(1, &[2, 3], &[0; 1]),
        ));
        for _ in 1..24 {
            buffer.extend_from_slice(&simple_section(0, 0));
        }
        let body = chunk_body(
            0,
            0,
            &no_heightmaps(),
            &buffer,
            &no_block_entities(),
            &empty_light(),
        );
        let decoded = decode_world_body(&body, WC_PLAY, &state, &play).unwrap();
        assert_eq!(decoded.event.sample_valid, 1);
        assert_eq!(
            (
                decoded.event.sample_x,
                decoded.event.sample_y,
                decoded.event.sample_z
            ),
            (1, -64, 2)
        );
        assert_eq!(decoded.event.state_id, 22);
        assert_eq!(decoded.event.biome_id, 2);

        // Negative coordinates use floor and the matching column.
        let play = play_state(0, -1.5, -62.5, -1.5);
        let mut buffer = simple_section(9, 3);
        for _ in 1..24 {
            buffer.extend_from_slice(&simple_section(0, 0));
        }
        let body = chunk_body(
            -1,
            -1,
            &no_heightmaps(),
            &buffer,
            &no_block_entities(),
            &empty_light(),
        );
        let decoded = decode_world_body(&body, WC_PLAY, &state, &play).unwrap();
        assert_eq!(decoded.event.sample_valid, 1);
        assert_eq!(
            (
                decoded.event.sample_x,
                decoded.event.sample_y,
                decoded.event.sample_z
            ),
            (-2, -64, -2)
        );
        assert_eq!(decoded.event.state_id, 9);
        assert_eq!(decoded.event.biome_id, 3);

        // No acknowledged pose, another column, below the dimension or a
        // non-finite axis: the chunk is valid and the sample is absent.
        let short_state = selected_state(-64, 16, 4);
        let mut no_pose = play_state(0, 1.5, -62.5, 2.5);
        no_pose.position_seen = 0;
        let body = valid_chunk(16);
        let decoded = decode_world_body(&body, WC_PLAY, &short_state, &no_pose).unwrap();
        assert_eq!(decoded.event.sample_valid, 0);

        let other_column = play_state(0, 100.0, -62.5, 2.5);
        let decoded = decode_world_body(&body, WC_PLAY, &short_state, &other_column).unwrap();
        assert_eq!(decoded.event.sample_valid, 0);

        let outside = play_state(0, 1.5, 320.0, 2.5);
        let decoded = decode_world_body(&body, WC_PLAY, &short_state, &outside).unwrap();
        assert_eq!(decoded.event.sample_valid, 0);

        let mut nan = play_state(0, f64::NAN, -62.5, 2.5);
        nan.position_bits[0] = f64::NAN.to_bits();
        let decoded = decode_world_body(&body, WC_PLAY, &short_state, &nan).unwrap();
        assert_eq!(decoded.event.sample_valid, 0);
    }

    #[test]
    fn block_entities_are_bounded_and_inside_the_dimension() {
        let state = selected_state(-64, 16, 1);
        let play = play_state(0, 0.0, 0.0, 0.0);
        let buffer = simple_section(0, 0);
        let entity = |y: i16, kind: i32, nbt: &[u8]| {
            let mut out = vec![0x12]; // packed xz
            out.extend_from_slice(&y.to_be_bytes());
            out.extend_from_slice(&varint(kind));
            out.extend_from_slice(nbt);
            out
        };
        let body = |entities: &[u8]| {
            let mut out = varint(46);
            out.extend_from_slice(&0i32.to_be_bytes());
            out.extend_from_slice(&0i32.to_be_bytes());
            out.extend_from_slice(&no_heightmaps());
            push_string(&mut out, &buffer);
            out.extend_from_slice(entities);
            out.extend_from_slice(&empty_light());
            out
        };
        let mut one = varint(1);
        one.extend_from_slice(&entity(-64, 3, &[0x00]));
        let decoded = decode_world_body(&body(&one), WC_PLAY, &state, &play).unwrap();
        assert_eq!(decoded.event.block_entities, 1);

        // The top of the dimension is min_y + height - 1.
        let mut top = varint(1);
        top.extend_from_slice(&entity(-49, 0, &[0x00]));
        assert!(decode_world_body(&body(&top), WC_PLAY, &state, &play).is_ok());
        for entity_bytes in [
            entity(-65, 0, &[0x00]),
            entity(-48, 0, &[0x00]),
            entity(-64, -1, &[0x00]),
            entity(-64, 0, &[0x0b, 0x00, 0x00, 0x00]),
        ] {
            let mut one = varint(1);
            one.extend_from_slice(&entity_bytes);
            assert_eq!(
                decode_world_body(&body(&one), WC_PLAY, &state, &play),
                Err(WorldError::BadFrame)
            );
        }

        let mut too_many = varint(4097);
        too_many.extend_from_slice(&entity(-64, 0, &[0x00]));
        assert_eq!(
            decode_world_body(&body(&too_many), WC_PLAY, &state, &play),
            Err(WorldError::Unsupported)
        );
    }

    #[test]
    fn light_update_packet_reads_varint_coordinates_and_the_same_light() {
        let state = selected_state(-64, 16, 1);
        let play = play_state(0, 0.0, 0.0, 0.0);
        let light = light_data([&[1], &[], &[], &[]], &layers(1, 0x33), &layers(0, 0));
        let mut body = varint(49);
        body.extend_from_slice(&varint(-7));
        body.extend_from_slice(&varint(9));
        body.extend_from_slice(&light);
        let decoded = decode_world_body(&body, WC_PLAY, &state, &play).unwrap();
        assert_eq!(decoded.event.kind, WC_LIGHT);
        assert_eq!(decoded.event.packet_id, 49);
        assert_eq!((decoded.event.chunk_x, decoded.event.chunk_z), (-7, 9));
        assert_eq!(decoded.event.sections, 1);
        assert_eq!(decoded.event.sky_layers, 1);
        assert_eq!(decoded.event.block_layers, 0);

        // The same light validation applies, and the body must end exactly.
        let mut bad = varint(49);
        bad.extend_from_slice(&varint(0));
        bad.extend_from_slice(&varint(0));
        bad.extend_from_slice(&light_data(
            [&[8], &[], &[], &[]],
            &layers(1, 0),
            &layers(0, 0),
        ));
        assert_eq!(
            decode_world_body(&bad, WC_PLAY, &state, &play),
            Err(WorldError::BadFrame)
        );
    }

    #[test]
    fn selecting_a_dimension_and_phase_are_checked() {
        let state = selected_state(-64, 16, 1);
        let play = play_state(1, 0.0, 0.0, 0.0);
        let body = valid_chunk(16);
        assert_eq!(
            decode_world_body(&body, WC_PLAY, &state, &play),
            Err(WorldError::Unsupported)
        );
        let play = play_state(-1, 0.0, 0.0, 0.0);
        assert_eq!(
            decode_world_body(&body, WC_PLAY, &state, &play),
            Err(WorldError::Unsupported)
        );
        let play = play_state(0, 0.0, 0.0, 0.0);
        assert_eq!(
            decode_world_body(&body, 2, &state, &play),
            Err(WorldError::BadArgument)
        );
        assert_eq!(
            decode_world_body(&[], WC_PLAY, &state, &play),
            Err(WorldError::BadArgument)
        );
    }

    #[test]
    fn malformed_and_truncated_bodies_never_panic() {
        let state = selected_state(-64, 16, 1);
        let play = play_state(0, 0.0, 0.0, 0.0);
        let chunk = valid_chunk(16);
        for end in 1..chunk.len() {
            assert_eq!(
                decode_world_body(&chunk[..end], WC_PLAY, &state, &play),
                Err(WorldError::BadFrame),
                "chunk prefix {end}"
            );
        }
        let dimension = dimension_nbt(-64, 384);
        let registry = registry_body(
            "minecraft:dimension_type",
            &[("minecraft:overworld", Some(&dimension))],
        );
        for end in 1..registry.len() {
            assert!(
                decode_world_body(&registry[..end], WC_CONFIGURATION, &state, &play).is_err(),
                "registry prefix {end}"
            );
        }
        // A VarInt wider than 32 bits.
        assert_eq!(
            decode_world_body(&[0xff, 0xff, 0xff, 0xff, 0x1f], WC_PLAY, &state, &play),
            Err(WorldError::BadFrame)
        );
        // Trailing bytes after an otherwise complete chunk.
        let mut trailing = chunk.clone();
        trailing.push(0);
        assert_eq!(
            decode_world_body(&trailing, WC_PLAY, &state, &play),
            Err(WorldError::BadFrame)
        );
    }

    #[test]
    fn ffi_statuses_and_transactional_state() {
        let state = selected_state(-64, 16, 1);
        let play = play_state(0, 0.0, 0.0, 0.0);
        let body = valid_chunk(16);
        let mut out_state = state;
        let mut event = WcEvent::zeroed();
        let status = unsafe {
            craftrustnx_world_decode_body(
                body.as_ptr(),
                body.len() as u32,
                WC_PLAY,
                &mut out_state,
                &play,
                &mut event,
            )
        };
        assert_eq!(status, WC_OK);
        assert_eq!(event.kind, WC_CHUNK);
        assert_eq!(out_state, state);

        // A failure leaves every event byte zero and the state bytes unchanged.
        let mut out_state = state;
        let mut event = WcEvent::zeroed();
        let event_bytes = unsafe {
            core::slice::from_raw_parts_mut(
                (&mut event as *mut WcEvent).cast::<u8>(),
                core::mem::size_of::<WcEvent>(),
            )
        };
        event_bytes.fill(0xAA);
        let broken = &body[..body.len() - 1];
        let status = unsafe {
            craftrustnx_world_decode_body(
                broken.as_ptr(),
                broken.len() as u32,
                WC_PLAY,
                &mut out_state,
                &play,
                &mut event,
            )
        };
        assert_eq!(status, WC_BAD_FRAME);
        assert_eq!(out_state, state);
        assert!(event_bytes.iter().all(|&byte| byte == 0));

        // Registry failure paths.
        let dimension = dimension_nbt(-64, 384);
        let valid = registry_body(
            "minecraft:dimension_type",
            &[("minecraft:overworld", Some(&dimension))],
        );
        let mut out_state = WcState::zeroed();
        let mut event = WcEvent::zeroed();
        assert_eq!(
            unsafe {
                craftrustnx_world_decode_body(
                    valid.as_ptr(),
                    valid.len() as u32,
                    WC_CONFIGURATION,
                    &mut out_state,
                    &play,
                    &mut event,
                )
            },
            WC_OK
        );
        assert_eq!(out_state.dimension_count, 1);

        let missing = registry_body("minecraft:dimension_type", &[("minecraft:overworld", None)]);
        let before = out_state;
        let mut event = WcEvent::zeroed();
        assert_eq!(
            unsafe {
                craftrustnx_world_decode_body(
                    missing.as_ptr(),
                    missing.len() as u32,
                    WC_CONFIGURATION,
                    &mut out_state,
                    &play,
                    &mut event,
                )
            },
            WC_UNSUPPORTED
        );
        assert_eq!(out_state, before);
        assert_eq!(event, WcEvent::zeroed());

        // NBT tag budget -> WC_LIMIT.
        let mut huge = vec![0x09, 0x03, 0x00, 0x01, 0x00, 0x00];
        huge.resize(huge.len() + 65_536 * 4, 0);
        let limit = registry_body("minecraft:block", &[("minecraft:stone", Some(&huge))]);
        assert_eq!(
            unsafe {
                craftrustnx_world_decode_body(
                    limit.as_ptr(),
                    limit.len() as u32,
                    WC_CONFIGURATION,
                    &mut out_state,
                    &play,
                    &mut event,
                )
            },
            WC_LIMIT
        );
    }

    #[test]
    fn ffi_arguments_are_checked_before_any_read() {
        let state = selected_state(-64, 16, 1);
        let play = play_state(0, 0.0, 0.0, 0.0);
        let body = valid_chunk(16);
        let mut out_state = state;
        let mut event = WcEvent::zeroed();
        let ok = body.as_ptr();
        let len = body.len() as u32;

        assert_eq!(
            unsafe {
                craftrustnx_world_decode_body(
                    ok,
                    len,
                    WC_PLAY,
                    &mut out_state,
                    &play,
                    core::ptr::null_mut(),
                )
            },
            WC_BAD_ARGUMENT
        );
        assert_eq!(
            unsafe {
                craftrustnx_world_decode_body(
                    ok,
                    len,
                    WC_PLAY,
                    core::ptr::null_mut(),
                    &play,
                    &mut event,
                )
            },
            WC_BAD_ARGUMENT
        );
        assert_eq!(
            unsafe {
                craftrustnx_world_decode_body(
                    ok,
                    len,
                    WC_PLAY,
                    &mut out_state,
                    core::ptr::null(),
                    &mut event,
                )
            },
            WC_BAD_ARGUMENT
        );
        assert_eq!(
            unsafe {
                craftrustnx_world_decode_body(
                    core::ptr::null(),
                    5,
                    WC_PLAY,
                    &mut out_state,
                    &play,
                    &mut event,
                )
            },
            WC_BAD_ARGUMENT
        );
        for length in [0, (WC_MAX_BODY + 1) as u32] {
            assert_eq!(
                unsafe {
                    craftrustnx_world_decode_body(
                        ok,
                        length,
                        WC_PLAY,
                        &mut out_state,
                        &play,
                        &mut event,
                    )
                },
                WC_BAD_ARGUMENT
            );
        }
        assert_eq!(
            unsafe {
                craftrustnx_world_decode_body(
                    ok,
                    len,
                    WC_PLAY + 1,
                    &mut out_state,
                    &play,
                    &mut event,
                )
            },
            WC_BAD_ARGUMENT
        );
    }

    #[test]
    fn opaque_metadata_ids_report_only_counters() {
        let state = selected_state(-64, 16, 1);
        let play = play_state(0, 0.0, 0.0, 0.0);
        // Play id 11 (ChunkBatchFinished) is not this module's packet.
        let body = [11u8, 0xff, 0xff];
        let decoded = decode_world_body(&body, WC_PLAY, &state, &play).unwrap();
        assert_eq!(decoded.event.kind, WC_METADATA);
        assert_eq!(decoded.event.packet_id, 11);
        assert!(!decoded.state_changed);
        // Configuration packets other than 7 are metadata too.
        let body = [3u8];
        let decoded = decode_world_body(&body, WC_CONFIGURATION, &state, &play).unwrap();
        assert_eq!(decoded.event.kind, WC_METADATA);
        assert_eq!(decoded.event.packet_id, 3);
    }
}
