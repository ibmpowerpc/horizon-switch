//! The exact C ABI of `platform/switch/login-probe/include/login_codec.h`.
//!
//! Every buffer is caller-owned; nothing here allocates, logs or opens a
//! socket, and no reply (LoginAcknowledged, configuration or play) is ever
//! produced. Pointer and length checks happen before any slice is formed, and
//! a failure leaves the caller's outputs in their documented state: `written`
//! is `0` and the encoded bytes are untouched for `start`; every byte of the
//! event (padding included) is zero for `decode`.

use core::ffi::c_char;
use core::{ptr, slice};

use crate::decode::{decode_login_frame, Decoded, Event};
use crate::{encode_login_start, Error, MAX_BODY, MAX_HOST, MAX_NAME, MAX_WIRE};

pub const LP_OK: i32 = 0;
pub const LP_NEED_MORE: i32 = 1;
pub const LP_BAD_ARGUMENT: i32 = -1;
pub const LP_OUTPUT_SMALL: i32 = -2;
pub const LP_BAD_FRAME: i32 = -3;
pub const LP_BAD_PROFILE: i32 = -4;
pub const LP_UNSUPPORTED: i32 = -5;
pub const LP_MAX_BODY: u32 = MAX_BODY as u32;
pub const LP_MAX_WIRE: u32 = MAX_WIRE as u32;

pub const LP_LOGIN_SUCCESS: u32 = 1;
pub const LP_DISCONNECT: u32 = 2;
pub const LP_ENCRYPTION_REQUEST: u32 = 3;
pub const LP_SET_COMPRESSION: u32 = 4;
pub const LP_CUSTOM_QUERY: u32 = 5;
pub const LP_COOKIE_REQUEST: u32 = 6;

/// `struct lp_event`: exactly the fields and order of the C header.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LpEvent {
    pub kind: u32,
    pub consumed: u32,
    /// Payload offset in the caller's scratch, after the packet ID.
    pub data_offset: u32,
    pub data_length: u32,
    /// Only meaningful for [`LP_SET_COMPRESSION`].
    pub threshold: i32,
    pub uuid: [u8; 16],
    /// Server profile name, NUL terminated; only for [`LP_LOGIN_SUCCESS`].
    pub name: [c_char; 17],
    /// Java 26.3 `LoginFinished` session UUID.
    pub session: [u8; 16],
}

fn status_of(error: Error) -> i32 {
    match error {
        Error::BadArgument => LP_BAD_ARGUMENT,
        Error::OutputSmall => LP_OUTPUT_SMALL,
        Error::NeedMore => LP_NEED_MORE,
        Error::BadFrame => LP_BAD_FRAME,
        Error::BadProfile => LP_BAD_PROFILE,
        Error::Unsupported => LP_UNSUPPORTED,
    }
}

fn event_of(decoded: Decoded) -> LpEvent {
    // Every field permits zero. Padding is not guaranteed to survive a typed
    // struct move, so the FFI writes fields separately into zeroed output.
    let mut event: LpEvent = unsafe { core::mem::zeroed() };
    event.consumed = decoded.consumed as u32;
    event.data_offset = decoded.data_offset as u32;
    event.data_length = decoded.data_length as u32;
    event.kind = match decoded.event {
        Event::Success {
            uuid,
            name,
            session,
            ..
        } => {
            event.uuid = uuid;
            event.name = name.map(|byte| byte as c_char);
            event.session = session;
            LP_LOGIN_SUCCESS
        }
        Event::Disconnect => LP_DISCONNECT,
        Event::Hello { .. } => LP_ENCRYPTION_REQUEST,
        Event::Compression { threshold } => {
            event.threshold = threshold;
            LP_SET_COMPRESSION
        }
        Event::CustomQuery { .. } => LP_CUSTOM_QUERY,
        Event::CookieRequest => LP_COOKIE_REQUEST,
    };
    event
}

/// Encode the login request: handshake (next state 2) + LoginStart (offline
/// UUID v3), as two complete TCP frames.
///
/// `*written` is set to 0 first, so every failure leaves it 0. On failure `out`
/// is never written. Invalid host, name or port are `LP_BAD_ARGUMENT`; a
/// too-small `out` is `LP_OUTPUT_SMALL`.
///
/// # Safety
///
/// The caller guarantees:
/// - `written` is null or points to one writable, aligned `uint32_t` in
///   caller-owned memory that does not overlap `host`, `name` or `out`;
/// - when `host_len` is in `1..=253`, `host` points to `host_len` readable
///   bytes; out-of-range lengths are rejected before `host` is dereferenced;
/// - when `name_len` is in `1..=16`, `name` points to `name_len` readable
///   bytes; out-of-range lengths are rejected before `name` is dereferenced;
/// - when `capacity` is non-zero, `out` points to `capacity` writable bytes
///   that do not overlap any other argument;
/// - every pointer stays valid and non-overlapping for the whole call.
///
/// Arbitrary pointer values cannot be validated; anything that breaks the
/// above is undefined behaviour, not an error code.
#[no_mangle]
#[allow(clippy::too_many_arguments)] // The C header fixes the parameter list.
pub unsafe extern "C" fn craftrustnx_login_start(
    host: *const c_char,
    host_len: u32,
    port: u16,
    name: *const c_char,
    name_len: u32,
    out: *mut u8,
    capacity: u32,
    written: *mut u32,
) -> i32 {
    if written.is_null() {
        return LP_BAD_ARGUMENT;
    }
    // SAFETY: `written` is non-null and writable per the contract above.
    unsafe { *written = 0 };

    // Bound both lengths before forming any slice, like the status codec.
    if host_len == 0 || host_len > MAX_HOST as u32 || name_len == 0 || name_len > MAX_NAME as u32 {
        return LP_BAD_ARGUMENT;
    }
    if port == 0 {
        return LP_BAD_ARGUMENT;
    }
    if host.is_null() || name.is_null() || out.is_null() {
        return LP_BAD_ARGUMENT;
    }
    if capacity == 0 {
        return LP_OUTPUT_SMALL;
    }

    // SAFETY: the lengths are in range and the pointers are non-null, valid and
    // readable for exactly those lengths per the contract above.
    let host = unsafe { slice::from_raw_parts(host.cast::<u8>(), host_len as usize) };
    let name = unsafe { slice::from_raw_parts(name.cast::<u8>(), name_len as usize) };
    let out = unsafe { slice::from_raw_parts_mut(out, capacity as usize) };

    match encode_login_start(host, port, name, out) {
        Ok(len) => {
            // SAFETY: `written` is non-null and writable per the contract.
            unsafe { *written = len as u32 };
            LP_OK
        }
        Err(error) => status_of(error),
    }
}

/// Decode one complete clientbound login frame from `data` into `scratch` and
/// fill `event`.
///
/// `compression_threshold` is `-1` before the server announces compression
/// (raw frames) or the announced threshold (frames carry the envelope). Every
/// byte of `*event` is zeroed before anything else, so failures never leak
/// stale fields. Only this frame is consumed; a following frame is left alone.
/// `event.data_offset`/`data_length` describe the packet payload after the
/// packet ID inside `scratch`, which holds the whole body on success.
///
/// # Safety
///
/// The caller guarantees:
/// - when `length` is non-zero, `data` points to `length` readable bytes
///   (only `length == 0` with a null `data` is accepted, as `LP_NEED_MORE`);
/// - `scratch_capacity` is the exact size of the writable, aligned region at
///   `scratch`, which does not overlap `data` or `event`; a null `scratch` is
///   only allowed with `scratch_capacity == 0`;
/// - `event` points to one writable, aligned, `struct lp_event`-sized region
///   that does not overlap `data` or `scratch`;
/// - every pointer stays valid and non-overlapping for the whole call.
///
/// A `length` larger than the real buffer, or a pointer that does not meet the
/// above, is undefined behaviour rather than an error code.
#[no_mangle]
pub unsafe extern "C" fn craftrustnx_login_decode(
    data: *const u8,
    length: u32,
    compression_threshold: i32,
    scratch: *mut u8,
    scratch_capacity: u32,
    event: *mut LpEvent,
) -> i32 {
    if event.is_null() {
        return LP_BAD_ARGUMENT;
    }
    // SAFETY: `event` is non-null, writable and sized per the contract above.
    unsafe { ptr::write_bytes(event.cast::<u8>(), 0, core::mem::size_of::<LpEvent>()) };

    if compression_threshold < -1 {
        return LP_BAD_ARGUMENT;
    }
    if length == 0 {
        // An empty stream is incomplete, not a bad pointer.
        return LP_NEED_MORE;
    }
    if data.is_null() {
        return LP_BAD_ARGUMENT;
    }
    if scratch.is_null() && scratch_capacity != 0 {
        return LP_BAD_ARGUMENT;
    }

    // SAFETY: `data` is non-null and valid for `length` bytes per the contract.
    let data = unsafe { slice::from_raw_parts(data, length as usize) };
    let scratch: &mut [u8] = if scratch_capacity == 0 {
        &mut []
    } else {
        // SAFETY: `scratch` is non-null and valid for `scratch_capacity` bytes.
        unsafe { slice::from_raw_parts_mut(scratch, scratch_capacity as usize) }
    };

    match decode_login_frame(data, compression_threshold, scratch) {
        Ok(decoded) => {
            let event_value = event_of(decoded);
            // SAFETY: `event` is non-null, writable and sized per the contract,
            // and was already zeroed. Copy fields only: a typed struct store
            // may overwrite tail padding with uninitialized padding bytes.
            unsafe {
                ptr::addr_of_mut!((*event).kind).write(event_value.kind);
                ptr::addr_of_mut!((*event).consumed).write(event_value.consumed);
                ptr::addr_of_mut!((*event).data_offset).write(event_value.data_offset);
                ptr::addr_of_mut!((*event).data_length).write(event_value.data_length);
                ptr::addr_of_mut!((*event).threshold).write(event_value.threshold);
                ptr::addr_of_mut!((*event).uuid).write(event_value.uuid);
                ptr::addr_of_mut!((*event).name).write(event_value.name);
                ptr::addr_of_mut!((*event).session).write(event_value.session);
            }
            LP_OK
        }
        // `*event` is already zeroed, padding included.
        Err(error) => status_of(error),
    }
}

// A panic must not unwind across C or leave the probe's UI waiting: abort.
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    extern "C" {
        fn abort() -> !;
    }
    // SAFETY: `abort` terminates the process and never returns.
    unsafe { abort() }
}

// Precompiled `core` can still reference the unwind personality even with
// `panic = "abort"`, and a `no_std` staticlib has no std to provide it. This
// host-only shim keeps a plain C linker working; the Switch target's abort-built
// core (`target_os = "horizon"`) does not need or include it.
#[cfg(all(not(test), not(target_os = "horizon")))]
#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::md5::offline_uuid;
    use std::vec;
    use std::vec::Vec;

    const REQUEST: &[u8] = b"\x10\x00\x89\x06\x09localhost\x63\xdd\x02\
                             \x17\x00\x05Notch\xb5\x0a\xd3\x85\x82\x9d\x31\x41\xa2\x16\x7e\x7d\x75\x39\xba\x7f";
    const DISCONNECT: &[u8] = b"\x10\x00\x0e{\"text\":\"bye\"}";
    const NOTCH_UUID: [u8; 16] = [
        0xb5, 0x0a, 0xd3, 0x85, 0x82, 0x9d, 0x31, 0x41, 0xa2, 0x16, 0x7e, 0x7d, 0x75, 0x39, 0xba,
        0x7f,
    ];

    fn start_with(host: &[u8], port: u16, name: &[u8], out: &mut [u8]) -> (i32, u32) {
        let mut written = u32::MAX;
        let status = unsafe {
            craftrustnx_login_start(
                host.as_ptr().cast(),
                host.len() as u32,
                port,
                name.as_ptr().cast(),
                name.len() as u32,
                out.as_mut_ptr(),
                out.len() as u32,
                &mut written,
            )
        };
        (status, written)
    }

    fn decode_with(data: &[u8], threshold: i32, scratch: &mut [u8], event: &mut LpEvent) -> i32 {
        unsafe {
            craftrustnx_login_decode(
                data.as_ptr(),
                data.len() as u32,
                threshold,
                scratch.as_mut_ptr(),
                scratch.len() as u32,
                event,
            )
        }
    }

    fn event_bytes(event: &LpEvent) -> &[u8] {
        // SAFETY: the struct is plain data and the slice covers exactly it.
        unsafe {
            slice::from_raw_parts(
                (event as *const LpEvent).cast::<u8>(),
                core::mem::size_of::<LpEvent>(),
            )
        }
    }

    fn push_varint(out: &mut Vec<u8>, value: i32) {
        let mut bits = value as u32;
        loop {
            let byte = (bits & 0x7f) as u8;
            bits >>= 7;
            if bits == 0 {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
    }

    fn success_frame() -> Vec<u8> {
        let mut body = vec![2u8];
        body.extend_from_slice(&NOTCH_UUID);
        body.extend_from_slice(b"\x05Notch"); // name
        body.push(1); // one property
        body.extend_from_slice(b"\x08textures\x04e30=\x00"); // name, value, no signature
        body.extend_from_slice(&[0xaa; 16]); // session
        let mut wire = Vec::new();
        push_varint(&mut wire, body.len() as i32);
        wire.extend_from_slice(&body);
        wire
    }

    #[test]
    fn event_layout_matches_the_c_header() {
        assert_eq!(core::mem::size_of::<LpEvent>(), 72);
        assert_eq!(core::mem::offset_of!(LpEvent, kind), 0);
        assert_eq!(core::mem::offset_of!(LpEvent, consumed), 4);
        assert_eq!(core::mem::offset_of!(LpEvent, data_offset), 8);
        assert_eq!(core::mem::offset_of!(LpEvent, data_length), 12);
        assert_eq!(core::mem::offset_of!(LpEvent, threshold), 16);
        assert_eq!(core::mem::offset_of!(LpEvent, uuid), 20);
        assert_eq!(core::mem::offset_of!(LpEvent, name), 36);
        assert_eq!(core::mem::offset_of!(LpEvent, session), 53);
        assert_eq!(core::mem::size_of::<[c_char; 17]>(), 17);

        // Header enum values.
        assert_eq!(
            [
                LP_OK,
                LP_NEED_MORE,
                LP_BAD_ARGUMENT,
                LP_OUTPUT_SMALL,
                LP_BAD_FRAME,
                LP_BAD_PROFILE,
                LP_UNSUPPORTED
            ],
            [0, 1, -1, -2, -3, -4, -5]
        );
        assert_eq!(LP_MAX_BODY, 65_536);
        assert_eq!(LP_MAX_WIRE, 65_541);
        assert_eq!(
            [
                LP_LOGIN_SUCCESS,
                LP_DISCONNECT,
                LP_ENCRYPTION_REQUEST,
                LP_SET_COMPRESSION,
                LP_CUSTOM_QUERY,
                LP_COOKIE_REQUEST
            ],
            [1, 2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn start_through_the_abi_matches_the_wire_vector() {
        let mut out = [0xa5u8; 64];
        let (status, written) = start_with(b"localhost", 25565, b"Notch", &mut out);
        assert_eq!(status, LP_OK);
        assert_eq!(written, REQUEST.len() as u32);
        assert_eq!(&out[..written as usize], REQUEST);
        assert_eq!(out[written as usize], 0xa5);

        let mut out = [0u8; 64];
        assert_eq!(start_with(b"localhost", 25565, b"Notch", &mut out).1, 41);
        assert_eq!(offline_uuid(b"Notch"), NOTCH_UUID);
    }

    #[test]
    fn start_argument_and_capacity_errors_leave_output_untouched() {
        let mut out = [0xa5u8; 64];
        for (host, port, name) in [
            (&b""[..], 25565u16, &b"Notch"[..]),
            (b"bad host", 25565, b"Notch"),
            (b"localhost", 0, b"Notch"),
            (b"localhost", 25565, b""),
            (b"localhost", 25565, b"bad-name"),
            (b"localhost", 25565, b"a-name-is-way-too-long"),
        ] {
            assert_eq!(
                start_with(host, port, name, &mut out).0,
                LP_BAD_ARGUMENT,
                "{host:?}/{port}/{name:?}"
            );
            assert_eq!(out, [0xa5u8; 64]);
        }
        for capacity in 0..REQUEST.len() {
            let mut written = 7;
            let status = unsafe {
                craftrustnx_login_start(
                    b"localhost".as_ptr().cast(),
                    9,
                    25565,
                    b"Notch".as_ptr().cast(),
                    5,
                    out.as_mut_ptr(),
                    capacity as u32,
                    &mut written,
                )
            };
            assert_eq!(status, LP_OUTPUT_SMALL);
            assert_eq!(written, 0);
            assert_eq!(out, [0xa5u8; 64]);
        }
    }

    #[test]
    fn start_null_pointers_are_arguments_not_dereferences() {
        let mut out = [0u8; 64];
        let status = unsafe {
            craftrustnx_login_start(
                b"localhost".as_ptr().cast(),
                9,
                25565,
                b"Notch".as_ptr().cast(),
                5,
                out.as_mut_ptr(),
                64,
                ptr::null_mut(),
            )
        };
        assert_eq!(status, LP_BAD_ARGUMENT);

        for (host, name) in [
            (ptr::null(), b"Notch".as_ptr().cast::<c_char>()),
            (b"localhost".as_ptr().cast::<c_char>(), ptr::null()),
        ] {
            let mut written = 5;
            let status = unsafe {
                craftrustnx_login_start(host, 9, 25565, name, 5, out.as_mut_ptr(), 64, &mut written)
            };
            assert_eq!(status, LP_BAD_ARGUMENT);
            assert_eq!(written, 0);
        }

        // Oversized lengths are rejected before the (one-byte) host is read.
        let one = [b'a'; 1];
        let mut written = 3;
        let status = unsafe {
            craftrustnx_login_start(
                one.as_ptr().cast(),
                254,
                25565,
                b"a".as_ptr().cast(),
                1,
                out.as_mut_ptr(),
                64,
                &mut written,
            )
        };
        assert_eq!(status, LP_BAD_ARGUMENT);
        assert_eq!(written, 0);
        assert_eq!(out, [0u8; 64]);
    }

    #[test]
    fn decode_success_carries_profile_session_and_offsets() {
        let wire = success_frame();
        let mut scratch = [0u8; LP_MAX_BODY as usize];
        let mut event: LpEvent = unsafe { core::mem::zeroed() };
        assert_eq!(decode_with(&wire, -1, &mut scratch, &mut event), LP_OK);
        assert_eq!(event.kind, LP_LOGIN_SUCCESS);
        assert_eq!(event.consumed as usize, wire.len());
        assert_eq!((event.data_offset, event.data_length), (1, 54));
        assert_eq!(event.uuid, NOTCH_UUID);
        assert_eq!(event.session, [0xaa; 16]);
        assert_eq!(event.threshold, 0);
        let name: Vec<u8> = event.name.iter().map(|&c| c as u8).collect();
        assert_eq!(&name, b"Notch\0\0\0\0\0\0\0\0\0\0\0\0");

        // The payload region really is the packet body after the ID.
        let offset = event.data_offset as usize;
        let length = event.data_length as usize;
        assert_eq!(&scratch[offset..offset + length], &wire[2..]);
    }

    #[test]
    fn decode_events_for_disconnect_encryption_and_compression() {
        let mut scratch = [0u8; LP_MAX_BODY as usize];
        let mut event: LpEvent = unsafe { core::mem::zeroed() };

        assert_eq!(decode_with(DISCONNECT, -1, &mut scratch, &mut event), LP_OK);
        assert_eq!(event.kind, LP_DISCONNECT);
        assert_eq!(
            (event.consumed, event.data_offset, event.data_length),
            (17, 1, 15)
        );
        assert_eq!(&scratch[1..16], &b"\x0e{\"text\":\"bye\"}"[..]);

        // Hello always ends the attempt, whatever should-authenticate says.
        let hello = b"\x07\x01\x00\x01\x02\x01\x01\x01";
        assert_eq!(decode_with(hello, -1, &mut scratch, &mut event), LP_OK);
        assert_eq!(event.kind, LP_ENCRYPTION_REQUEST);

        let compression = b"\x03\x03\x80\x01";
        assert_eq!(
            decode_with(compression, -1, &mut scratch, &mut event),
            LP_OK
        );
        assert_eq!(event.kind, LP_SET_COMPRESSION);
        assert_eq!(event.threshold, 128);

        let query = b"\x07\x04\x03\x03a:b\x07";
        assert_eq!(decode_with(query, -1, &mut scratch, &mut event), LP_OK);
        assert_eq!(event.kind, LP_CUSTOM_QUERY);
        assert_eq!((event.data_offset, event.data_length), (1, 6));

        let cookie = b"\x05\x05\x03a:b";
        assert_eq!(decode_with(cookie, -1, &mut scratch, &mut event), LP_OK);
        assert_eq!(event.kind, LP_COOKIE_REQUEST);
    }

    #[test]
    fn every_failing_decode_leaves_every_event_byte_zero() {
        let mut scratch = [0u8; LP_MAX_BODY as usize];
        let mut event = LpEvent {
            kind: 0xffff_ffff,
            consumed: 0xffff_ffff,
            data_offset: 0xffff_ffff,
            data_length: 0xffff_ffff,
            threshold: -1,
            uuid: [0xff; 16],
            name: [0x7f; 17],
            session: [0xff; 16],
        };
        // Malformed frame, invalid UTF-8, unsupported id, bad profile, bad
        // threshold and a scratch that is too small.
        let bad_profile = {
            let mut body = vec![2u8];
            body.extend_from_slice(&NOTCH_UUID);
            body.push(17);
            body.extend_from_slice(&[b'a'; 17]); // one byte over the name cap
            body.push(0); // no properties
            body.extend_from_slice(&[0xaa; 16]);
            let mut wire = Vec::new();
            push_varint(&mut wire, body.len() as i32);
            wire.extend_from_slice(&body);
            wire
        };
        let cases: [(&[u8], i32, i32); 5] = [
            (b"\x03\x00\x05a", -1, LP_BAD_FRAME), // truncated reason string
            (b"\x03\x00\x01\xff", -1, LP_BAD_FRAME), // invalid UTF-8
            (b"\x01\x06", -1, LP_UNSUPPORTED),    // unmodelled packet id
            (&bad_profile, -1, LP_BAD_PROFILE),
            (&[0x01, 0x10], 16, LP_BAD_FRAME), // raw body at/over the threshold
        ];
        for (data, threshold, expected) in cases {
            assert_eq!(
                decode_with(data, threshold, &mut scratch, &mut event),
                expected,
                "{data:?}"
            );
            assert_eq!(event_bytes(&event), [0u8; 72], "{data:?}");
        }

        // A scratch smaller than the body.
        let mut small = [0u8; 8];
        assert_eq!(
            decode_with(DISCONNECT, -1, &mut small, &mut event),
            LP_OUTPUT_SMALL
        );
        assert_eq!(event_bytes(&event), [0u8; 72]);
        // A bad threshold and a null event.
        assert_eq!(
            decode_with(DISCONNECT, -2, &mut scratch, &mut event),
            LP_BAD_ARGUMENT
        );
        let status = unsafe {
            craftrustnx_login_decode(
                DISCONNECT.as_ptr(),
                DISCONNECT.len() as u32,
                -1,
                scratch.as_mut_ptr(),
                scratch.len() as u32,
                ptr::null_mut(),
            )
        };
        assert_eq!(status, LP_BAD_ARGUMENT);
    }

    #[test]
    fn decode_needs_more_and_consumes_only_one_frame() {
        let mut scratch = [0u8; LP_MAX_BODY as usize];
        let mut event: LpEvent = unsafe { core::mem::zeroed() };
        for end in 0..DISCONNECT.len() {
            assert_eq!(
                decode_with(&DISCONNECT[..end], -1, &mut scratch, &mut event),
                LP_NEED_MORE,
                "{end}"
            );
            assert_eq!(event_bytes(&event), [0u8; 72]);
        }

        // Empty input, and a null data pointer only when the length is zero.
        assert_eq!(decode_with(&[], -1, &mut scratch, &mut event), LP_NEED_MORE);
        let status = unsafe {
            craftrustnx_login_decode(
                ptr::null(),
                0,
                -1,
                scratch.as_mut_ptr(),
                scratch.len() as u32,
                &mut event,
            )
        };
        assert_eq!(status, LP_NEED_MORE);
        let status = unsafe {
            craftrustnx_login_decode(
                ptr::null(),
                4,
                -1,
                scratch.as_mut_ptr(),
                scratch.len() as u32,
                &mut event,
            )
        };
        assert_eq!(status, LP_BAD_ARGUMENT);

        // A null scratch with a non-zero capacity is an argument error.
        let status = unsafe {
            craftrustnx_login_decode(
                DISCONNECT.as_ptr(),
                DISCONNECT.len() as u32,
                -1,
                ptr::null_mut(),
                16,
                &mut event,
            )
        };
        assert_eq!(status, LP_BAD_ARGUMENT);
        assert_eq!(event_bytes(&event), [0u8; 72]);

        // One frame is consumed, a following frame stays in the buffer.
        let mut joined = DISCONNECT.to_vec();
        joined.extend_from_slice(&[0x01, 0x06]);
        assert_eq!(decode_with(&joined, -1, &mut scratch, &mut event), LP_OK);
        assert_eq!(event.consumed, 17);

        // A compressed frame round-trips through the ABI as well.
        let compressed = b"\x19\x10\x78\xda\x63\xe0\xab\x56\x2a\x49\xad\x28\x51\xb2\x52\x4a\xaa\x4c\x55\xaa\x05\x00\x25\x11\x04\xce";
        assert_eq!(decode_with(compressed, 16, &mut scratch, &mut event), LP_OK);
        assert_eq!(event.kind, LP_DISCONNECT);
        assert_eq!(event.consumed as usize, compressed.len());
    }

    #[test]
    fn scratch_capacity_zero_and_unknown_threshold_stay_bounded() {
        let mut event: LpEvent = unsafe { core::mem::zeroed() };
        let mut empty: [u8; 0] = [];
        assert_eq!(
            decode_with(DISCONNECT, -1, &mut empty, &mut event),
            LP_OUTPUT_SMALL
        );
        assert_eq!(event_bytes(&event), [0u8; 72]);
        // Zero capacity with a valid frame still needs the whole body in scratch.
        let status = unsafe {
            craftrustnx_login_decode(
                DISCONNECT.as_ptr(),
                DISCONNECT.len() as u32,
                -1,
                ptr::null_mut(),
                0,
                &mut event,
            )
        };
        assert_eq!(status, LP_OUTPUT_SMALL);
    }
}
