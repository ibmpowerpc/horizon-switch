//! The exact C ABI of `platform/switch/login-probe/include/world_codec.h`.
//!
//! Every buffer is caller-owned. Nothing here allocates, logs, opens a socket,
//! loads a registry or simulates a world. Pointer and length checks happen
//! before any slice is formed. A failure leaves the documented state: every
//! byte of `event` (padding included) is zero, and `state` is untouched.

use core::{mem::size_of, ptr, slice};

use crate::play::PlState;
use crate::world::{
    decode_world_body, WcEvent, WcState, WorldError, WC_BAD_ARGUMENT, WC_BAD_FRAME, WC_LIMIT,
    WC_MAX_BODY, WC_MAX_DIMENSIONS, WC_OK, WC_PLAY, WC_UNSUPPORTED,
};

fn status_of(error: WorldError) -> i32 {
    match error {
        WorldError::BadArgument => WC_BAD_ARGUMENT,
        WorldError::BadFrame => WC_BAD_FRAME,
        WorldError::Unsupported => WC_UNSUPPORTED,
        WorldError::Limit => WC_LIMIT,
    }
}

/// Write every field of `state` from a fully built copy. The struct has no
/// padding (all fields are 4-byte scalars or packed structs), but fields are
/// still stored individually, matching the other ABI modules.
///
/// # Safety
///
/// `state` must be non-null, writable and aligned for `WcState`.
unsafe fn commit_state(state: *mut WcState, value: &WcState) {
    // SAFETY: the caller contract guarantees a writable, aligned region.
    unsafe {
        ptr::addr_of_mut!((*state).dimension_count).write(value.dimension_count);
        ptr::addr_of_mut!((*state).biome_count).write(value.biome_count);
        ptr::addr_of_mut!((*state).dimensions_seen).write(value.dimensions_seen);
        ptr::addr_of_mut!((*state).biomes_seen).write(value.biomes_seen);
        ptr::copy_nonoverlapping(
            value.dimensions.as_ptr(),
            ptr::addr_of_mut!((*state).dimensions).cast::<crate::world::WcDimension>(),
            WC_MAX_DIMENSIONS,
        );
    }
}

/// Write every field of `event` from a decoded value.
///
/// # Safety
///
/// `event` must be non-null, writable and aligned for `WcEvent`.
unsafe fn write_event(event: *mut WcEvent, value: &WcEvent) {
    // SAFETY: the caller contract guarantees a writable, aligned region.
    unsafe {
        ptr::addr_of_mut!((*event).kind).write(value.kind);
        ptr::addr_of_mut!((*event).packet_id).write(value.packet_id);
        ptr::addr_of_mut!((*event).entries).write(value.entries);
        ptr::addr_of_mut!((*event).chunk_x).write(value.chunk_x);
        ptr::addr_of_mut!((*event).chunk_z).write(value.chunk_z);
        ptr::addr_of_mut!((*event).sections).write(value.sections);
        ptr::addr_of_mut!((*event).heightmaps).write(value.heightmaps);
        ptr::addr_of_mut!((*event).block_entities).write(value.block_entities);
        ptr::addr_of_mut!((*event).sky_layers).write(value.sky_layers);
        ptr::addr_of_mut!((*event).block_layers).write(value.block_layers);
        ptr::addr_of_mut!((*event).nonempty_blocks).write(value.nonempty_blocks);
        ptr::addr_of_mut!((*event).fluid_blocks).write(value.fluid_blocks);
        ptr::addr_of_mut!((*event).sample_valid).write(value.sample_valid);
        ptr::addr_of_mut!((*event).sample_x).write(value.sample_x);
        ptr::addr_of_mut!((*event).sample_y).write(value.sample_y);
        ptr::addr_of_mut!((*event).sample_z).write(value.sample_z);
        ptr::addr_of_mut!((*event).state_id).write(value.state_id);
        ptr::addr_of_mut!((*event).biome_id).write(value.biome_id);
    }
}

/// Decode one complete, already-unframed and already-inflated body and fill
/// `*state` (only when the registry state changed) and `*event`.
///
/// `body` carries the packet id first and at most `WC_MAX_BODY` (8 MiB) bytes.
/// `phase` is `WC_CONFIGURATION` (0) or `WC_PLAY` (1). `play` is the play
/// decoder's state: its dimension type selects the retained dimension and its
/// acknowledged pose is the sample reference.
///
/// # Safety
///
/// The caller guarantees:
/// - when `length` is in `1..=WC_MAX_BODY`, `body` points to `length` readable
///   bytes; out-of-contract lengths are rejected before `body` is read;
/// - `state` points to one writable, aligned, `struct wc_state`-sized region
///   that does not overlap `body`, `play` or `event`;
/// - `play` points to one readable, aligned, `struct pl_state`-sized region;
/// - `event` points to one writable, aligned, `struct wc_event`-sized region;
/// - none of the regions overlap, and every pointer stays valid for the call.
///
/// Arbitrary pointer values cannot be validated; anything that breaks the
/// above is undefined behaviour, not an error code.
#[no_mangle]
pub unsafe extern "C" fn craftrustnx_world_decode_body(
    body: *const u8,
    length: u32,
    phase: u32,
    state: *mut WcState,
    play: *const PlState,
    event: *mut WcEvent,
) -> i32 {
    if event.is_null() {
        return WC_BAD_ARGUMENT;
    }
    // SAFETY: `event` is non-null, writable and sized per the contract above.
    unsafe { ptr::write_bytes(event.cast::<u8>(), 0, size_of::<WcEvent>()) };

    if state.is_null() || play.is_null() {
        return WC_BAD_ARGUMENT;
    }
    if length == 0 || length as usize > WC_MAX_BODY {
        return WC_BAD_ARGUMENT;
    }
    if body.is_null() {
        return WC_BAD_ARGUMENT;
    }
    if phase > WC_PLAY {
        return WC_BAD_ARGUMENT;
    }

    // SAFETY: the length and pointers are in contract per the checks above;
    // the caller guarantees the regions and their non-overlap.
    let body = unsafe { slice::from_raw_parts(body, length as usize) };
    let state_ref: &WcState = unsafe { &*state };
    let play_ref: &PlState = unsafe { &*play };

    match decode_world_body(body, phase, state_ref, play_ref) {
        Ok(decoded) => {
            // SAFETY: `state` and `event` are non-null, writable and sized per
            // the contract; only a changed registry state is committed.
            unsafe {
                if decoded.state_changed {
                    commit_state(state, &decoded.state);
                }
                write_event(event, &decoded.event);
            }
            WC_OK
        }
        // `*event` is already zero, padding included, and `state` was not
        // written.
        Err(error) => status_of(error),
    }
}
