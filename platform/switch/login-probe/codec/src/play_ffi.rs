//! The exact C ABI of `platform/switch/login-probe/include/play_codec.h`.
//!
//! Every buffer is caller-owned; nothing here allocates, logs, opens a socket,
//! stores a cookie, downloads a pack, loads a registry or simulates a world.
//! Pointer and length checks happen before any slice is formed. A failure
//! leaves the documented state: every byte of `pl_event` (padding included) is
//! zero, `reply` and `state` are untouched.

use core::{ptr, slice};

use crate::play::{
    decode_play_body, PlEvent, PlState, StateChange, PL_BAD_ARGUMENT, PL_BAD_FRAME, PL_OK,
    PL_OUTPUT_SMALL, PL_UNSUPPORTED,
};
use crate::Error;
use crate::{PLAY_DIMENSION_CAP, PLAY_MAX_BODY};

fn status_of(error: Error) -> i32 {
    match error {
        Error::BadArgument => PL_BAD_ARGUMENT,
        Error::OutputSmall => PL_OUTPUT_SMALL,
        // A body is already complete (the framing decoder rejected nothing):
        // there is no "need more" outcome in this ABI.
        Error::BadFrame | Error::BadProfile | Error::NeedMore => PL_BAD_FRAME,
        Error::Unsupported => PL_UNSUPPORTED,
    }
}

/// Decode one complete, already-unframed and already-inflated play body and
/// fill `*event`, `*state` and `reply`.
///
/// `body` carries the packet ID first and at most `PL_MAX_BODY` (8 MiB) bytes;
/// the configuration decoder owns framing, stream caps and compression, so
/// nothing here reads a prefix or inflates. A reply, when the packet has one,
/// is an *unframed* body of at most `PL_REPLY_BODY` (1024) bytes for the caller
/// to frame and cipher exactly once.
///
/// # Safety
///
/// The caller guarantees:
/// - when `length` is in `1..=PL_MAX_BODY`, `body` points to `length` readable
///   bytes; out-of-contract lengths are rejected before `body` is read;
/// - `state` points to one writable, aligned, `struct pl_state`-sized region
///   that does not overlap `body`, `reply` or `event`;
/// - `reply_capacity` is the exact size of the writable, aligned region at
///   `reply`; a null `reply` is only allowed with a zero capacity;
/// - `event` points to one writable, aligned, `struct pl_event`-sized region;
/// - none of the regions overlap, and every pointer stays valid for the call.
///
/// Arbitrary pointer values cannot be validated; anything that breaks the
/// above is undefined behaviour, not an error code.
#[no_mangle]
#[allow(clippy::too_many_arguments)] // The C header fixes the parameter list.
pub unsafe extern "C" fn craftrustnx_play_decode_body(
    body: *const u8,
    length: u32,
    state: *mut PlState,
    reply: *mut u8,
    reply_capacity: u32,
    event: *mut PlEvent,
) -> i32 {
    if event.is_null() {
        return PL_BAD_ARGUMENT;
    }
    // SAFETY: `event` is non-null, writable and sized per the contract above.
    unsafe { ptr::write_bytes(event.cast::<u8>(), 0, core::mem::size_of::<PlEvent>()) };

    if state.is_null() {
        return PL_BAD_ARGUMENT;
    }
    if length == 0 || length > PLAY_MAX_BODY as u32 {
        return PL_BAD_ARGUMENT;
    }
    if body.is_null() {
        return PL_BAD_ARGUMENT;
    }
    if reply.is_null() && reply_capacity != 0 {
        return PL_BAD_ARGUMENT;
    }

    // SAFETY: the length and pointers are in contract per the bounds checked
    // above; the caller guarantees the regions and their non-overlap.
    let body = unsafe { slice::from_raw_parts(body, length as usize) };
    let state_ref: &PlState = unsafe { &*state };
    let reply: &mut [u8] = if reply_capacity == 0 {
        &mut []
    } else {
        unsafe { slice::from_raw_parts_mut(reply, reply_capacity as usize) }
    };

    match decode_play_body(body, state_ref, reply) {
        Ok(decoded) => {
            // Fields are written individually (the array included) so a typed
            // struct store can never put a source padding byte into C's
            // struct; the state's own tail padding is left as the caller had
            // it.
            // SAFETY: `state` is non-null, writable and sized per the contract.
            unsafe {
                match decoded.change {
                    StateChange::None => {}
                    StateChange::Login(update) => {
                        ptr::addr_of_mut!((*state).position_bits).write([0u64; 3]);
                        ptr::addr_of_mut!((*state).yaw_bits).write(0);
                        ptr::addr_of_mut!((*state).pitch_bits).write(0);
                        ptr::addr_of_mut!((*state).player_id).write(update.player_id);
                        ptr::addr_of_mut!((*state).dimension_type).write(update.dimension_type);
                        ptr::addr_of_mut!((*state).game_mode).write(update.game_mode);
                        ptr::addr_of_mut!((*state).teleport_id).write(0);
                        ptr::addr_of_mut!((*state).login_seen).write(1);
                        ptr::addr_of_mut!((*state).position_seen).write(0);
                        ptr::copy_nonoverlapping(
                            update.dimension.as_ptr(),
                            ptr::addr_of_mut!((*state).dimension).cast::<u8>(),
                            PLAY_DIMENSION_CAP,
                        );
                    }
                    StateChange::Pose(pose) => {
                        ptr::addr_of_mut!((*state).position_bits).write(pose.position_bits);
                        ptr::addr_of_mut!((*state).yaw_bits).write(pose.yaw_bits);
                        ptr::addr_of_mut!((*state).pitch_bits).write(pose.pitch_bits);
                        ptr::addr_of_mut!((*state).teleport_id).write(pose.teleport_id);
                        ptr::addr_of_mut!((*state).position_seen).write(1);
                    }
                }
                // `*event` was zeroed above; only the fields that describe this
                // body are stored.
                let fields = decoded.event;
                ptr::addr_of_mut!((*event).kind).write(fields.kind);
                ptr::addr_of_mut!((*event).packet_id).write(fields.packet_id);
                ptr::addr_of_mut!((*event).reply_length).write(fields.reply_length);
                ptr::addr_of_mut!((*event).payload_offset).write(fields.payload_offset);
                ptr::addr_of_mut!((*event).payload_length).write(fields.payload_length);
                ptr::addr_of_mut!((*event).value).write(fields.value);
                ptr::addr_of_mut!((*event).aux).write(fields.aux);
                ptr::addr_of_mut!((*event).flags).write(fields.flags);
            }
            PL_OK
        }
        // `*event` is already zero, padding included, and neither `state` nor
        // `reply` was written.
        Err(error) => status_of(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play::PL_FLAG_PACK_REQUIRED;
    use crate::{
        PLAY_MAX_REPLY, PL_BATCH_END, PL_BATCH_START, PL_CHUNK, PL_COOKIE, PL_DISCONNECT,
        PL_KEEP_ALIVE, PL_LOGIN, PL_METADATA, PL_PING, PL_POSITION, PL_RECONFIGURE,
        PL_RESOURCE_PACK, PL_RESPAWN, PL_TRANSFER,
    };
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

    fn push_string(out: &mut Vec<u8>, text: &str) {
        push_varint(out, text.len() as i32);
        out.extend_from_slice(text.as_bytes());
    }

    fn zero_state() -> PlState {
        // SAFETY: `PlState` is plain integer data, so zero is a valid value.
        unsafe { core::mem::zeroed() }
    }

    fn state_bytes(state: &PlState) -> &[u8] {
        // SAFETY: the struct is plain data and the slice covers exactly it.
        unsafe {
            slice::from_raw_parts(
                (state as *const PlState).cast::<u8>(),
                core::mem::size_of::<PlState>(),
            )
        }
    }

    fn state_bytes_mut(state: &mut PlState) -> &mut [u8] {
        // SAFETY: as above, with a unique borrow of the whole struct.
        unsafe {
            slice::from_raw_parts_mut(
                (state as *mut PlState).cast::<u8>(),
                core::mem::size_of::<PlState>(),
            )
        }
    }

    fn event_bytes(event: &PlEvent) -> &[u8] {
        // SAFETY: the struct is plain data and the slice covers exactly it.
        unsafe {
            slice::from_raw_parts(
                (event as *const PlEvent).cast::<u8>(),
                core::mem::size_of::<PlEvent>(),
            )
        }
    }

    fn event_bytes_mut(event: &mut PlEvent) -> &mut [u8] {
        // SAFETY: as above, with a unique borrow of the whole struct.
        unsafe {
            slice::from_raw_parts_mut(
                (event as *mut PlEvent).cast::<u8>(),
                core::mem::size_of::<PlEvent>(),
            )
        }
    }

    fn sentinel_state() -> PlState {
        let mut state = zero_state();
        for byte in state_bytes_mut(&mut state) {
            *byte = 0x5a;
        }
        state
    }

    fn sentinel_event() -> PlEvent {
        let mut event = PlEvent {
            kind: 0,
            packet_id: 0,
            reply_length: 0,
            payload_offset: 0,
            payload_length: 0,
            value: 0,
            aux: 0,
            flags: 0,
        };
        for byte in event_bytes_mut(&mut event) {
            *byte = 0xa5;
        }
        event
    }

    fn call(body: &[u8], state: &mut PlState, reply: &mut [u8], event: &mut PlEvent) -> i32 {
        // SAFETY: the test buffers are alive, aligned and non-overlapping.
        unsafe {
            craftrustnx_play_decode_body(
                body.as_ptr(),
                body.len() as u32,
                state,
                reply.as_mut_ptr(),
                reply.len() as u32,
                event,
            )
        }
    }

    fn decode_ok(body: &[u8], state: &mut PlState, reply: &mut [u8]) -> PlEvent {
        let mut event = sentinel_event();
        assert_eq!(call(body, state, reply, &mut event), PL_OK);
        event
    }

    fn login_body_with(
        dimension: &str,
        levels: &[&str],
        max_players: i32,
        radius: i32,
        game_type: i32,
        previous_game_type: i32,
    ) -> Vec<u8> {
        let mut body = Vec::new();
        push_varint(&mut body, 50);
        body.extend_from_slice(&1i32.to_be_bytes()); // player_id
        body.push(0); // hardcore
        push_varint(&mut body, levels.len() as i32);
        for level in levels {
            push_string(&mut body, level);
        }
        push_varint(&mut body, max_players);
        push_varint(&mut body, radius); // chunk_radius
        push_varint(&mut body, 8); // simulation_distance
        body.push(0); // reduced_debug_info
        body.push(1); // show_death_screen
        body.push(0); // limited_crafting
        push_varint(&mut body, 0); // dimension_type
        push_string(&mut body, dimension);
        body.extend_from_slice(&0i64.to_be_bytes()); // seed
        push_varint(&mut body, game_type);
        push_varint(&mut body, previous_game_type); // OptVarInt: 0 = none
        body.push(0); // debug
        body.push(0); // flat
        body.push(0); // last_death: none
        push_varint(&mut body, 0); // portal_cooldown
        push_varint(&mut body, 63); // sea_level
        body.push(0); // online_mode
        body.push(0); // enforces_secure_chat
        body
    }

    fn login_body(dimension: &str, levels: &[&str], radius: i32) -> Vec<u8> {
        login_body_with(dimension, levels, 20, radius, 0, 0)
    }

    fn position_body(
        teleport: i32,
        pos: [f64; 3],
        delta: [f64; 3],
        yaw: f32,
        pitch: f32,
        relatives: i32,
    ) -> Vec<u8> {
        let mut body = Vec::new();
        push_varint(&mut body, 73);
        push_varint(&mut body, teleport);
        for value in pos {
            body.extend_from_slice(&value.to_be_bytes());
        }
        for value in delta {
            body.extend_from_slice(&value.to_be_bytes());
        }
        body.extend_from_slice(&yaw.to_be_bytes());
        body.extend_from_slice(&pitch.to_be_bytes());
        body.extend_from_slice(&relatives.to_be_bytes());
        body
    }

    #[test]
    fn abi_layout_and_constants_match_the_header() {
        assert_eq!(core::mem::size_of::<PlState>(), 320);
        assert_eq!(core::mem::align_of::<PlState>(), 8);
        assert_eq!(core::mem::offset_of!(PlState, position_bits), 0);
        assert_eq!(core::mem::offset_of!(PlState, yaw_bits), 24);
        assert_eq!(core::mem::offset_of!(PlState, pitch_bits), 28);
        assert_eq!(core::mem::offset_of!(PlState, player_id), 32);
        assert_eq!(core::mem::offset_of!(PlState, dimension_type), 36);
        assert_eq!(core::mem::offset_of!(PlState, game_mode), 40);
        assert_eq!(core::mem::offset_of!(PlState, teleport_id), 44);
        assert_eq!(core::mem::offset_of!(PlState, login_seen), 48);
        assert_eq!(core::mem::offset_of!(PlState, position_seen), 52);
        assert_eq!(core::mem::offset_of!(PlState, dimension), 56);
        assert_eq!(core::mem::size_of::<PlEvent>(), 32);
        assert_eq!(core::mem::offset_of!(PlEvent, kind), 0);
        assert_eq!(core::mem::offset_of!(PlEvent, packet_id), 4);
        assert_eq!(core::mem::offset_of!(PlEvent, reply_length), 8);
        assert_eq!(core::mem::offset_of!(PlEvent, payload_offset), 12);
        assert_eq!(core::mem::offset_of!(PlEvent, payload_length), 16);
        assert_eq!(core::mem::offset_of!(PlEvent, value), 20);
        assert_eq!(core::mem::offset_of!(PlEvent, aux), 24);
        assert_eq!(core::mem::offset_of!(PlEvent, flags), 28);

        assert_eq!(
            [
                PL_METADATA,
                PL_LOGIN,
                PL_POSITION,
                PL_KEEP_ALIVE,
                PL_PING,
                PL_CHUNK,
                PL_BATCH_START,
                PL_BATCH_END,
                PL_COOKIE,
                PL_RESOURCE_PACK,
                PL_DISCONNECT,
                PL_RECONFIGURE,
                PL_TRANSFER,
                PL_RESPAWN
            ],
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]
        );
        assert_eq!(
            [
                PL_OK,
                PL_BAD_ARGUMENT,
                PL_OUTPUT_SMALL,
                PL_BAD_FRAME,
                PL_UNSUPPORTED
            ],
            [0, -1, -2, -3, -5]
        );
        assert_eq!((PLAY_MAX_BODY, PLAY_MAX_REPLY), (8_388_608, 1024));
        assert_eq!(PLAY_DIMENSION_CAP, 257);
        assert_eq!(PL_FLAG_PACK_REQUIRED, 1);
    }

    #[test]
    fn login_installs_join_data_and_leaves_padding_alone() {
        let mut body = Vec::new();
        push_varint(&mut body, 50);
        body.extend_from_slice(&1234i32.to_be_bytes());
        body.push(1); // hardcore
        push_varint(&mut body, 2);
        push_string(&mut body, "minecraft:overworld");
        push_string(&mut body, "custom:nether_x");
        push_varint(&mut body, 20); // max_players
        push_varint(&mut body, 10); // chunk_radius
        push_varint(&mut body, 8); // simulation_distance
        body.push(0); // reduced_debug_info
        body.push(1); // show_death_screen
        body.push(0); // limited_crafting
        push_varint(&mut body, 0); // dimension_type
        push_string(&mut body, "minecraft:the_nether");
        body.extend_from_slice(&(-1_234_567_890_123i64).to_be_bytes());
        push_varint(&mut body, 1); // game_type: creative
        push_varint(&mut body, 2); // previous_game_type: 1 (creative)
        body.push(1); // debug
        body.push(0); // flat
        body.push(1); // last_death present
        push_string(&mut body, "minecraft:overworld");
        body.extend_from_slice(&[0, 0, 0, 8, 0, 0, 0, 4]);
        push_varint(&mut body, 0); // portal_cooldown
        push_varint(&mut body, 63); // sea_level
        body.push(1); // online_mode
        body.push(1); // enforces_secure_chat

        let mut state = zero_state();
        // Scribble the struct's tail padding (313..320) to prove it is kept.
        state_bytes_mut(&mut state)[313..320].fill(0xaa);
        let mut reply = [0xccu8; PLAY_MAX_REPLY];
        let event = decode_ok(&body, &mut state, &mut reply);

        assert_eq!(event.kind, PL_LOGIN);
        assert_eq!(event.packet_id, 50);
        assert_eq!(
            (event.payload_offset, event.payload_length),
            (1, body.len() as u32 - 1)
        );
        assert_eq!(event.reply_length, 0);
        assert!(reply.iter().all(|&byte| byte == 0xcc), "reply touched");

        assert_eq!(state.player_id, 1234);
        assert_eq!(state.dimension_type, 0);
        assert_eq!(state.game_mode, 1);
        assert_eq!(state.login_seen, 1);
        assert_eq!(state.position_seen, 0);
        assert_eq!(state.teleport_id, 0);
        assert_eq!(state.position_bits, [0, 0, 0]);
        assert_eq!((state.yaw_bits, state.pitch_bits), (0, 0));
        assert_eq!(&state.dimension[..20], b"minecraft:the_nether");
        assert!(state.dimension[20..].iter().all(|&byte| byte == 0));
        assert_eq!(&state_bytes(&state)[313..320], &[0xaa; 7]);
    }

    #[test]
    fn login_rejects_out_of_contract_fields_without_touching_outputs() {
        let long_dimension = format!("a:{}", "b".repeat(298));
        let mut levels = Vec::new();
        for index in 0..257 {
            levels.push(format!("minecraft:l{index}"));
        }
        let level_refs: Vec<&str> = levels.iter().map(|level| level.as_str()).collect();

        let mut truncated = login_body("minecraft:overworld", &[], 10);
        truncated.pop();
        let mut trailing = login_body("minecraft:overworld", &[], 10);
        trailing.push(0);

        let cases: [(Vec<u8>, i32); 5] = [
            (login_body(&long_dimension, &[], 10), PL_UNSUPPORTED),
            (
                login_body("minecraft:overworld", &level_refs, 10),
                PL_BAD_FRAME,
            ),
            (
                login_body_with("minecraft:overworld", &[], 20, -1, 0, 0),
                PL_BAD_FRAME,
            ),
            (
                login_body_with("minecraft:overworld", &[], -1, 10, 0, 0),
                PL_BAD_FRAME,
            ),
            (truncated, PL_BAD_FRAME),
        ];
        for (body, expected) in cases {
            let mut state = sentinel_state();
            let before = state_bytes(&state).to_vec();
            let mut reply = [0xccu8; PLAY_MAX_REPLY];
            let reply_before = reply;
            let mut event = sentinel_event();
            assert_eq!(call(&body, &mut state, &mut reply, &mut event), expected);
            assert!(event_bytes(&event).iter().all(|&byte| byte == 0));
            assert_eq!(state_bytes(&state), &before[..]);
            assert_eq!(reply, reply_before);
        }

        // A trailing byte after a structurally valid body is a frame error.
        let mut state = sentinel_state();
        let mut reply = [0xccu8; PLAY_MAX_REPLY];
        let mut event = sentinel_event();
        assert_eq!(
            call(&trailing, &mut state, &mut reply, &mut event),
            PL_BAD_FRAME
        );
        assert!(event_bytes(&event).iter().all(|&byte| byte == 0));
    }

    #[test]
    fn login_rejects_out_of_range_game_modes_and_counts() {
        let cases = [
            login_body_with("minecraft:overworld", &[], 20, 10, 4, 0),
            login_body_with("minecraft:overworld", &[], 20, 10, 0, 6),
            login_body_with("minecraft:overworld", &[], 20, 10, 0, 5),
            login_body_with("minecraft:overworld", &[], 20, 10, 0, i32::MIN),
        ];
        for body in cases {
            let mut state = sentinel_state();
            let before = state_bytes(&state).to_vec();
            let mut reply = [0xccu8; PLAY_MAX_REPLY];
            let mut event = sentinel_event();
            assert_eq!(
                call(&body, &mut state, &mut reply, &mut event),
                PL_BAD_FRAME
            );
            assert!(event_bytes(&event).iter().all(|&byte| byte == 0));
            assert_eq!(state_bytes(&state), &before[..]);
        }

        // The in-range edge values are accepted.
        for (game_type, previous) in [(0, 0), (3, 4)] {
            let body = login_body_with("minecraft:overworld", &[], 20, 10, game_type, previous);
            let mut state = zero_state();
            let mut reply = [0xccu8; PLAY_MAX_REPLY];
            let event = decode_ok(&body, &mut state, &mut reply);
            assert_eq!(event.kind, PL_LOGIN);
            assert_eq!(state.game_mode, game_type);
        }
    }

    #[test]
    fn position_absolute_first_is_acknowledged_with_the_full_pose() {
        let body = position_body(7, [1.5, -64.25, 300.0], [0.0, 0.0, 0.0], 90.0, -45.0, 0);
        let mut state = zero_state();
        let mut reply = [0xccu8; PLAY_MAX_REPLY];
        let event = decode_ok(&body, &mut state, &mut reply);

        let mut expected = Vec::new();
        expected.push(0u8); // Serverbound AcceptTeleportation.
        expected.push(7u8);
        for value in [1.5f64, -64.25, 300.0] {
            expected.extend_from_slice(&value.to_be_bytes());
        }
        expected.extend_from_slice(&90.0f32.to_be_bytes());
        expected.extend_from_slice(&(-45.0f32).to_be_bytes());

        assert_eq!(event.kind, PL_POSITION);
        assert_eq!(event.packet_id, 73);
        assert_eq!(event.value, 7);
        assert_eq!(event.reply_length as usize, expected.len());
        assert_eq!(&reply[..expected.len()], &expected[..]);
        assert!(reply[expected.len()..].iter().all(|&byte| byte == 0xcc));

        assert_eq!(
            state.position_bits,
            [1.5f64.to_bits(), (-64.25f64).to_bits(), 300.0f64.to_bits()]
        );
        assert_eq!(state.yaw_bits, 90.0f32.to_bits());
        assert_eq!(state.pitch_bits, (-45.0f32).to_bits());
        assert_eq!(state.teleport_id, 7);
        assert_eq!(state.position_seen, 1);
        assert_eq!(state.login_seen, 0);
    }

    #[test]
    fn position_relative_needs_a_baseline_and_adds_it() {
        // A relative axis with no absolute pose is unsupported, not a frame
        // error, and never installs state.
        let body = position_body(1, [1.0, 2.0, 3.0], [0.0; 3], 0.0, 0.0, 1);
        let mut state = zero_state();
        let before = state_bytes(&state).to_vec();
        let mut reply = [0xccu8; PLAY_MAX_REPLY];
        let mut event = sentinel_event();
        assert_eq!(
            call(&body, &mut state, &mut reply, &mut event),
            PL_UNSUPPORTED
        );
        assert!(event_bytes(&event).iter().all(|&byte| byte == 0));
        assert_eq!(state_bytes(&state), &before[..]);

        // Delta-only flags do not need a position: they are accepted and never
        // simulated.
        let body = position_body(
            5,
            [0.0, 64.0, 0.0],
            [1.0, 2.0, 3.0],
            10.0,
            20.0,
            0x20 | 0x40 | 0x80 | 0x100,
        );
        let mut state = zero_state();
        let mut reply = [0xccu8; PLAY_MAX_REPLY];
        let event = decode_ok(&body, &mut state, &mut reply);
        assert_eq!(event.kind, PL_POSITION);
        assert_eq!(state.position_seen, 1);
        assert_eq!(
            state.position_bits,
            [0.0f64.to_bits(), 64.0f64.to_bits(), 0]
        );
        assert_eq!(state.yaw_bits, 10.0f32.to_bits());
        assert_eq!(state.pitch_bits, 20.0f32.to_bits());

        // Relative x and yaw add the prior pose; absolute y, z and pitch win.
        let body = position_body(6, [1.0, 7.0, 8.0], [0.0; 3], 5.0, 30.0, 1 | 8);
        let event = decode_ok(&body, &mut state, &mut reply);
        assert_eq!(event.value, 6);
        assert_eq!(state.position_bits[0], 1.0f64.to_bits());
        assert_eq!(state.position_bits[1], 7.0f64.to_bits());
        assert_eq!(state.position_bits[2], 8.0f64.to_bits());
        assert_eq!(state.yaw_bits, 15.0f32.to_bits());
        assert_eq!(state.pitch_bits, 30.0f32.to_bits());
        assert_eq!(state.teleport_id, 6);
    }

    #[test]
    fn position_rejects_nonfinite_overflow_flags_and_short_reply() {
        // A relative sum that overflows is a frame error; state is unchanged.
        let first = position_body(1, [1e308, 0.0, 0.0], [0.0; 3], 0.0, 0.0, 0);
        let mut state = zero_state();
        let mut reply = [0xccu8; PLAY_MAX_REPLY];
        decode_ok(&first, &mut state, &mut reply);
        let before = state_bytes(&state).to_vec();

        let overflow = position_body(2, [1e308, 0.0, 0.0], [0.0; 3], 0.0, 0.0, 1);
        let mut event = sentinel_event();
        assert_eq!(
            call(&overflow, &mut state, &mut reply, &mut event),
            PL_BAD_FRAME
        );
        assert_eq!(state_bytes(&state), &before[..]);

        for body in [
            position_body(3, [f64::NAN, 0.0, 0.0], [0.0; 3], 0.0, 0.0, 0),
            position_body(3, [0.0; 3], [f64::INFINITY, 0.0, 0.0], 0.0, 0.0, 0),
            position_body(3, [0.0; 3], [0.0; 3], f32::NAN, 0.0, 0),
            position_body(3, [0.0; 3], [0.0; 3], 0.0, f32::INFINITY, 0),
            position_body(3, [0.0; 3], [0.0; 3], 0.0, 0.0, 0x200),
            position_body(-1, [0.0; 3], [0.0; 3], 0.0, 0.0, 0),
        ] {
            let mut state = zero_state();
            let mut reply = [0xccu8; PLAY_MAX_REPLY];
            let mut event = sentinel_event();
            assert_eq!(
                call(&body, &mut state, &mut reply, &mut event),
                PL_BAD_FRAME
            );
            assert!(event_bytes(&event).iter().all(|&byte| byte == 0));
        }

        // Truncation after the fixed part and one trailing byte are frame
        // errors.
        let mut truncated = position_body(3, [0.0; 3], [0.0; 3], 0.0, 0.0, 0);
        truncated.pop();
        let mut trailing = position_body(3, [0.0; 3], [0.0; 3], 0.0, 0.0, 0);
        trailing.push(0);
        for body in [truncated, trailing] {
            let mut reply = [0xccu8; PLAY_MAX_REPLY];
            let mut event = sentinel_event();
            assert_eq!(
                call(&body, &mut state, &mut reply, &mut event),
                PL_BAD_FRAME
            );
        }

        // A reply buffer too small for the full pose is output-small, with the
        // state still unchanged.
        let body = position_body(9, [1.0, 2.0, 3.0], [0.0; 3], 4.0, 5.0, 0);
        let mut small = [0xccu8; 8];
        let before = state_bytes(&state).to_vec();
        let mut event = sentinel_event();
        assert_eq!(
            call(&body, &mut state, &mut small, &mut event),
            PL_OUTPUT_SMALL
        );
        assert_eq!(state_bytes(&state), &before[..]);
        assert!(event_bytes(&event).iter().all(|&byte| byte == 0));
        assert!(small.iter().all(|&byte| byte == 0xcc));
    }

    #[test]
    fn keep_alive_ping_and_batch_replies() {
        let mut state = zero_state();
        let mut reply = [0xccu8; PLAY_MAX_REPLY];

        let mut body = vec![45u8];
        body.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let event = decode_ok(&body, &mut state, &mut reply);
        assert_eq!(event.kind, PL_KEEP_ALIVE);
        assert_eq!(event.reply_length, 9);
        assert_eq!(&reply[..9], &[28, 1, 2, 3, 4, 5, 6, 7, 8]);

        let ping = [62u8, 0x11, 0x22, 0x33, 0x44];
        let event = decode_ok(&ping, &mut state, &mut reply);
        assert_eq!(event.kind, PL_PING);
        assert_eq!(event.reply_length, 5);
        assert_eq!(&reply[..5], &[45, 0x11, 0x22, 0x33, 0x44]);

        let event = decode_ok(&[11, 5], &mut state, &mut reply);
        assert_eq!(event.kind, PL_BATCH_END);
        assert_eq!(event.value, 5);
        assert_eq!(event.reply_length, 5);
        assert_eq!(&reply[..5], &[11, 0x3f, 0x80, 0x00, 0x00]);

        let event = decode_ok(&[12], &mut state, &mut reply);
        assert_eq!(event.kind, PL_BATCH_START);
        assert_eq!(event.reply_length, 0);

        // A negative batch size and a non-empty batch start are frame errors.
        let mut event = sentinel_event();
        assert_eq!(
            call(
                &[11, 0xff, 0xff, 0xff, 0xff, 0x0f],
                &mut state,
                &mut reply,
                &mut event
            ),
            PL_BAD_FRAME
        );
        assert_eq!(
            call(&[12, 0], &mut state, &mut reply, &mut event),
            PL_BAD_FRAME
        );
    }

    #[test]
    fn cookie_and_resource_pack_replies_never_store_or_download() {
        let mut state = zero_state();
        let mut reply = [0xccu8; PLAY_MAX_REPLY];

        let mut body = vec![21u8];
        push_string(&mut body, "minecraft:probe");
        let event = decode_ok(&body, &mut state, &mut reply);
        assert_eq!(event.kind, PL_COOKIE);
        assert_eq!(
            &reply[..event.reply_length as usize],
            &[
                21, 15, b'm', b'i', b'n', b'e', b'c', b'r', b'a', b'f', b't', b':', b'p', b'r',
                b'o', b'b', b'e', 0
            ]
        );

        let mut body = vec![82u8];
        body.extend_from_slice(&UUID);
        push_string(&mut body, "http://example.invalid/pack.zip");
        push_string(&mut body, "");
        body.push(1); // required
        body.push(0); // no prompt
        let event = decode_ok(&body, &mut state, &mut reply);
        assert_eq!(event.kind, PL_RESOURCE_PACK);
        assert_eq!(event.flags, PL_FLAG_PACK_REQUIRED);
        assert_eq!(event.reply_length, 18);
        assert_eq!(reply[0], 50);
        assert_eq!(&reply[1..17], &UUID);
        assert_eq!(reply[17], 1);

        // An absent prompt must end the body; a present one must carry bytes.
        let mut event = sentinel_event();
        assert_eq!(
            call(&body[..body.len() - 1], &mut state, &mut reply, &mut event),
            PL_BAD_FRAME
        );
        let mut body = vec![82u8];
        body.extend_from_slice(&UUID);
        push_string(&mut body, "u");
        push_string(&mut body, "h");
        body.push(0);
        body.push(1); // prompt present, but no NBT bytes follow
        assert_eq!(
            call(&body, &mut state, &mut reply, &mut event),
            PL_BAD_FRAME
        );
        body.push(0); // one opaque NBT byte is enough
        assert_eq!(call(&body, &mut state, &mut reply, &mut event), PL_OK);
    }

    #[test]
    fn chunk_header_and_bundle_events() {
        let mut state = zero_state();
        let mut reply = [0xccu8; PLAY_MAX_REPLY];
        let mut body = vec![46u8];
        body.extend_from_slice(&(-3i32).to_be_bytes());
        body.extend_from_slice(&12i32.to_be_bytes());
        body.push(0x00); // opaque chunk data
        let event = decode_ok(&body, &mut state, &mut reply);
        assert_eq!(event.kind, PL_CHUNK);
        assert_eq!((event.value, event.aux), (-3, 12));
        assert_eq!((event.payload_offset, event.payload_length), (1, 9));
        assert_eq!(event.reply_length, 0);

        let mut event = sentinel_event();
        let mut header_only = vec![46u8];
        header_only.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            call(&header_only, &mut state, &mut reply, &mut event),
            PL_BAD_FRAME
        );

        let mut event = decode_ok(&[0], &mut state, &mut reply);
        assert_eq!(event.kind, PL_METADATA);
        assert_eq!(event.packet_id, 0);
        assert_eq!(
            call(&[0, 1], &mut state, &mut reply, &mut event),
            PL_BAD_FRAME
        );
    }

    #[test]
    fn disconnect_reconfigure_transfer_and_respawn_are_reported_not_followed() {
        let mut state = zero_state();
        let mut reply = [0xccu8; PLAY_MAX_REPLY];

        let event = decode_ok(&[32, 0x08, 0x00], &mut state, &mut reply);
        assert_eq!(event.kind, PL_DISCONNECT);
        assert_eq!(event.reply_length, 0);
        let mut event = sentinel_event();
        assert_eq!(
            call(&[32], &mut state, &mut reply, &mut event),
            PL_BAD_FRAME
        );

        let mut event = decode_ok(&[120], &mut state, &mut reply);
        assert_eq!(event.kind, PL_RECONFIGURE);
        assert_eq!(
            call(&[120, 0], &mut state, &mut reply, &mut event),
            PL_BAD_FRAME
        );

        let mut body = Vec::new();
        push_varint(&mut body, 132);
        push_string(&mut body, "mc.example.invalid");
        push_varint(&mut body, 25_565);
        let mut event = decode_ok(&body, &mut state, &mut reply);
        assert_eq!(event.kind, PL_TRANSFER);
        assert_eq!(event.reply_length, 0);

        let mut body = Vec::new();
        push_varint(&mut body, 132);
        push_string(&mut body, "mc.example.invalid");
        push_varint(&mut body, 0);
        assert_eq!(
            call(&body, &mut state, &mut reply, &mut event),
            PL_BAD_FRAME
        );

        // Respawn is reported as an unsupported continuation; the body is
        // opaque, so even an empty payload is only an event.
        let event = decode_ok(&[84], &mut state, &mut reply);
        assert_eq!(event.kind, PL_RESPAWN);
        assert_eq!(event.packet_id, 84);
    }

    #[test]
    fn opaque_known_ids_and_unsupported_ids() {
        let mut state = zero_state();
        let mut reply = [0xccu8; PLAY_MAX_REPLY];

        let mut body = vec![63u8, 0xde, 0xad];
        let event = decode_ok(&body, &mut state, &mut reply);
        assert_eq!(event.kind, PL_METADATA);
        assert_eq!(event.packet_id, 63);
        assert_eq!((event.payload_offset, event.payload_length), (1, 2));
        assert_eq!(event.reply_length, 0);

        body = vec![143, 1];
        let event = decode_ok(&body, &mut state, &mut reply);
        assert_eq!(event.kind, PL_METADATA);
        assert_eq!(event.packet_id, 143);

        let mut event = sentinel_event();
        assert_eq!(
            call(&[0x90, 0x01], &mut state, &mut reply, &mut event),
            PL_UNSUPPORTED
        );
        assert_eq!(
            call(
                &[0xff, 0xff, 0xff, 0xff, 0x0f],
                &mut state,
                &mut reply,
                &mut event
            ),
            PL_BAD_FRAME
        );
        assert!(event_bytes(&event).iter().all(|&byte| byte == 0));
    }

    #[test]
    fn bounds_and_null_arguments_are_checked_before_slices() {
        let mut state = zero_state();
        let mut reply = [0u8; PLAY_MAX_REPLY];
        let mut event = sentinel_event();
        let body = [45u8, 0, 0, 0, 0, 0, 0, 0, 0];

        // Null event cannot be reported through the event itself.
        assert_eq!(
            unsafe {
                craftrustnx_play_decode_body(
                    body.as_ptr(),
                    body.len() as u32,
                    &mut state,
                    reply.as_mut_ptr(),
                    reply.len() as u32,
                    core::ptr::null_mut(),
                )
            },
            PL_BAD_ARGUMENT
        );
        assert_eq!(
            unsafe {
                craftrustnx_play_decode_body(
                    body.as_ptr(),
                    body.len() as u32,
                    core::ptr::null_mut(),
                    reply.as_mut_ptr(),
                    reply.len() as u32,
                    &mut event,
                )
            },
            PL_BAD_ARGUMENT
        );
        assert!(event_bytes(&event).iter().all(|&byte| byte == 0));
        assert_eq!(
            unsafe {
                craftrustnx_play_decode_body(
                    core::ptr::null(),
                    body.len() as u32,
                    &mut state,
                    reply.as_mut_ptr(),
                    reply.len() as u32,
                    &mut event,
                )
            },
            PL_BAD_ARGUMENT
        );
        assert_eq!(
            unsafe {
                craftrustnx_play_decode_body(
                    body.as_ptr(),
                    body.len() as u32,
                    &mut state,
                    core::ptr::null_mut(),
                    4,
                    &mut event,
                )
            },
            PL_BAD_ARGUMENT
        );
        assert_eq!(
            unsafe {
                craftrustnx_play_decode_body(
                    body.as_ptr(),
                    0,
                    &mut state,
                    reply.as_mut_ptr(),
                    reply.len() as u32,
                    &mut event,
                )
            },
            PL_BAD_ARGUMENT
        );
        // An oversized length is rejected from the number alone, before the
        // body is dereferenced.
        assert_eq!(
            unsafe {
                craftrustnx_play_decode_body(
                    body.as_ptr(),
                    PLAY_MAX_BODY as u32 + 1,
                    &mut state,
                    reply.as_mut_ptr(),
                    reply.len() as u32,
                    &mut event,
                )
            },
            PL_BAD_ARGUMENT
        );

        // A zero-capacity reply cannot hold the keepalive echo.
        assert_eq!(
            unsafe {
                craftrustnx_play_decode_body(
                    body.as_ptr(),
                    body.len() as u32,
                    &mut state,
                    core::ptr::null_mut(),
                    0,
                    &mut event,
                )
            },
            PL_OUTPUT_SMALL
        );

        // A zero-length reply slice is fine for a body that needs no reply.
        let mut reply = [];
        let event = decode_ok(&[120], &mut state, &mut reply);
        assert_eq!(event.kind, PL_RECONFIGURE);
    }

    #[test]
    fn malformed_bodies_never_panic() {
        let mut state = zero_state();
        let mut reply = [0u8; PLAY_MAX_REPLY];
        let mut event = sentinel_event();

        for length in 0..=2 {
            let mut body = vec![0x80u8; length];
            if let Some(last) = body.last_mut() {
                *last = 0x00;
            }
            let _ = call(&body, &mut state, &mut reply, &mut event);
        }
        // Every bit flip of a valid position body must return, never panic.
        let base = position_body(3, [0.5, -1.0, 2.0], [0.0; 3], 1.0, 2.0, 0);
        for index in 0..base.len() {
            for bit in 0..8 {
                let mut body = base.clone();
                body[index] ^= 1 << bit;
                let _ = call(&body, &mut state, &mut reply, &mut event);
            }
        }
    }
}
