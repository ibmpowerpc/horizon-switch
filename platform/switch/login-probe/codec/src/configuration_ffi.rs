//! The exact C ABI of `platform/switch/login-probe/include/configuration_codec.h`.
//!
//! Every buffer is caller-owned; nothing here allocates, logs, opens a socket or
//! stores state, and no reply packet is ever sent. Pointer and length checks
//! happen before any slice is formed. A failure leaves the caller's outputs in
//! their documented state: every byte of `cf_event` (padding included) is zero,
//! and the frame function leaves `written` at 0 with `out` untouched.

use core::{ptr, slice};

use crate::configuration::{
    decode_configuration_frame, encode_client_frame, CfEvent, CF_BAD_ARGUMENT, CF_BAD_FRAME,
    CF_NEED_MORE, CF_OK, CF_OUTPUT_SMALL, CF_PHASE_PLAY, CF_UNSUPPORTED,
};
use crate::Error;
use crate::CONFIG_MAX_CLIENT_BODY;

fn status_of(error: Error) -> i32 {
    match error {
        Error::BadArgument => CF_BAD_ARGUMENT,
        Error::OutputSmall => CF_OUTPUT_SMALL,
        Error::NeedMore => CF_NEED_MORE,
        // A configuration frame never reports a profile problem; should the
        // login path's error surface here, it is still a frame error.
        Error::BadFrame | Error::BadProfile => CF_BAD_FRAME,
        Error::Unsupported => CF_UNSUPPORTED,
    }
}

/// Decode one complete clientbound frame from `data` into `scratch`, write any
/// reply body into `reply`, and fill `*event`.
///
/// `compression` is `-1` before/without compression or the announced threshold;
/// `phase` is `0` (configuration) or `1` (play). Every byte of `*event` is
/// zeroed first, so failures never leak stale fields. Only one frame is
/// consumed. In the configuration phase the reply is the unframed body of the
/// acknowledgement this diagnostic would send (finish, keepalive, pong, empty
/// known packs, cookie response without a payload, declined resource pack); in
/// the play phase there is none.
///
/// # Safety
///
/// The caller guarantees:
/// - when `length` is non-zero, `data` points to `length` readable bytes
///   (only `length == 0` with a null `data` is accepted, as `CF_NEED_MORE`);
/// - `scratch_capacity` is the exact size of the writable, aligned region at
///   `scratch`; a null `scratch` is only allowed with a zero capacity;
/// - `reply_capacity` is the exact size of the writable, aligned region at
///   `reply`; a null `reply` is only allowed with a zero capacity;
/// - `event` points to one writable, aligned, `struct cf_event`-sized region;
/// - none of the regions overlap, and every pointer stays valid for the call.
///
/// Arbitrary pointer values cannot be validated; anything that breaks the
/// above is undefined behaviour, not an error code.
#[no_mangle]
#[allow(clippy::too_many_arguments)] // The C header fixes the parameter list.
pub unsafe extern "C" fn craftrustnx_configuration_decode(
    data: *const u8,
    length: u32,
    compression: i32,
    phase: u32,
    scratch: *mut u8,
    scratch_capacity: u32,
    reply: *mut u8,
    reply_capacity: u32,
    event: *mut CfEvent,
) -> i32 {
    if event.is_null() {
        return CF_BAD_ARGUMENT;
    }
    // SAFETY: `event` is non-null, writable and sized per the contract above.
    unsafe { ptr::write_bytes(event.cast::<u8>(), 0, core::mem::size_of::<CfEvent>()) };

    if compression < -1 || phase > CF_PHASE_PLAY {
        return CF_BAD_ARGUMENT;
    }
    if length == 0 {
        // An empty stream is incomplete, not a bad pointer.
        return CF_NEED_MORE;
    }
    if data.is_null() {
        return CF_BAD_ARGUMENT;
    }
    if scratch.is_null() && scratch_capacity != 0 {
        return CF_BAD_ARGUMENT;
    }
    if reply.is_null() && reply_capacity != 0 {
        return CF_BAD_ARGUMENT;
    }

    // SAFETY: `data` is non-null and valid for `length` bytes per the contract.
    let data = unsafe { slice::from_raw_parts(data, length as usize) };
    let scratch: &mut [u8] = if scratch_capacity == 0 {
        &mut []
    } else {
        // SAFETY: `scratch` is non-null and valid for `scratch_capacity` bytes.
        unsafe { slice::from_raw_parts_mut(scratch, scratch_capacity as usize) }
    };
    let reply: &mut [u8] = if reply_capacity == 0 {
        &mut []
    } else {
        // SAFETY: `reply` is non-null and valid for `reply_capacity` bytes.
        unsafe { slice::from_raw_parts_mut(reply, reply_capacity as usize) }
    };

    match decode_configuration_frame(data, compression, phase, scratch, reply) {
        Ok(decoded) => {
            // SAFETY: `event` is non-null, writable and sized per the contract,
            // and was already zeroed. Fields are written individually so a
            // typed struct store cannot put padding bytes into C's struct.
            unsafe {
                ptr::addr_of_mut!((*event).kind).write(decoded.kind);
                ptr::addr_of_mut!((*event).consumed).write(decoded.consumed);
                ptr::addr_of_mut!((*event).body_length).write(decoded.body_length);
                ptr::addr_of_mut!((*event).packet_id).write(decoded.packet_id);
                ptr::addr_of_mut!((*event).payload_offset).write(decoded.payload_offset);
                ptr::addr_of_mut!((*event).payload_length).write(decoded.payload_length);
                ptr::addr_of_mut!((*event).reply_length).write(decoded.reply_length);
                ptr::addr_of_mut!((*event).flags).write(decoded.flags);
            }
            CF_OK
        }
        // `*event` is already zeroed, padding included.
        Err(error) => status_of(error),
    }
}

/// Frame one client body of 1..=1024 bytes for the configuration state.
///
/// `compression` is `-1` for a raw frame or the announced threshold. Below the
/// threshold the frame carries an uncompressed envelope; at or above it the
/// envelope carries a valid zlib stream (a stored DEFLATE block and an exact
/// Adler-32 trailer), which any inflater accepts. This covers thresholds 0 and
/// 1 for small client acknowledgements without a compressor dependency.
///
/// `*written` is set to 0 first, so every failure leaves it 0. On failure `out`
/// is never written: the complete frame length is checked against `capacity`
/// before anything is serialized. Ciphering the frame is the caller's job.
///
/// # Safety
///
/// The caller guarantees:
/// - `written` points to one writable, aligned `uint32_t` that does not overlap
///   `body` or `out`;
/// - when `length` is in `1..=1024`, `body` points to `length` readable bytes;
///   out-of-contract lengths are rejected before `body` is dereferenced;
/// - when `capacity` is non-zero, `out` points to `capacity` writable bytes that
///   do not overlap `body`;
/// - every pointer stays valid and non-overlapping for the whole call.
///
/// Arbitrary pointer values cannot be validated; anything that breaks the
/// above is undefined behaviour, not an error code.
#[no_mangle]
#[allow(clippy::too_many_arguments)] // The C header fixes the parameter list.
pub unsafe extern "C" fn craftrustnx_configuration_frame(
    body: *const u8,
    length: u32,
    compression: i32,
    out: *mut u8,
    capacity: u32,
    written: *mut u32,
) -> i32 {
    if written.is_null() {
        return CF_BAD_ARGUMENT;
    }
    // SAFETY: `written` is non-null and writable per the contract above.
    unsafe { *written = 0 };

    if length == 0 || length > CONFIG_MAX_CLIENT_BODY as u32 || compression < -1 {
        return CF_BAD_ARGUMENT;
    }
    if body.is_null() || out.is_null() {
        return CF_BAD_ARGUMENT;
    }
    if capacity == 0 {
        return CF_OUTPUT_SMALL;
    }

    // SAFETY: the length is in range and the pointers are non-null, valid and
    // readable/writable for exactly the given lengths per the contract above.
    let body = unsafe { slice::from_raw_parts(body, length as usize) };
    let out = unsafe { slice::from_raw_parts_mut(out, capacity as usize) };

    match encode_client_frame(body, compression, out) {
        Ok(len) => {
            // SAFETY: `written` is non-null and writable per the contract.
            unsafe { *written = len as u32 };
            CF_OK
        }
        Err(error) => status_of(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{CF_FLAG_PACK_REQUIRED, CF_PHASE_CONFIGURATION, CF_PHASE_PLAY};
    use crate::{CONFIG_MAX_BODY, CONFIG_MAX_FRAME, CONFIG_MAX_REPLY, CONFIG_MAX_WIRE};
    use std::vec;
    use std::vec::Vec;

    const UUID: [u8; 16] = [
        0xb5, 0x0a, 0xd3, 0x85, 0x82, 0x9d, 0x31, 0x41, 0xa2, 0x16, 0x7e, 0x7d, 0x75, 0x39, 0xba,
        0x7f,
    ];

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

    fn frame(body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        push_varint(&mut out, body.len() as i32);
        out.extend_from_slice(body);
        out
    }

    fn decode_with(
        data: &[u8],
        compression: i32,
        phase: u32,
        scratch: &mut [u8],
        reply: &mut [u8],
        event: &mut CfEvent,
    ) -> i32 {
        unsafe {
            craftrustnx_configuration_decode(
                data.as_ptr(),
                data.len() as u32,
                compression,
                phase,
                scratch.as_mut_ptr(),
                scratch.len() as u32,
                reply.as_mut_ptr(),
                reply.len() as u32,
                event,
            )
        }
    }

    fn frame_with(body: &[u8], compression: i32, out: &mut [u8]) -> (i32, u32) {
        let mut written = u32::MAX;
        let status = unsafe {
            craftrustnx_configuration_frame(
                body.as_ptr(),
                body.len() as u32,
                compression,
                out.as_mut_ptr(),
                out.len() as u32,
                &mut written,
            )
        };
        (status, written)
    }

    fn event_bytes(event: &CfEvent) -> &[u8] {
        // SAFETY: the struct is plain data and the slice covers exactly it.
        unsafe {
            slice::from_raw_parts(
                (event as *const CfEvent).cast::<u8>(),
                core::mem::size_of::<CfEvent>(),
            )
        }
    }

    #[test]
    fn event_and_constants_match_the_c_header() {
        assert_eq!(core::mem::size_of::<CfEvent>(), 32);
        assert_eq!(core::mem::offset_of!(CfEvent, kind), 0);
        assert_eq!(core::mem::offset_of!(CfEvent, consumed), 4);
        assert_eq!(core::mem::offset_of!(CfEvent, body_length), 8);
        assert_eq!(core::mem::offset_of!(CfEvent, packet_id), 12);
        assert_eq!(core::mem::offset_of!(CfEvent, payload_offset), 16);
        assert_eq!(core::mem::offset_of!(CfEvent, payload_length), 20);
        assert_eq!(core::mem::offset_of!(CfEvent, reply_length), 24);
        assert_eq!(core::mem::offset_of!(CfEvent, flags), 28);

        assert_eq!(
            [
                CF_OK,
                CF_NEED_MORE,
                CF_BAD_ARGUMENT,
                CF_OUTPUT_SMALL,
                CF_BAD_FRAME,
                CF_UNSUPPORTED
            ],
            [0, 1, -1, -2, -3, -5]
        );
        assert_eq!([CF_PHASE_CONFIGURATION, CF_PHASE_PLAY], [0, 1]);
        assert_eq!(
            [
                crate::CF_METADATA,
                crate::CF_FINISH,
                crate::CF_KEEP_ALIVE,
                crate::CF_PING,
                crate::CF_KNOWN_PACKS,
                crate::CF_COOKIE_REQUEST,
                crate::CF_RESOURCE_PACK,
                crate::CF_DISCONNECT,
                crate::CF_USER_ACTION,
                crate::CF_TRANSFER,
                crate::CF_PLAY_PACKET
            ],
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
        );
        assert_eq!(CONFIG_MAX_BODY, 8_388_608);
        assert_eq!(CONFIG_MAX_FRAME, 2_097_151);
        assert_eq!(CONFIG_MAX_WIRE, 2_097_156);
        assert_eq!(CONFIG_MAX_REPLY, 1024);
    }

    #[test]
    fn control_events_and_replies_use_vanilla_ids() {
        let mut scratch = vec![0u8; 65_536];
        let mut reply = [0u8; CONFIG_MAX_REPLY];
        let mut event: CfEvent = unsafe { core::mem::zeroed() };

        // FinishConfiguration (clientbound 3) -> serverbound 3, empty body.
        let wire = frame(&[3]);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_OK
        );
        assert_eq!(event.kind, crate::CF_FINISH);
        assert_eq!(
            (event.consumed, event.body_length, event.packet_id),
            (2, 1, 3)
        );
        assert_eq!((event.payload_offset, event.payload_length), (1, 0));
        assert_eq!(&reply[..event.reply_length as usize], &[3]);

        // KeepAlive (4) with a long, echoed back byte for byte.
        let mut body = vec![4u8];
        body.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let wire = frame(&body);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_OK
        );
        assert_eq!(event.kind, crate::CF_KEEP_ALIVE);
        assert_eq!(
            (event.consumed, event.body_length, event.packet_id),
            (10, 9, 4)
        );
        assert_eq!((event.payload_offset, event.payload_length), (1, 8));
        assert_eq!(&reply[..9], &body[..]);
        assert_eq!(&scratch[1..9], &[1, 2, 3, 4, 5, 6, 7, 8]);

        // Ping (5) with an int, answered by Pong (5) with the same bytes.
        let ping = [5u8, 0x11, 0x22, 0x33, 0x44];
        let wire = frame(&ping);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_OK
        );
        assert_eq!(event.kind, crate::CF_PING);
        assert_eq!((event.payload_offset, event.payload_length), (1, 4));
        assert_eq!(&reply[..5], &ping);

        // SelectKnownPacks (15) with no packs: the reply always claims none.
        let wire = frame(&[0x0f, 0]);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_OK
        );
        assert_eq!(event.kind, crate::CF_KNOWN_PACKS);
        assert_eq!(&reply[..2], &[7, 0]);
        assert_eq!(event.reply_length, 2);

        // The same reply for a non-empty offer: this probe has no packs.
        let mut body = vec![0x0fu8, 2];
        for _ in 0..2 {
            for text in [&b"minecraft"[..], &b"core"[..], &b"26.3"[..]] {
                push_varint(&mut body, text.len() as i32);
                body.extend_from_slice(text);
            }
        }
        let wire = frame(&body);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_OK
        );
        assert_eq!(event.kind, crate::CF_KNOWN_PACKS);
        assert_eq!(&reply[..2], &[7, 0]);

        // CookieRequest (0): CookieResponse (1) with the key and no payload.
        let cookie = b"\x00\x03a:b";
        let wire = frame(cookie);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_OK
        );
        assert_eq!(event.kind, crate::CF_COOKIE_REQUEST);
        assert_eq!(&reply[..6], b"\x01\x03a:b\x00");

        // CodeOfConduct (20): reported, never accepted.
        let conduct = b"\x14\x02hi";
        let wire = frame(conduct);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_OK
        );
        assert_eq!(event.kind, crate::CF_USER_ACTION);
        assert_eq!(event.reply_length, 0);

        // Transfer (12): host and port reported, nothing followed.
        let mut body = vec![0x0cu8];
        push_varint(&mut body, 4);
        body.extend_from_slice(b"host");
        push_varint(&mut body, 25565);
        let wire = frame(&body);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_OK
        );
        assert_eq!(event.kind, crate::CF_TRANSFER);
        assert_eq!(event.reply_length, 0);

        // Disconnect (2): raw NBT bytes, opaque and unclaimed.
        let disconnect = [2u8, 8, 0, 4, b't', b'e', b'x', b't', 0, 3, b'b', b'y', b'e'];
        let wire = frame(&disconnect);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_OK
        );
        assert_eq!(event.kind, crate::CF_DISCONNECT);
        assert_eq!(event.reply_length, 0);
        assert_eq!(&scratch[1..disconnect.len()], &disconnect[1..]);
    }

    #[test]
    fn resource_pack_is_declined_and_required_bit_reported() {
        let mut scratch = vec![0u8; 65_536];
        let mut reply = [0u8; CONFIG_MAX_REPLY];
        let mut event: CfEvent = unsafe { core::mem::zeroed() };

        for (required, flags) in [(true, CF_FLAG_PACK_REQUIRED), (false, 0)] {
            let mut body = vec![9u8];
            body.extend_from_slice(&UUID);
            push_varint(&mut body, 4);
            body.extend_from_slice(b"http");
            push_varint(&mut body, 8);
            body.extend_from_slice(b"abc12345");
            body.push(u8::from(required));
            body.push(0); // No prompt NBT.
            let wire = frame(&body);
            assert_eq!(
                decode_with(
                    &wire,
                    -1,
                    CF_PHASE_CONFIGURATION,
                    &mut scratch,
                    &mut reply,
                    &mut event
                ),
                CF_OK
            );
            assert_eq!(event.kind, crate::CF_RESOURCE_PACK);
            assert_eq!(event.flags, flags);
            assert_eq!(event.reply_length, 18);
            assert_eq!(reply[0], 6); // Serverbound ResourcePack.
            assert_eq!(&reply[1..17], &UUID);
            assert_eq!(reply[17], 1); // Declined: never downloaded or applied.

            // A prompt may stay opaque, but it must carry bytes.
            let mut with_prompt = body.clone();
            let last = with_prompt.len() - 1;
            with_prompt[last] = 1;
            with_prompt.extend_from_slice(&[0x08, 0x00]);
            let wire = frame(&with_prompt);
            assert_eq!(
                decode_with(
                    &wire,
                    -1,
                    CF_PHASE_CONFIGURATION,
                    &mut scratch,
                    &mut reply,
                    &mut event
                ),
                CF_OK
            );
            assert_eq!(event.kind, crate::CF_RESOURCE_PACK);
            assert_eq!(event.reply_length, 18);
            assert_eq!(reply[17], 1);

            let mut empty_prompt = body.clone();
            let last = empty_prompt.len() - 1;
            empty_prompt[last] = 1;
            let wire = frame(&empty_prompt);
            assert_eq!(
                decode_with(
                    &wire,
                    -1,
                    CF_PHASE_CONFIGURATION,
                    &mut scratch,
                    &mut reply,
                    &mut event
                ),
                CF_BAD_FRAME
            );

            // A 41-unit hash is over Vanilla's StrN<40>; 40 is accepted.
            for (hash_len, expected) in [(41usize, CF_BAD_FRAME), (40, CF_OK)] {
                let mut body = vec![9u8];
                body.extend_from_slice(&UUID);
                push_varint(&mut body, 4);
                body.extend_from_slice(b"http");
                push_varint(&mut body, hash_len as i32);
                body.extend_from_slice(&vec![b'h'; hash_len]);
                body.push(1);
                body.push(0);
                let wire = frame(&body);
                assert_eq!(
                    decode_with(
                        &wire,
                        -1,
                        CF_PHASE_CONFIGURATION,
                        &mut scratch,
                        &mut reply,
                        &mut event
                    ),
                    expected,
                    "{hash_len}"
                );
            }
        }
    }

    #[test]
    fn known_packs_are_fully_validated_but_never_claimed() {
        let mut scratch = vec![0u8; 65_536];
        let mut reply = [0u8; CONFIG_MAX_REPLY];
        let mut event: CfEvent = unsafe { core::mem::zeroed() };

        // 256 packs is the accepted maximum.
        let mut body = vec![0x0fu8];
        push_varint(&mut body, 256);
        for _ in 0..256 {
            for text in [&b"a"[..], &b"b"[..], &b"c"[..]] {
                push_varint(&mut body, text.len() as i32);
                body.extend_from_slice(text);
            }
        }
        let wire = frame(&body);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_OK
        );
        assert_eq!(&reply[..2], &[7, 0]);

        // 257 packs is over the diagnostic cap.
        let mut body = vec![0x0fu8];
        push_varint(&mut body, 257);
        let wire = frame(&body);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_BAD_FRAME
        );

        // A tuple string over 32767 UTF-16 units is a frame error.
        let mut body = vec![0x0fu8, 1];
        push_varint(&mut body, 32_768);
        body.extend_from_slice(&vec![b'a'; 32_768]);
        for _ in 0..2 {
            push_varint(&mut body, 1);
            body.push(b'a');
        }
        let wire = frame(&body);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_BAD_FRAME
        );

        // A truncated tuple, and any trailing byte, are frame errors.
        let truncated = [0x0fu8, 1, 1, b'a', 1];
        let wire = frame(&truncated);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_BAD_FRAME
        );
        let trailing = [0x0fu8, 0, 0];
        let wire = frame(&trailing);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_BAD_FRAME
        );
    }

    #[test]
    fn metadata_payloads_stay_opaque_and_unparsed() {
        let mut scratch = vec![0u8; 65_536];
        let mut reply = [0xa5u8; CONFIG_MAX_REPLY];
        let mut event: CfEvent = unsafe { core::mem::zeroed() };

        // RegistryData (7), tags (14), feature flags (13) and the rest: no
        // field is parsed, including bytes no NBT reader would accept.
        for id in [1u8, 6, 7, 8, 10, 11, 13, 14, 16, 17, 18, 19] {
            let payload = [0xff, 0x00, 0x80, 0xfe];
            let mut body = vec![id];
            body.extend_from_slice(&payload);
            let wire = frame(&body);
            assert_eq!(
                decode_with(
                    &wire,
                    -1,
                    CF_PHASE_CONFIGURATION,
                    &mut scratch,
                    &mut reply,
                    &mut event
                ),
                CF_OK,
                "{id}"
            );
            assert_eq!(event.kind, crate::CF_METADATA);
            assert_eq!(event.packet_id, i32::from(id));
            assert_eq!(event.body_length, body.len() as u32);
            assert_eq!(
                (event.payload_offset, event.payload_length),
                (1, payload.len() as u32)
            );
            assert_eq!(event.reply_length, 0);
            assert_eq!(&scratch[1..5], &payload);
            // The payload was copied, not interpreted or installed anywhere.
            assert_eq!(&reply, &[0xa5u8; CONFIG_MAX_REPLY]);
        }

        // An empty payload is opaque too.
        let wire = frame(&[7]);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_OK
        );
        assert_eq!((event.payload_offset, event.payload_length), (1, 0));
    }

    #[test]
    fn play_phase_reports_the_packet_id_and_opaque_body() {
        let mut scratch = vec![0u8; 65_536];
        let mut reply = [0xa5u8; CONFIG_MAX_REPLY];
        let mut event: CfEvent = unsafe { core::mem::zeroed() };

        let body = [0x2au8, 0x01, 0x02, 0x03];
        let wire = frame(&body);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_PLAY,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_OK
        );
        assert_eq!(event.kind, crate::CF_PLAY_PACKET);
        assert_eq!(event.packet_id, 42);
        assert_eq!((event.consumed, event.body_length), (5, 4));
        assert_eq!((event.payload_offset, event.payload_length), (1, 3));
        assert_eq!(event.reply_length, 0);
        assert_eq!(&scratch[1..4], &[1, 2, 3]);
        assert_eq!(&reply, &[0xa5u8; CONFIG_MAX_REPLY]);

        // The same bytes in the configuration phase are an unknown packet.
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_UNSUPPORTED
        );

        // A negative play packet ID is malformed.
        let wire = frame(&[0xff, 0xff, 0xff, 0xff, 0x0f]);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_PLAY,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_BAD_FRAME
        );
        // An unknown phase is an argument error.
        assert_eq!(
            decode_with(&frame(&[3]), -1, 2, &mut scratch, &mut reply, &mut event),
            CF_BAD_ARGUMENT
        );
    }

    #[test]
    fn unknown_configuration_ids_are_unsupported() {
        let mut scratch = vec![0u8; 65_536];
        let mut reply = [0u8; CONFIG_MAX_REPLY];
        let mut event: CfEvent = unsafe { core::mem::zeroed() };

        for id in [21u8, 22, 127] {
            let wire = frame(&[id]);
            assert_eq!(
                decode_with(
                    &wire,
                    -1,
                    CF_PHASE_CONFIGURATION,
                    &mut scratch,
                    &mut reply,
                    &mut event
                ),
                CF_UNSUPPORTED,
                "{id}"
            );
        }
        // A negative VarInt ID is a valid VarInt but not a configuration packet.
        let wire = frame(&[0xff, 0xff, 0xff, 0xff, 0x0f]);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_UNSUPPORTED
        );
        // A malformed ID VarInt is a frame error.
        let wire = frame(&[0x80, 0x80, 0x80, 0x80, 0x80]);
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_BAD_FRAME
        );
    }

    #[test]
    fn compressed_envelopes_coalesce_and_partial_frames_need_more() {
        let mut scratch = vec![0u8; 65_536];
        let mut reply = [0u8; CONFIG_MAX_REPLY];
        let mut event: CfEvent = unsafe { core::mem::zeroed() };

        // A Python-zlib stream for the KeepAlive body (id 4 and eight bytes).
        let body = [4u8, 1, 2, 3, 4, 5, 6, 7, 8];
        let zlib: &[u8] = b"\x78\xda\x63\x61\x64\x62\x66\x61\x65\x63\xe7\x00\x00\x00\xa5\x00\x29";
        let mut inner = vec![body.len() as u8];
        inner.extend_from_slice(zlib);
        let compressed = frame(&inner);

        let mut joined = compressed.clone();
        joined.extend_from_slice(&compressed);
        assert_eq!(
            decode_with(
                &joined,
                9,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_OK
        );
        assert_eq!(event.kind, crate::CF_KEEP_ALIVE);
        assert_eq!(event.consumed as usize, compressed.len());
        // The second frame is intact for the caller's next call.
        assert_eq!(&joined[compressed.len()..], &compressed[..]);
        assert_eq!(
            decode_with(
                &joined[compressed.len()..],
                9,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_OK
        );

        // Should the zlib vector be wrong, inflate would refuse it; assert the
        // ABI reports the body it validated.
        assert_eq!(event.body_length, body.len() as u32);

        // A corrupted Adler-32 trailer and a negative declared length are frame
        // errors, not incomplete frames.
        let mut corrupt = compressed.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0xff;
        assert_eq!(
            decode_with(
                &corrupt,
                9,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_BAD_FRAME
        );
        let mut negative = Vec::new();
        push_varint(&mut negative, -1);
        negative.push(0);
        let wire = frame(&negative);
        assert_eq!(
            decode_with(
                &wire,
                9,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_BAD_FRAME
        );

        // Every proper prefix of the outer frame is incomplete.
        for end in 0..compressed.len() {
            assert_eq!(
                decode_with(
                    &compressed[..end],
                    9,
                    CF_PHASE_CONFIGURATION,
                    &mut scratch,
                    &mut reply,
                    &mut event
                ),
                CF_NEED_MORE,
                "{end}"
            );
        }
        assert_eq!(
            decode_with(
                &[],
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_NEED_MORE
        );
        // A five-byte, still-open length prefix is incomplete.
        assert_eq!(
            decode_with(
                &[0x80; 4],
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_NEED_MORE
        );
        // Five continuation bytes are an impossible prefix.
        assert_eq!(
            decode_with(
                &[0x80; 5],
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_BAD_FRAME
        );
    }

    #[test]
    fn outer_prefix_and_inflated_caps_reject_before_buffering_or_inflating() {
        // An outer prefix over CF_MAX_FRAME is rejected from the prefix alone,
        // even though the frame body has not arrived: no buffering wait.
        let mut oversize = Vec::new();
        push_varint(&mut oversize, CONFIG_MAX_FRAME as i32 + 1);
        oversize.push(0);
        let mut scratch = vec![0u8; 65_536];
        let mut reply = [0u8; CONFIG_MAX_REPLY];
        let mut event: CfEvent = unsafe { core::mem::zeroed() };
        assert_eq!(
            decode_with(
                &oversize,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_BAD_FRAME
        );
        // At the cap the frame is possible, so it is incomplete, not bad.
        let mut at_cap = Vec::new();
        push_varint(&mut at_cap, CONFIG_MAX_FRAME as i32);
        assert_eq!(
            decode_with(
                &at_cap,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_NEED_MORE
        );

        // An envelope declaring one byte over CF_MAX_BODY is rejected from the
        // envelope, before the (large enough) scratch is used or anything is
        // inflated. These are the only tests that allocate the 8 MiB body.
        let mut scratch = vec![0u8; CONFIG_MAX_BODY];
        let mut over = Vec::new();
        push_varint(&mut over, CONFIG_MAX_BODY as i32 + 1);
        over.push(0);
        let wire = frame(&over);
        assert_eq!(
            decode_with(
                &wire,
                16,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_BAD_FRAME
        );

        // At the cap the body fits, so a short scratch is reported before any
        // inflation work.
        let mut at_limit = Vec::new();
        push_varint(&mut at_limit, CONFIG_MAX_BODY as i32);
        at_limit.push(0);
        let wire = frame(&at_limit);
        let mut short = vec![0u8; CONFIG_MAX_BODY - 1];
        assert_eq!(
            decode_with(
                &wire,
                16,
                CF_PHASE_CONFIGURATION,
                &mut short,
                &mut reply,
                &mut event
            ),
            CF_OUTPUT_SMALL
        );
        // With the exact scratch it reaches the inflater, which rejects the
        // one-byte non-stream: the cap itself was accepted.
        assert_eq!(
            decode_with(
                &wire,
                16,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_BAD_FRAME
        );
        // A declared length below the announced threshold is not Vanilla.
        assert_eq!(
            decode_with(
                &frame(&[0x01, 0, 0]),
                16,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_BAD_FRAME
        );
    }

    #[test]
    fn malformed_control_bodies_are_frame_errors() {
        let mut scratch = vec![0u8; 65_536];
        let mut reply = [0u8; CONFIG_MAX_REPLY];
        let mut event: CfEvent = unsafe { core::mem::zeroed() };

        let cases: [&[u8]; 8] = [
            b"\x04\x01\x02\x03",                         // KeepAlive: 3 bytes, not 8
            b"\x04\x01\x02\x03\x04\x05\x06\x07\x08\x00", // KeepAlive: trailing byte
            b"\x05\x01\x02\x03",                         // Ping: 3 bytes, not 4
            b"\x00\x03A:b",                              // Cookie key: bad identifier
            b"\x00\x03a:b\x00",                          // Cookie: trailing byte
            b"\x02",                                     // Disconnect: no NBT bytes
            b"\x14\x02hi\x00",                           // Code of conduct: trailing byte
            b"\x03\x00",                                 // Finish: trailing byte
        ];
        for body in cases {
            let wire = frame(body);
            assert_eq!(
                decode_with(
                    &wire,
                    -1,
                    CF_PHASE_CONFIGURATION,
                    &mut scratch,
                    &mut reply,
                    &mut event
                ),
                CF_BAD_FRAME,
                "{body:?}"
            );
            assert_eq!(event_bytes(&event), [0u8; 32], "{body:?}");
        }
    }

    #[test]
    fn reply_capacity_and_cap_are_checked_before_serialization() {
        let mut scratch = vec![0u8; 65_536];
        let mut event: CfEvent = unsafe { core::mem::zeroed() };

        // A cookie key long enough to exceed CF_REPLY_BODY cannot be answered.
        let mut body = vec![0u8];
        push_varint(&mut body, 1021);
        body.extend_from_slice(&vec![b'a'; 1021]);
        let wire = frame(&body);
        let mut reply = [0xa5u8; CONFIG_MAX_REPLY];
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_BAD_FRAME
        );
        assert_eq!(reply, [0xa5u8; CONFIG_MAX_REPLY]);

        // 1020 leaves room for the header, the key and the absent payload.
        let mut body = vec![0u8];
        push_varint(&mut body, 1020);
        body.extend_from_slice(&vec![b'a'; 1020]);
        let wire = frame(&body);
        let mut reply = [0xa5u8; CONFIG_MAX_REPLY];
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_OK
        );
        assert_eq!(event.reply_length, 1024);
        assert_eq!(reply[0], 1);
        assert_eq!(reply[1023], 0);

        // One byte short in the caller's reply buffer is OUTPUT_SMALL, and the
        // reply is untouched.
        let cookie = b"\x00\x03a:b";
        let wire = frame(cookie);
        let mut reply = [0xa5u8; 5];
        assert_eq!(
            decode_with(
                &wire,
                -1,
                CF_PHASE_CONFIGURATION,
                &mut scratch,
                &mut reply,
                &mut event
            ),
            CF_OUTPUT_SMALL
        );
        assert_eq!(reply, [0xa5u8; 5]);
        assert_eq!(event_bytes(&event), [0u8; 32]);

        // No reply buffer at all is fine for a packet with no reply.
        let wire = frame(&[0x0f, 0]);
        let status = unsafe {
            craftrustnx_configuration_decode(
                wire.as_ptr(),
                wire.len() as u32,
                -1,
                CF_PHASE_CONFIGURATION,
                scratch.as_mut_ptr(),
                scratch.len() as u32,
                ptr::null_mut(),
                0,
                &mut event,
            )
        };
        assert_eq!(status, CF_OUTPUT_SMALL);
        let wire = frame(&[7, 0xff]);
        let status = unsafe {
            craftrustnx_configuration_decode(
                wire.as_ptr(),
                wire.len() as u32,
                -1,
                CF_PHASE_CONFIGURATION,
                scratch.as_mut_ptr(),
                scratch.len() as u32,
                ptr::null_mut(),
                0,
                &mut event,
            )
        };
        assert_eq!(status, CF_OK);
        assert_eq!(event.reply_length, 0);
    }

    #[test]
    fn every_failing_decode_leaves_every_event_byte_zero() {
        let mut scratch = vec![0u8; 65_536];
        let mut reply = [0xa5u8; CONFIG_MAX_REPLY];
        let mut event = CfEvent {
            kind: 0xffff_ffff,
            consumed: 0xffff_ffff,
            body_length: 0xffff_ffff,
            packet_id: -1,
            payload_offset: 0xffff_ffff,
            payload_length: 0xffff_ffff,
            reply_length: 0xffff_ffff,
            flags: 0xffff_ffff,
        };
        let wire = frame(&[0x0f, 0]);
        let cases: [(&[u8], i32, u32, i32); 5] = [
            (&wire, -2, CF_PHASE_CONFIGURATION, CF_BAD_ARGUMENT),
            (&wire, -1, 2, CF_BAD_ARGUMENT),
            (b"\x01\x15", -1, CF_PHASE_CONFIGURATION, CF_UNSUPPORTED),
            (b"\x02\x04\x01", -1, CF_PHASE_CONFIGURATION, CF_BAD_FRAME),
            (&[], -1, CF_PHASE_CONFIGURATION, CF_NEED_MORE),
        ];
        for (data, compression, phase, expected) in cases {
            assert_eq!(
                decode_with(
                    data,
                    compression,
                    phase,
                    &mut scratch,
                    &mut reply,
                    &mut event
                ),
                expected,
                "{data:?}"
            );
            assert_eq!(event_bytes(&event), [0u8; 32], "{data:?}");
        }

        // A null event is an argument error, never a dereference.
        let status = unsafe {
            craftrustnx_configuration_decode(
                wire.as_ptr(),
                wire.len() as u32,
                -1,
                CF_PHASE_CONFIGURATION,
                scratch.as_mut_ptr(),
                scratch.len() as u32,
                reply.as_mut_ptr(),
                reply.len() as u32,
                ptr::null_mut(),
            )
        };
        assert_eq!(status, CF_BAD_ARGUMENT);

        // Null data only with a zero length; null scratch/reply only with zero
        // capacity.
        let status = unsafe {
            craftrustnx_configuration_decode(
                ptr::null(),
                0,
                -1,
                CF_PHASE_CONFIGURATION,
                scratch.as_mut_ptr(),
                scratch.len() as u32,
                reply.as_mut_ptr(),
                reply.len() as u32,
                &mut event,
            )
        };
        assert_eq!(status, CF_NEED_MORE);
        let status = unsafe {
            craftrustnx_configuration_decode(
                ptr::null(),
                5,
                -1,
                CF_PHASE_CONFIGURATION,
                scratch.as_mut_ptr(),
                scratch.len() as u32,
                reply.as_mut_ptr(),
                reply.len() as u32,
                &mut event,
            )
        };
        assert_eq!(status, CF_BAD_ARGUMENT);
        let status = unsafe {
            craftrustnx_configuration_decode(
                wire.as_ptr(),
                wire.len() as u32,
                -1,
                CF_PHASE_CONFIGURATION,
                ptr::null_mut(),
                16,
                reply.as_mut_ptr(),
                reply.len() as u32,
                &mut event,
            )
        };
        assert_eq!(status, CF_BAD_ARGUMENT);
        let status = unsafe {
            craftrustnx_configuration_decode(
                wire.as_ptr(),
                wire.len() as u32,
                -1,
                CF_PHASE_CONFIGURATION,
                scratch.as_mut_ptr(),
                scratch.len() as u32,
                ptr::null_mut(),
                8,
                &mut event,
            )
        };
        assert_eq!(status, CF_BAD_ARGUMENT);
        assert_eq!(event_bytes(&event), [0u8; 32]);
    }

    #[test]
    fn frame_vectors_match_python_zlib() {
        // Python's zlib.decompress accepts these exact bytes: a stored DEFLATE
        // block inside a zlib wrapper with its Adler-32 trailer.
        let body: &[u8] = b"\x05\x03a:b";

        // compression -1: one length prefix, then the body without an envelope.
        let mut out = [0xa5u8; 64];
        let (status, written) = frame_with(body, -1, &mut out);
        assert_eq!((status, written), (CF_OK, 6));
        assert_eq!(&out[..6], b"\x05\x05\x03a:b");
        assert_eq!(out[6], 0xa5);

        // threshold 0 and 1: compressed envelope. Expected bytes were produced
        // and decompressed independently with Python zlib.
        let expected: &[u8] = b"\x11\x05\x78\x01\x01\x05\x00\xfa\xff\x05\x03a:b\x02\x23\x01\x06";
        for threshold in [0, 1] {
            let mut out = [0xa5u8; 64];
            let (status, written) = frame_with(body, threshold, &mut out);
            assert_eq!(
                (status, written),
                (CF_OK, expected.len() as u32),
                "{threshold}"
            );
            assert_eq!(&out[..expected.len()], expected, "{threshold}");
            assert_eq!(out[expected.len()], 0xa5);
        }

        // threshold 6: the body is below it, so an uncompressed envelope.
        let mut out = [0xa5u8; 64];
        let (status, written) = frame_with(body, 6, &mut out);
        assert_eq!((status, written), (CF_OK, 7));
        assert_eq!(&out[..7], b"\x06\x00\x05\x03a:b");

        // A one-byte client acknowledgement at threshold 0 and 1.
        let expected: &[u8] = b"\x0d\x01\x78\x01\x01\x01\x00\xfe\xff\x05\x00\x06\x00\x06";
        for threshold in [0, 1] {
            let mut out = [0xa5u8; 64];
            let (status, written) = frame_with(b"\x05", threshold, &mut out);
            assert_eq!((status, written), (CF_OK, expected.len() as u32));
            assert_eq!(&out[..expected.len()], expected, "{threshold}");
        }
        // threshold 2: below it, so a raw envelope.
        let mut out = [0xa5u8; 64];
        let (status, written) = frame_with(b"\x05", 2, &mut out);
        assert_eq!((status, written), (CF_OK, 3));
        assert_eq!(&out[..3], b"\x02\x00\x05");

        // A zero threshold compresses every accepted body; the 1024-byte cap is
        // the largest one.
        let body = vec![0x5au8; 1024];
        let mut out = vec![0xa5u8; 1200];
        let (status, written) = frame_with(&body, 0, &mut out);
        assert_eq!(status, CF_OK);
        // Outer prefix (2) + declared length (2) + zlib (1035).
        assert_eq!(written as usize, 2 + 2 + 1024 + 11);
        assert_eq!(&out[..2], &[0x8d, 0x08]); // 1037
        assert_eq!(&out[2..4], &[0x80, 0x08]); // 1024
        assert_eq!(&out[4..6], &[0x78, 0x01]); // zlib header
        assert_eq!(out[6], 1); // final stored block
        assert_eq!(u16::from_le_bytes([out[7], out[8]]), 1024);
        assert_eq!(u16::from_le_bytes([out[9], out[10]]), !1024u16);
        assert_eq!(&out[11..11 + 1024], &body[..]);
        // Adler-32 of 1024 bytes of 0x5a, computed independently.
        assert_eq!(&out[1035..1039], &0xe230_6810u32.to_be_bytes());
        assert_eq!(out[written as usize], 0xa5);
    }

    #[test]
    fn frame_rejects_out_of_contract_arguments_and_short_output() {
        let mut out = [0xa5u8; 1200];
        for (length, compression) in [(0u32, -1i32), (1025, -1), (5, -2), (5, i32::MIN)] {
            let mut written = 7;
            let status = unsafe {
                craftrustnx_configuration_frame(
                    out.as_ptr(),
                    length,
                    compression,
                    out.as_mut_ptr(),
                    out.len() as u32,
                    &mut written,
                )
            };
            assert_eq!(
                (status, written),
                (CF_BAD_ARGUMENT, 0),
                "{length}/{compression}"
            );
            assert_eq!(out, [0xa5u8; 1200]);
        }

        // Null pointers, and a null `written`, are arguments not dereferences.
        let body = b"\x05";
        let status = unsafe {
            craftrustnx_configuration_frame(
                body.as_ptr(),
                1,
                -1,
                out.as_mut_ptr(),
                out.len() as u32,
                ptr::null_mut(),
            )
        };
        assert_eq!(status, CF_BAD_ARGUMENT);
        for (body_ptr, out_ptr) in [
            (ptr::null(), out.as_mut_ptr()),
            (body.as_ptr(), ptr::null_mut()),
        ] {
            let mut written = 7;
            let status = unsafe {
                craftrustnx_configuration_frame(
                    body_ptr,
                    1,
                    -1,
                    out_ptr,
                    out.len() as u32,
                    &mut written,
                )
            };
            assert_eq!(status, CF_BAD_ARGUMENT);
            assert_eq!(written, 0);
        }
        assert_eq!(out, [0xa5u8; 1200]);

        // Every capacity below the compressed frame is short and untouched;
        // the exact length is enough.
        let total = 14;
        for capacity in 0..total {
            let mut out = [0xa5u8; 32];
            let (status, written) = frame_with(b"\x05", 0, &mut out[..capacity]);
            assert_eq!((status, written), (CF_OUTPUT_SMALL, 0), "{capacity}");
            assert_eq!(out, [0xa5u8; 32]);
        }
        let mut out = [0xa5u8; 32];
        let (status, written) = frame_with(b"\x05", 0, &mut out[..total]);
        assert_eq!((status, written as usize), (CF_OK, total));

        // A zero-length output region reports a short buffer.
        let mut written = 9;
        let status = unsafe {
            craftrustnx_configuration_frame(body.as_ptr(), 1, -1, out.as_mut_ptr(), 0, &mut written)
        };
        assert_eq!(status, CF_OUTPUT_SMALL);
        assert_eq!(written, 0);
    }

    #[test]
    fn malformed_input_never_panics() {
        let mut scratch = vec![0u8; 4096];
        let mut reply = [0u8; CONFIG_MAX_REPLY];
        let mut event: CfEvent = unsafe { core::mem::zeroed() };

        for compression in [-1, 0, 16] {
            for a in 0..=255u8 {
                for b in 0..=255u8 {
                    let _ = decode_with(
                        &[a, b],
                        compression,
                        CF_PHASE_CONFIGURATION,
                        &mut scratch,
                        &mut reply,
                        &mut event,
                    );
                    let _ = decode_with(
                        &[a, b],
                        compression,
                        CF_PHASE_PLAY,
                        &mut scratch,
                        &mut reply,
                        &mut event,
                    );
                }
            }
        }

        // Bit flips across a valid KeepAlive frame and one compressed frame.
        let wires = [
            frame(&[4, 1, 2, 3, 4, 5, 6, 7, 8]),
            b"\x12\x09\x78\xda\x63\x61\x64\x62\x66\x61\x65\x63\xe7\x00\x00\x00\xa5\x00\x29"
                .to_vec(),
        ];
        for wire in wires {
            for i in 0..wire.len() {
                for bit in 0..8 {
                    let mut bad = wire.clone();
                    bad[i] ^= 1 << bit;
                    for compression in [-1, 0, 16] {
                        for phase in [CF_PHASE_CONFIGURATION, CF_PHASE_PLAY] {
                            let _ = decode_with(
                                &bad,
                                compression,
                                phase,
                                &mut scratch,
                                &mut reply,
                                &mut event,
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn frame_function_never_panics_on_arbitrary_bodies() {
        let mut out = [0u8; 2048];
        let mut state = 0x1234_5678u32;
        for _ in 0..2_000 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let len = (state as usize % 1024) + 1;
            let mut body = vec![0u8; len];
            for byte in body.iter_mut() {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                *byte = state as u8;
            }
            for compression in [-1, 0, 1, 64, i32::MAX] {
                let _ = frame_with(&body, compression, &mut out);
            }
        }
    }
}
