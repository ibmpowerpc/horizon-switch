//! The exact C ABI of `platform/switch/login-probe/include/column_snapshot.h`.
//!
//! Binary snapshot format: 48-byte header + block_ids + biome_ids.
//! Header: magic "CTCOL001" (8), protocol 777 (4), format 1 (4),
//! chunk_x (4), chunk_z (4), min_y (4), sections (4), blocks_written (4), biomes_written (4),
//! reserved (8).
//!
//! All integers are little-endian. No allocation, no std, no unsafe outside FFI.

use core::{mem::size_of, ptr, slice};

#[cfg(test)]
use crate::snapshot::header_size;
use crate::snapshot::{
    decode_header, decode_snapshot, encode_snapshot, MAX_FILE_SIZE, MAX_SECTIONS, PROTOCOL,
    SNAP_BAD_ARGUMENT, SNAP_BAD_FRAME, SNAP_OK, SNAP_OUTPUT_SMALL,
};

/// C struct mirroring the Rust ColInfo.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapInfo {
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub min_y: i32,
    pub sections: u32,
    pub blocks_written: u32,
    pub biomes_written: u32,
}

impl SnapInfo {
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

/// Validate a CTCOL001 header and return the parsed fields.
///
/// # Safety
///
/// `data` must be non-null and point to `length` readable bytes.
/// `length` must be at least 48.
/// `info` must be non-null and writable.
/// The input and info buffers must not overlap; info must be aligned.
#[no_mangle]
pub unsafe extern "C" fn craftrustnx_snapshot_decode_header(
    data: *const u8,
    length: u32,
    info: *mut SnapInfo,
) -> i32 {
    if info.is_null() {
        return SNAP_BAD_ARGUMENT;
    }
    unsafe { ptr::write_bytes(info.cast::<u8>(), 0, size_of::<SnapInfo>()) };

    if data.is_null() || length == 0 || length > MAX_FILE_SIZE as u32 {
        return SNAP_BAD_ARGUMENT;
    }
    if length < 48 {
        return SNAP_BAD_FRAME;
    }

    // SAFETY: caller guarantees non-null data and sufficient length.
    let data = unsafe { slice::from_raw_parts(data, length as usize) };

    match decode_header(data) {
        Ok((proto, format, chunk_x, chunk_z, min_y, sections, blocks_written, biomes_written)) => {
            if proto != PROTOCOL || format != 1 {
                return SNAP_BAD_FRAME;
            }
            // SAFETY: info is non-null and writable per earlier check.
            unsafe {
                ptr::addr_of_mut!((*info).chunk_x).write(chunk_x);
                ptr::addr_of_mut!((*info).chunk_z).write(chunk_z);
                ptr::addr_of_mut!((*info).min_y).write(min_y);
                ptr::addr_of_mut!((*info).sections).write(sections);
                ptr::addr_of_mut!((*info).blocks_written).write(blocks_written);
                ptr::addr_of_mut!((*info).biomes_written).write(biomes_written);
            }
            SNAP_OK
        }
        Err(-1) => SNAP_BAD_ARGUMENT,
        Err(-3) => SNAP_BAD_FRAME,
        _ => SNAP_BAD_FRAME,
    }
}

/// Encode a CTCOL001 snapshot into caller-owned output.
///
/// # Safety
///
/// Caller guarantees valid, aligned, non-overlapping buffers and truthful lengths.
/// `blocks`/`biomes` pointers are validated and sliced only after capacity checks.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn craftrustnx_snapshot_encode(
    chunk_x: i32,
    chunk_z: i32,
    min_y: i32,
    sections: u32,
    blocks: *const u32,
    blocks_written: u32,
    biomes: *const u32,
    biomes_written: u32,
    out: *mut u8,
    capacity: u32,
    written: *mut u32,
) -> i32 {
    if written.is_null() {
        return SNAP_BAD_ARGUMENT;
    }
    unsafe {
        *written = 0;
    }

    if out.is_null() {
        return SNAP_BAD_ARGUMENT;
    }

    if capacity == 0 || capacity > MAX_FILE_SIZE as u32 {
        return SNAP_BAD_ARGUMENT;
    }

    if sections == 0 || sections > MAX_SECTIONS as u32 {
        return SNAP_BAD_ARGUMENT;
    }
    if min_y % 16 != 0 || min_y.checked_add(sections as i32 * 16).is_none() {
        return SNAP_BAD_ARGUMENT;
    }

    if blocks_written != sections * 4096 || biomes_written != sections * 64 {
        return SNAP_BAD_ARGUMENT;
    }

    if blocks.is_null() || biomes.is_null() {
        return SNAP_BAD_ARGUMENT;
    }

    // SAFETY: pointers validated, capacities checked.
    let blocks = unsafe { slice::from_raw_parts(blocks, blocks_written as usize) };
    let biomes = unsafe { slice::from_raw_parts(biomes, biomes_written as usize) };
    let out = unsafe { slice::from_raw_parts_mut(out, capacity as usize) };

    match encode_snapshot(chunk_x, chunk_z, min_y, sections, blocks, biomes, out) {
        Ok(n) => {
            unsafe {
                *written = n as u32;
            }
            SNAP_OK
        }
        Err(-1) => SNAP_BAD_ARGUMENT,
        Err(-2) => SNAP_OUTPUT_SMALL,
        Err(-3) => SNAP_BAD_FRAME,
        _ => SNAP_BAD_FRAME,
    }
}

/// Decode a CTCOL001 snapshot into caller-owned arrays.
///
/// # Safety
///
/// Caller guarantees valid, aligned, non-overlapping buffers and truthful capacities.
/// All validations happen before any writes.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn craftrustnx_snapshot_decode(
    data: *const u8,
    length: u32,
    blocks: *mut u32,
    block_capacity: u32,
    biomes: *mut u32,
    biome_capacity: u32,
    info: *mut SnapInfo,
) -> i32 {
    if info.is_null() {
        return SNAP_BAD_ARGUMENT;
    }
    unsafe { ptr::write_bytes(info.cast::<u8>(), 0, size_of::<SnapInfo>()) };

    if data.is_null() || length == 0 || length > MAX_FILE_SIZE as u32 {
        return SNAP_BAD_ARGUMENT;
    }

    if blocks.is_null() || biomes.is_null() {
        return SNAP_BAD_ARGUMENT;
    }

    if block_capacity > 256 * 4096 || biome_capacity > 256 * 64 {
        return SNAP_BAD_ARGUMENT;
    }

    // SAFETY: pointers and lengths validated.
    let data = unsafe { slice::from_raw_parts(data, length as usize) };
    let blocks = unsafe { slice::from_raw_parts_mut(blocks, block_capacity as usize) };
    let biomes = unsafe { slice::from_raw_parts_mut(biomes, biome_capacity as usize) };

    match decode_snapshot(data, blocks, biomes) {
        Ok((chunk_x, chunk_z, min_y, sections, blocks_written, biomes_written)) => {
            // SAFETY: info is non-null and writable.
            unsafe {
                ptr::addr_of_mut!((*info).chunk_x).write(chunk_x);
                ptr::addr_of_mut!((*info).chunk_z).write(chunk_z);
                ptr::addr_of_mut!((*info).min_y).write(min_y);
                ptr::addr_of_mut!((*info).sections).write(sections);
                ptr::addr_of_mut!((*info).blocks_written).write(blocks_written);
                ptr::addr_of_mut!((*info).biomes_written).write(biomes_written);
            }
            SNAP_OK
        }
        Err(-1) => SNAP_BAD_ARGUMENT,
        Err(-2) => SNAP_OUTPUT_SMALL,
        Err(-3) => SNAP_BAD_FRAME,
        _ => SNAP_BAD_FRAME,
    }
}

/// Maximum file size in bytes: 48 + 256 * (4096 + 64) * 4.
///
/// # Safety
///
/// This function is safe to call from C. It returns the maximum file size in bytes.
#[no_mangle]
pub unsafe extern "C" fn craftrustnx_snapshot_max_file_size() -> u32 {
    MAX_FILE_SIZE as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    const SENTINEL: u32 = 0xDEAD_BEEF;

    fn make_blocks(count: usize) -> Vec<u32> {
        vec![0x1234_5678; count]
    }

    fn make_biomes(count: usize) -> Vec<u32> {
        vec![0xABCD_EF01; count]
    }

    #[test]
    fn c_abi_encode_decode_roundtrip() {
        let sections = 2u32;
        let blocks = make_blocks(sections as usize * 4096);
        let biomes = make_biomes(sections as usize * 64);

        let file_size = header_size(sections);
        let mut out = vec![0u8; file_size];
        let mut written: u32 = 0;

        let status = unsafe {
            craftrustnx_snapshot_encode(
                -3,
                7,
                -16,
                sections,
                blocks.as_ptr(),
                blocks.len() as u32,
                biomes.as_ptr(),
                biomes.len() as u32,
                out.as_mut_ptr(),
                out.len() as u32,
                &mut written,
            )
        };
        assert_eq!(status, SNAP_OK);
        assert_eq!(written, file_size as u32);

        let mut decoded_blocks = vec![SENTINEL; sections as usize * 4096];
        let mut decoded_biomes = vec![SENTINEL; sections as usize * 64];
        let mut info = SnapInfo::zeroed();

        let status = unsafe {
            craftrustnx_snapshot_decode(
                out.as_ptr(),
                written,
                decoded_blocks.as_mut_ptr(),
                decoded_blocks.len() as u32,
                decoded_biomes.as_mut_ptr(),
                decoded_biomes.len() as u32,
                &mut info,
            )
        };
        assert_eq!(status, SNAP_OK);
        assert_eq!(info.chunk_x, -3);
        assert_eq!(info.chunk_z, 7);
        assert_eq!(info.min_y, -16);
        assert_eq!(info.sections, 2);
        assert_eq!(info.blocks_written, 2 * 4096);
        assert_eq!(info.biomes_written, 2 * 64);
        assert_eq!(decoded_blocks, blocks);
        assert_eq!(decoded_biomes, biomes);
    }

    #[test]
    fn c_abi_decode_header() {
        let sections = 1u32;
        let blocks = make_blocks(4096);
        let biomes = make_biomes(64);

        let file_size = header_size(sections);
        let mut out = vec![0u8; file_size];
        let mut written: u32 = 0;

        let status = unsafe {
            craftrustnx_snapshot_encode(
                0,
                0,
                0,
                sections,
                blocks.as_ptr(),
                4096,
                biomes.as_ptr(),
                64,
                out.as_mut_ptr(),
                out.len() as u32,
                &mut written,
            )
        };
        assert_eq!(status, SNAP_OK);

        let mut info = SnapInfo::zeroed();
        let status =
            unsafe { craftrustnx_snapshot_decode_header(out.as_ptr(), written, &mut info) };
        assert_eq!(status, SNAP_OK);
        assert_eq!(info.sections, 1);
        assert_eq!(info.blocks_written, 4096);
        assert_eq!(info.biomes_written, 64);
    }

    #[test]
    fn c_abi_bad_arguments() {
        let mut info = SnapInfo::zeroed();
        let mut blocks = make_blocks(4096);
        let mut biomes = make_biomes(64);
        let mut out = vec![0u8; 1024];
        let mut written: u32 = 0;

        // Null info
        assert_eq!(
            unsafe { craftrustnx_snapshot_decode_header(out.as_ptr(), 48, core::ptr::null_mut()) },
            SNAP_BAD_ARGUMENT
        );
        assert_eq!(
            unsafe {
                craftrustnx_snapshot_decode(
                    out.as_ptr(),
                    48,
                    blocks.as_mut_ptr(),
                    4096,
                    biomes.as_mut_ptr(),
                    64,
                    core::ptr::null_mut(),
                )
            },
            SNAP_BAD_ARGUMENT
        );
        assert_eq!(
            unsafe {
                craftrustnx_snapshot_encode(
                    0,
                    0,
                    0,
                    1,
                    blocks.as_ptr(),
                    4096,
                    biomes.as_ptr(),
                    64,
                    out.as_mut_ptr(),
                    out.len() as u32,
                    core::ptr::null_mut(),
                )
            },
            SNAP_BAD_ARGUMENT
        );

        // Null data
        assert_eq!(
            unsafe { craftrustnx_snapshot_decode_header(core::ptr::null(), 48, &mut info) },
            SNAP_BAD_ARGUMENT
        );
        assert_eq!(
            unsafe {
                craftrustnx_snapshot_decode(
                    core::ptr::null(),
                    48,
                    blocks.as_mut_ptr(),
                    4096,
                    biomes.as_mut_ptr(),
                    64,
                    &mut info,
                )
            },
            SNAP_BAD_ARGUMENT
        );

        // Null output
        assert_eq!(
            unsafe {
                craftrustnx_snapshot_encode(
                    0,
                    0,
                    0,
                    1,
                    blocks.as_ptr(),
                    4096,
                    biomes.as_ptr(),
                    64,
                    core::ptr::null_mut(),
                    1024,
                    &mut written,
                )
            },
            SNAP_BAD_ARGUMENT
        );

        // Null blocks/biomes
        assert_eq!(
            unsafe {
                craftrustnx_snapshot_encode(
                    0,
                    0,
                    0,
                    1,
                    core::ptr::null(),
                    4096,
                    biomes.as_ptr(),
                    64,
                    out.as_mut_ptr(),
                    out.len() as u32,
                    &mut written,
                )
            },
            SNAP_BAD_ARGUMENT
        );
        assert_eq!(
            unsafe {
                craftrustnx_snapshot_encode(
                    0,
                    0,
                    0,
                    1,
                    blocks.as_ptr(),
                    4096,
                    core::ptr::null(),
                    64,
                    out.as_mut_ptr(),
                    out.len() as u32,
                    &mut written,
                )
            },
            SNAP_BAD_ARGUMENT
        );
        assert_eq!(
            unsafe {
                craftrustnx_snapshot_decode(
                    48u8.to_le_bytes().as_ptr(),
                    48,
                    core::ptr::null_mut(),
                    4096,
                    biomes.as_mut_ptr(),
                    64,
                    &mut info,
                )
            },
            SNAP_BAD_ARGUMENT
        );
        assert_eq!(
            unsafe {
                craftrustnx_snapshot_decode(
                    48u8.to_le_bytes().as_ptr(),
                    48,
                    blocks.as_mut_ptr(),
                    4096,
                    core::ptr::null_mut(),
                    64,
                    &mut info,
                )
            },
            SNAP_BAD_ARGUMENT
        );

        // Zero capacity
        assert_eq!(
            unsafe {
                craftrustnx_snapshot_encode(
                    0,
                    0,
                    0,
                    1,
                    blocks.as_ptr(),
                    4096,
                    biomes.as_ptr(),
                    64,
                    out.as_mut_ptr(),
                    0,
                    &mut written,
                )
            },
            SNAP_BAD_ARGUMENT
        );
    }

    #[test]
    fn c_abi_output_small() {
        let sections = 1u32;
        let blocks = make_blocks(4096);
        let biomes = make_biomes(64);

        let mut out = vec![0u8; 100]; // Too small
        let mut written: u32 = 0;

        let status = unsafe {
            craftrustnx_snapshot_encode(
                0,
                0,
                0,
                sections,
                blocks.as_ptr(),
                4096,
                biomes.as_ptr(),
                64,
                out.as_mut_ptr(),
                out.len() as u32,
                &mut written,
            )
        };
        assert_eq!(status, SNAP_OUTPUT_SMALL);
        assert_eq!(written, 0);

        // Short arrays on decode
        let mut out = vec![0u8; header_size(sections)];
        let mut written: u32 = 0;
        unsafe {
            craftrustnx_snapshot_encode(
                0,
                0,
                0,
                sections,
                blocks.as_ptr(),
                4096,
                biomes.as_ptr(),
                64,
                out.as_mut_ptr(),
                out.len() as u32,
                &mut written,
            );
        }

        let mut short_blocks = vec![SENTINEL; 4095];
        let mut short_biomes = vec![SENTINEL; 64];
        let mut info = SnapInfo::zeroed();

        let status = unsafe {
            craftrustnx_snapshot_decode(
                out.as_ptr(),
                written,
                short_blocks.as_mut_ptr(),
                short_blocks.len() as u32,
                short_biomes.as_mut_ptr(),
                short_biomes.len() as u32,
                &mut info,
            )
        };
        assert_eq!(status, SNAP_OUTPUT_SMALL);
        assert_eq!(info, SnapInfo::zeroed());
    }

    #[test]
    fn c_abi_bad_frame() {
        let mut info = SnapInfo::zeroed();
        let mut blocks = make_blocks(4096);
        let mut biomes = make_biomes(64);
        let mut out = vec![0u8; header_size(1)];

        // Corrupt magic
        out[0] = b'X';
        let status = unsafe {
            craftrustnx_snapshot_decode(
                out.as_ptr(),
                48,
                blocks.as_mut_ptr(),
                4096,
                biomes.as_mut_ptr(),
                64,
                &mut info,
            )
        };
        assert_eq!(status, SNAP_BAD_FRAME);
        assert_eq!(info, SnapInfo::zeroed());

        // Too short
        let status = unsafe {
            craftrustnx_snapshot_decode(
                out.as_ptr(),
                47,
                blocks.as_mut_ptr(),
                4096,
                biomes.as_mut_ptr(),
                64,
                &mut info,
            )
        };
        assert_eq!(status, SNAP_BAD_FRAME);
        assert_eq!(info, SnapInfo::zeroed());
    }

    #[test]
    fn c_abi_max_file_size() {
        let expected = MAX_FILE_SIZE as u32;
        let actual = unsafe { craftrustnx_snapshot_max_file_size() };
        assert_eq!(actual, expected);
    }
}
