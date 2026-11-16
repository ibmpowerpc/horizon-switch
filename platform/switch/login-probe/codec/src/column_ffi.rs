//! The exact C ABI of `platform/switch/login-probe/include/column_codec.h`.
//!
//! Every buffer is caller-owned. Nothing here allocates, logs, opens a socket,
//! installs a registry or simulates a world. Pointer, length and capacity
//! checks happen before any slice is formed. On every failure `info` is zeroed
//! (all 24 bytes, padding included — there is no padding today) and the two
//! output arrays and the input states are untouched.

use core::{mem::size_of, ptr, slice};

use crate::column::{
    decode_column_body, ColInfo, ColumnError, COL_BAD_ARGUMENT, COL_BAD_FRAME, COL_LIMIT,
    COL_MAX_BIOMES, COL_MAX_BLOCKS, COL_OK, COL_OUTPUT_SMALL, COL_UNSUPPORTED,
};
use crate::play::PlState;
use crate::world::{WcState, WC_MAX_BODY};

fn status_of(error: ColumnError) -> i32 {
    match error {
        ColumnError::BadArgument => COL_BAD_ARGUMENT,
        ColumnError::OutputSmall => COL_OUTPUT_SMALL,
        ColumnError::BadFrame => COL_BAD_FRAME,
        ColumnError::Unsupported => COL_UNSUPPORTED,
        ColumnError::Limit => COL_LIMIT,
    }
}

/// Write every field of `info` from a completed copy.
///
/// # Safety
///
/// `info` must be non-null, writable and aligned for `ColInfo`.
unsafe fn write_info(info: *mut ColInfo, value: &ColInfo) {
    // SAFETY: the caller contract guarantees a writable, aligned region.
    unsafe {
        ptr::addr_of_mut!((*info).chunk_x).write(value.chunk_x);
        ptr::addr_of_mut!((*info).chunk_z).write(value.chunk_z);
        ptr::addr_of_mut!((*info).min_y).write(value.min_y);
        ptr::addr_of_mut!((*info).sections).write(value.sections);
        ptr::addr_of_mut!((*info).blocks_written).write(value.blocks_written);
        ptr::addr_of_mut!((*info).biomes_written).write(value.biomes_written);
    }
}

/// Validate one complete, already-unframed and already-inflated play id 46
/// body and copy its normalized block-state and biome ids into the caller's
/// arrays.
///
/// # Safety
///
/// The caller guarantees:
/// - when `length` is in `1..=WC_MAX_BODY`, `body` points to `length` readable
///   bytes; out-of-contract lengths are rejected before `body` is read;
/// - `world` and `play` point to readable, aligned, `struct wc_state`-
///   and `struct pl_state`-sized regions;
/// - `blocks` and `biomes` are non-null, 4-byte aligned and point to
///   `block_capacity` and `biome_capacity` writable `uint32_t` elements, at
///   most `COL_MAX_BLOCKS` and `COL_MAX_BIOMES`;
/// - `info` points to one writable, aligned, `struct col_info`-sized region;
/// - none of the regions overlap, and every pointer stays valid for the call.
///
/// Arbitrary pointer values cannot be validated; anything that breaks the
/// above is undefined behaviour, not an error code.
#[no_mangle]
#[allow(clippy::too_many_arguments)] // The C header fixes the parameter list.
pub unsafe extern "C" fn craftrustnx_column_copy(
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
    if info.is_null() {
        return COL_BAD_ARGUMENT;
    }
    // SAFETY: `info` is non-null, writable and sized per the contract above.
    // The failure shape is written first, so every early return below leaves
    // it all-zero.
    unsafe { ptr::write_bytes(info.cast::<u8>(), 0, size_of::<ColInfo>()) };

    if world.is_null() || play.is_null() {
        return COL_BAD_ARGUMENT;
    }
    if length == 0 || length as usize > WC_MAX_BODY {
        return COL_BAD_ARGUMENT;
    }
    if body.is_null() {
        return COL_BAD_ARGUMENT;
    }
    if block_capacity as usize > COL_MAX_BLOCKS || biome_capacity as usize > COL_MAX_BIOMES {
        return COL_BAD_ARGUMENT;
    }
    if blocks.is_null() || biomes.is_null() {
        return COL_BAD_ARGUMENT;
    }

    // SAFETY: the lengths and pointers are in contract per the checks above;
    // the caller guarantees the regions and their non-overlap.
    let body = unsafe { slice::from_raw_parts(body, length as usize) };
    let state: &WcState = unsafe { &*world };
    let play_ref: &PlState = unsafe { &*play };
    let blocks = unsafe { slice::from_raw_parts_mut(blocks, block_capacity as usize) };
    let biomes = unsafe { slice::from_raw_parts_mut(biomes, biome_capacity as usize) };

    match decode_column_body(body, state, play_ref, blocks, biomes) {
        Ok(value) => {
            // SAFETY: `info` is non-null, writable and sized per the contract.
            unsafe { write_info(info, &value) };
            COL_OK
        }
        // `*info` is already zero and nothing else was written.
        Err(error) => status_of(error),
    }
}
