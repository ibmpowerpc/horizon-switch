//! Exact C ABI for the `no_std` Java status codec, per
//! `platform/switch/net-probe/include/status_codec.h`.
//!
//! The crate is a `staticlib` (`libcraftrustnx_status_ffi.a`) and owns no state:
//! no allocation, logging, networking or globals. All buffers are caller-owned.
//! Packet framing is *not* re-implemented here; every byte comes from
//! `craftrustnx-net-probe-codec`, the probe's single source of wire truth.
//!
//! Status codes returned by the exports match the header enum exactly:
//! `OK = 0`, `NEED_MORE = 1`, `BAD_ARGUMENT = -1`, `OUTPUT_SMALL = -2`,
//! `BAD_FRAME = -3`, `BAD_HOST = -4`.
#![cfg_attr(not(test), no_std)]

use core::slice;

use craftrustnx_net_probe_codec as codec;

const STATUS_OK: i32 = 0;
const STATUS_NEED_MORE: i32 = 1;
const STATUS_BAD_ARGUMENT: i32 = -1;
const STATUS_OUTPUT_SMALL: i32 = -2;
const STATUS_BAD_FRAME: i32 = -3;
const STATUS_BAD_HOST: i32 = -4;

/// Longest host string the codec accepts (253 printable ASCII bytes).
const MAX_HOST_LEN: u32 = 253;

fn status_of(error: codec::Error) -> i32 {
    match error {
        codec::Error::OutputTooSmall => STATUS_OUTPUT_SMALL,
        codec::Error::NeedMore => STATUS_NEED_MORE,
        codec::Error::InvalidHost => STATUS_BAD_HOST,
        codec::Error::VarIntOverflow
        | codec::Error::InvalidLength
        | codec::Error::InvalidUtf8
        | codec::Error::InvalidPacketId
        | codec::Error::FrameLengthMismatch => STATUS_BAD_FRAME,
    }
}

/// Encode the two complete TCP frames of a server-list status request
/// (handshake with next-state 1, then the empty status request).
///
/// `*written` is set to 0 before anything else, so every error leaves it 0 and
/// a success leaves the exact byte count. On an error `out` is not written.
///
/// # Safety
///
/// The caller guarantees:
/// - `written` is null or points to one writable, aligned `uint32_t` in
///   caller-owned memory that does not overlap `host` or `out`;
/// - when `host_len` is non-zero, `host` points to `host_len` readable bytes;
///   `host_len` bytes are only read when `1..=253`, so an out-of-range length
///   never dereferences `host`;
/// - when `capacity` is non-zero, `out` points to `capacity` writable bytes that
///   do not overlap `host` or `written`;
/// - the pointers are valid for the whole call and stay so until it returns.
///
/// Arbitrary pointer values cannot be validated here; a pointer that does not
/// meet the above is undefined behaviour, not an error code.
#[no_mangle]
pub unsafe extern "C" fn craftrustnx_status_encode(
    host: *const core::ffi::c_char,
    host_len: u32,
    port: u16,
    protocol: i32,
    out: *mut u8,
    capacity: u32,
    written: *mut u32,
) -> i32 {
    if written.is_null() {
        return STATUS_BAD_ARGUMENT;
    }
    unsafe { *written = 0 };

    // Bound the length before forming any slice: an oversized `host_len` is a
    // host error, and is decided without reading through `host`.
    if host_len == 0 || host_len > MAX_HOST_LEN {
        return STATUS_BAD_HOST;
    }
    if host.is_null() || out.is_null() {
        return STATUS_BAD_ARGUMENT;
    }
    if capacity == 0 {
        return STATUS_OUTPUT_SMALL;
    }

    let host_bytes = unsafe { slice::from_raw_parts(host.cast::<u8>(), host_len as usize) };
    let Ok(host) = core::str::from_utf8(host_bytes) else {
        return STATUS_BAD_HOST;
    };
    let out = unsafe { slice::from_raw_parts_mut(out, capacity as usize) };

    match codec::encode_status_request(host, port, protocol, out) {
        Ok(n) => {
            unsafe { *written = n as u32 };
            STATUS_OK
        }
        Err(error) => status_of(error),
    }
}

/// Parse one complete server status response frame.
///
/// All three outputs are reset to 0 first, so an error never leaves stale
/// values behind. On success `*json_offset` is relative to `data` (not to the
/// end of the outer frame length prefix), `*json_length` counts the JSON bytes
/// and `*consumed` excludes any following TCP frame such as a pong.
///
/// # Safety
///
/// The caller guarantees:
/// - `json_offset`, `json_length` and `consumed` are null or point to one
///   writable, aligned `uint32_t` each, in caller-owned memory that does not
///   overlap `data`;
/// - when `length` is non-zero, `data` points to `length` readable bytes;
///   `length` bytes are only read when `data` is non-null;
/// - the pointers are valid for the whole call and stay so until it returns.
///
/// Arbitrary pointer values cannot be validated here; `length` itself is only
/// trusted as a `usize` conversion, so a length larger than the real buffer is
/// undefined behaviour, not an error code.
#[no_mangle]
pub unsafe extern "C" fn craftrustnx_status_decode(
    data: *const u8,
    length: u32,
    json_offset: *mut u32,
    json_length: *mut u32,
    consumed: *mut u32,
) -> i32 {
    if !json_offset.is_null() {
        unsafe { *json_offset = 0 };
    }
    if !json_length.is_null() {
        unsafe { *json_length = 0 };
    }
    if !consumed.is_null() {
        unsafe { *consumed = 0 };
    }
    if json_offset.is_null() || json_length.is_null() || consumed.is_null() {
        return STATUS_BAD_ARGUMENT;
    }
    if data.is_null() {
        // An empty stream is incomplete, not a bad pointer.
        return if length == 0 {
            STATUS_NEED_MORE
        } else {
            STATUS_BAD_ARGUMENT
        };
    }
    let Ok(length) = usize::try_from(length) else {
        return STATUS_BAD_ARGUMENT;
    };

    let input = unsafe { slice::from_raw_parts(data, length) };
    match codec::parse_status_frame(input) {
        Ok(frame) => {
            let json = frame.json.len() as u32;
            unsafe {
                *json_length = json;
                *json_offset = (frame.consumed - frame.json.len()) as u32;
                *consumed = frame.consumed as u32;
            }
            STATUS_OK
        }
        Err(error) => status_of(error),
    }
}

/// Bytes a caller must be able to buffer for one complete response frame:
/// the codec's maximum body plus the widest outer length prefix (five bytes).
#[no_mangle]
pub extern "C" fn craftrustnx_status_max_frame_bytes() -> u32 {
    (codec::MAX_FRAME_BYTES + 5) as u32
}

// Do not unwind across C or leave the UI waiting forever after a panic.
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    extern "C" {
        fn abort() -> !;
    }
    unsafe { abort() }
}

// Precompiled `core` can still emit unwind-table references to this personality
// even with `panic = "abort"`, and a `no_std` staticlib has no std to provide
// it. Without the shim a C linker stops on `_rust_eh_personality`; with it the
// archive links into a plain host program. Switch's abort-built core does not
// need or include this host-only shim.
#[cfg(all(not(test), not(target_os = "horizon")))]
#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

#[cfg(test)]
mod tests {
    use super::*;
    use core::ptr;

    /// Independent wire vector: localhost:25565, protocol 777.
    const REQUEST: [u8; 19] = *b"\x10\x00\x89\x06\x09localhost\x63\xdd\x01\x01\x00";
    /// length 4, id 0, string length 2, "{}".
    const RESPONSE: [u8; 5] = [4, 0, 2, b'{', b'}'];

    fn encode(host: &[u8], port: u16, protocol: i32, out: &mut [u8], written: &mut u32) -> i32 {
        unsafe {
            craftrustnx_status_encode(
                host.as_ptr().cast(),
                host.len() as u32,
                port,
                protocol,
                out.as_mut_ptr(),
                out.len() as u32,
                written,
            )
        }
    }

    fn decode(input: &[u8], out: &mut [u32; 3]) -> i32 {
        unsafe {
            craftrustnx_status_decode(
                input.as_ptr(),
                input.len() as u32,
                &mut out[0],
                &mut out[1],
                &mut out[2],
            )
        }
    }

    #[test]
    fn encode_matches_independent_wire_vector() {
        let mut out = [0xa5u8; 64];
        let mut written = 0xdead_beef;
        assert_eq!(
            encode(b"localhost", 25565, 777, &mut out, &mut written),
            STATUS_OK
        );
        assert_eq!(written, REQUEST.len() as u32);
        assert_eq!(&out[..written as usize], &REQUEST);

        // Signed protocol -1 is the five-byte VarInt form.
        assert_eq!(
            encode(b"localhost", 25565, -1, &mut out, &mut written),
            STATUS_OK
        );
        assert_eq!(
            &out[..written as usize],
            b"\x13\x00\xff\xff\xff\xff\x0f\x09localhost\x63\xdd\x01\x01\x00"
        );
    }

    #[test]
    fn decode_reports_offsets_relative_to_input_and_stops_at_one_frame() {
        let mut joined = [0u8; 7];
        joined[..5].copy_from_slice(&RESPONSE);
        joined[5..].copy_from_slice(&[1, 0]); // Arbitrary trailing frame, unconsumed.

        let mut out = [0xffff_ffffu32; 3];
        assert_eq!(decode(&joined, &mut out), STATUS_OK);
        assert_eq!(out, [3, 2, 5]);
        assert_eq!(&joined[out[0] as usize..][..out[1] as usize], b"{}");
    }

    #[test]
    fn every_incomplete_prefix_needs_more_and_clears_outputs() {
        for end in 0..RESPONSE.len() {
            let mut out = [7u32; 3];
            assert_eq!(decode(&RESPONSE[..end], &mut out), STATUS_NEED_MORE);
            assert_eq!(out, [0, 0, 0]);
        }
        let empty: [u8; 0] = [];
        let mut out = [7u32; 3];
        assert_eq!(decode(&empty, &mut out), STATUS_NEED_MORE);
        assert_eq!(out, [0, 0, 0]);
    }

    #[test]
    fn malformed_frames_fail_and_clear_outputs() {
        let cases: [(&[u8], i32); 4] = [
            (&[2, 1, 0], STATUS_BAD_FRAME),                      // packet id 1
            (&[3, 0, 1, 0xff], STATUS_BAD_FRAME),                // invalid UTF-8
            (&[0xff, 0xff, 0xff, 0xff, 0x0f], STATUS_BAD_FRAME), // length 0xffffffff
            (&[3, 0, 2, b'{'], STATUS_BAD_FRAME),                // inner mismatch
        ];
        for (bytes, expected) in cases {
            let mut out = [0xffff_ffffu32; 3];
            assert_eq!(decode(bytes, &mut out), expected);
            assert_eq!(out, [0, 0, 0]);
        }
    }

    #[test]
    fn small_output_and_bad_hosts_leave_out_untouched() {
        let mut out = [0xa5u8; 19];
        for capacity in 0..REQUEST.len() {
            let mut written = 9;
            let rc = unsafe {
                craftrustnx_status_encode(
                    b"localhost".as_ptr().cast(),
                    9,
                    25565,
                    777,
                    out.as_mut_ptr(),
                    capacity as u32,
                    &mut written,
                )
            };
            assert_eq!(rc, STATUS_OUTPUT_SMALL);
            assert_eq!(written, 0);
            assert_eq!(out, [0xa5; 19]);
        }

        let mut ok = [0u8; 64];
        let mut written = 1;
        for host in [&[][..], b"bad host", b"bad\0host", &[0xff][..]] {
            assert_eq!(
                encode(host, 25565, 777, &mut ok, &mut written),
                STATUS_BAD_HOST
            );
            assert_eq!(written, 0);
        }
        // 253 bytes is accepted, 254 is rejected.
        let longest = [b'a'; 253];
        let mut roomy = [0u8; 512];
        assert_eq!(
            encode(&longest, 25565, 777, &mut roomy, &mut written),
            STATUS_OK
        );
        assert_eq!(written, 2 + 1 + 2 + 2 + 253 + 2 + 1 + 2);
        assert_eq!(
            encode(&[b'a'; 254], 25565, 777, &mut roomy, &mut written),
            STATUS_BAD_HOST
        );
        assert_eq!(written, 0);
    }

    #[test]
    fn null_pointers_are_arguments_not_dereferences() {
        let mut ok = [0u8; 64];
        let rc = unsafe {
            craftrustnx_status_encode(
                b"localhost".as_ptr().cast(),
                9,
                25565,
                777,
                ok.as_mut_ptr(),
                ok.len() as u32,
                ptr::null_mut(),
            )
        };
        assert_eq!(rc, STATUS_BAD_ARGUMENT);

        let mut out = [9u32; 3];
        assert_eq!(
            unsafe {
                craftrustnx_status_decode(
                    RESPONSE.as_ptr(),
                    RESPONSE.len() as u32,
                    ptr::null_mut(),
                    &mut out[1],
                    &mut out[2],
                )
            },
            STATUS_BAD_ARGUMENT
        );
        assert_eq!(out, [9, 0, 0]);

        for (host, len, out_ptr) in [
            (ptr::null(), 9u32, ok.as_mut_ptr()),
            (b"localhost".as_ptr().cast(), 9, ptr::null_mut()),
        ] {
            let mut written = 5;
            let rc = unsafe {
                craftrustnx_status_encode(host, len, 25565, 777, out_ptr, 64, &mut written)
            };
            assert_eq!(rc, STATUS_BAD_ARGUMENT);
            assert_eq!(written, 0);
        }

        let mut out = [9u32; 3];
        assert_eq!(
            unsafe {
                craftrustnx_status_decode(ptr::null(), 9, &mut out[0], &mut out[1], &mut out[2])
            },
            STATUS_BAD_ARGUMENT
        );
        assert_eq!(out, [0, 0, 0]);
    }

    #[test]
    fn oversize_host_is_rejected_without_reading_it() {
        // A one-byte allocation with a 254-byte length: nothing may read past it.
        let one = [b'a'; 1];
        let mut ok = [0u8; 300];
        let mut written = 3;
        let rc = unsafe {
            craftrustnx_status_encode(
                one.as_ptr().cast(),
                254,
                25565,
                777,
                ok.as_mut_ptr(),
                ok.len() as u32,
                &mut written,
            )
        };
        assert_eq!(rc, STATUS_BAD_HOST);
        assert_eq!(written, 0);
        assert_eq!(ok, [0u8; 300]);
        assert_eq!(craftrustnx_status_max_frame_bytes(), 16_399);
    }
}
