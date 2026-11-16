//! Frame and control logic for the standalone Switch GPU triangle smoke test.
//!
//! This crate is the Rust side of the C ABI declared in
//! `platform/switch/gpu-probe/include/gpu_probe.h`. It owns all behaviour:
//! clear colour, geometry, animation, pause, palette, reset and the exit flag.
//! The native bridge only initializes deko3d, uploads these uniforms and draws.
//!
//! No SDK, network, allocation, mutable global, libm or recursion is used. The
//! triangle is positioned in device-pixel proportions for a 1280x720 target, so
//! the clip-space x extent is scaled by `720/1280` and the shape stays square.
//! Animation uses an integer triangular phase over a two-second period; no
//! floating-point time or trigonometry is involved.
#![cfg_attr(not(test), no_std)]

use core::mem::{align_of, offset_of, size_of};

/// A-mapped pause toggle (edge-triggered by the caller).
pub const GP_PAUSE: u32 = 1;
/// Palette cycle.
pub const GP_PALETTE: u32 = 2;
/// Reset animation and palette.
pub const GP_RESET: u32 = 4;
/// Ask the native loop to leave.
pub const GP_EXIT: u32 = 8;

/// Animation period. The sweep covers the first half, the return the second.
pub const ANIMATION_PERIOD_NS: u64 = 2_000_000_000;
/// Longest frame delta that may advance animation (suspend/resume guard).
pub const MAX_DELTA_NS: u64 = 250_000_000;
/// Number of palettes addressed by [`GpState::palette`].
pub const PALETTE_COUNT: u32 = 4;

const HALF_PERIOD_NS: u64 = ANIMATION_PERIOD_NS / 2;
/// Fixed-point denominator for the triangular sweep: `1 << 16`.
const PHASE_STEPS: u64 = 1 << 16;

/// Triangle half-width in clip space. `0.5625` wide is 360 px on a 1280-wide
/// target; the height below is also 360 px, so the shape is not stretched.
const HALF_WIDTH: f32 = 0.28125;
/// Triangle half-height in clip space (360 px on a 720-high target).
const HALF_HEIGHT: f32 = 0.5;
/// Horizontal travel of the triangle centre, from `-0.5` to `+0.5` clip space.
const SWEEP_HALF: f32 = 0.5;

const CLEAR: [f32; 4] = [0.02, 0.03, 0.06, 1.0];

const PALETTES: [[[f32; 4]; 3]; PALETTE_COUNT as usize] = [
    // RGB
    [
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 1.0],
    ],
    // Warm
    [
        [1.0, 0.5, 0.0, 1.0],
        [1.0, 0.0, 0.5, 1.0],
        [0.5, 0.0, 1.0, 1.0],
    ],
    // Cool
    [
        [0.0, 1.0, 1.0, 1.0],
        [0.25, 0.75, 1.0, 1.0],
        [0.0, 0.5, 1.0, 1.0],
    ],
    // Grayscale
    [
        [1.0, 1.0, 1.0, 1.0],
        [0.7, 0.7, 0.7, 1.0],
        [0.4, 0.4, 0.4, 1.0],
    ],
];

/// C `struct gp_state`: 32 bytes, align 8. Caller-owned, zero-initialized.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpState {
    pub frames: u64,
    pub animation_ns: u64,
    pub previous_ns: u64,
    pub paused: u32,
    pub palette: u32,
}

impl GpState {
    /// The header's documented initial value.
    pub const ZERO: Self = Self {
        frames: 0,
        animation_ns: 0,
        previous_ns: 0,
        paused: 0,
        palette: 0,
    };
}

/// C `struct gp_frame`: 128 bytes, align 4. First the clear colour, then three
/// std140 clip-space positions and three RGBA colors, then flags.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GpFrame {
    pub clear: [f32; 4],
    pub positions: [[f32; 4]; 3],
    pub colors: [[f32; 4]; 3],
    pub should_exit: u32,
    pub reserved: [u32; 3],
}

// The header is the contract; these fail the build if Rust's layout drifts.
const _: () = {
    assert!(size_of::<GpState>() == 32);
    assert!(align_of::<GpState>() == 8);
    assert!(offset_of!(GpState, frames) == 0);
    assert!(offset_of!(GpState, animation_ns) == 8);
    assert!(offset_of!(GpState, previous_ns) == 16);
    assert!(offset_of!(GpState, paused) == 24);
    assert!(offset_of!(GpState, palette) == 28);

    assert!(size_of::<GpFrame>() == 128);
    assert!(align_of::<GpFrame>() == 4);
    assert!(offset_of!(GpFrame, clear) == 0);
    assert!(offset_of!(GpFrame, positions) == 16);
    assert!(offset_of!(GpFrame, colors) == 64);
    assert!(offset_of!(GpFrame, should_exit) == 112);
    assert!(offset_of!(GpFrame, reserved) == 116);
};

/// Advances the smoke test by one frame and fills `frame`.
///
/// # Safety
///
/// `state` and `frame` must either be null or point to valid, properly aligned
/// `GpState`/`GpFrame` objects that do not overlap each other or any other
/// memory in use during the call. `state` must be initialized (zero-initialize
/// it first); `frame` may be uninitialized because every one of its 128 bytes is
/// written on success. Null `frame` returns `-1` without touching `state`; null
/// `state` zeroes a non-null `frame` and returns `-1`. Arbitrary non-null
/// pointers cannot be validated and are undefined behaviour, not an error code.
#[no_mangle]
pub unsafe extern "C" fn craftrustnx_gpu_update(
    state: *mut GpState,
    elapsed_ns: u64,
    buttons_down: u32,
    frame: *mut GpFrame,
) -> i32 {
    if frame.is_null() {
        return -1;
    }
    if state.is_null() {
        // SAFETY: `frame` is non-null and writable per the caller contract; a
        // zeroed frame is the documented output-error state.
        unsafe { frame.write_bytes(0, 1) };
        return -1;
    }

    // SAFETY: both pointers are non-null, aligned, valid and non-overlapping
    // per the caller contract. `GpState` has no padding and is initialized.
    let mut state_value = unsafe { state.read() };

    // Time first, using the pause state that was active when this frame
    // started. A press that pauses this frame still counts the elapsed time up
    // to it; a press that unpauses does not count the paused interval.
    let delta_ns = elapsed_ns
        .saturating_sub(state_value.previous_ns)
        .min(MAX_DELTA_NS);
    state_value.animation_ns %= ANIMATION_PERIOD_NS;
    if state_value.paused == 0 {
        state_value.animation_ns = (state_value.animation_ns + delta_ns) % ANIMATION_PERIOD_NS;
    }
    // Keep the timestamp base monotonic: a backwards timestamp contributes zero
    // and cannot inflate a later delta, while paused frames still advance it.
    state_value.previous_ns = state_value.previous_ns.max(elapsed_ns);

    // Only the four known bits act; every other bit is ignored.
    let mut paused = state_value.paused != 0;
    if buttons_down & GP_PAUSE != 0 {
        paused = !paused;
    }
    if buttons_down & GP_PALETTE != 0 {
        state_value.palette = (state_value.palette % PALETTE_COUNT + 1) % PALETTE_COUNT;
    } else {
        state_value.palette %= PALETTE_COUNT;
    }
    if buttons_down & GP_RESET != 0 {
        // Reset wins over a palette cycle in the same frame.
        state_value.animation_ns = 0;
        state_value.palette = 0;
    }
    state_value.paused = u32::from(paused);
    state_value.frames = state_value.frames.wrapping_add(1);

    let out = build_frame(
        state_value.animation_ns,
        state_value.palette,
        buttons_down & GP_EXIT != 0,
    );

    // SAFETY: both pointers are non-null, aligned and writable per contract.
    unsafe {
        state.write(state_value);
        frame.write(out);
    }
    0
}

/// Integer triangular phase in `0..=PHASE_STEPS`: rises for the first second,
/// falls for the second, and repeats every [`ANIMATION_PERIOD_NS`].
fn sweep_q16(animation_ns: u64) -> u64 {
    let phase_ns = animation_ns % ANIMATION_PERIOD_NS;
    let rising_ns = if phase_ns <= HALF_PERIOD_NS {
        phase_ns
    } else {
        ANIMATION_PERIOD_NS - phase_ns
    };
    // rising_ns is at most 1e9, so this multiplication cannot overflow u64.
    rising_ns * PHASE_STEPS / HALF_PERIOD_NS
}

fn build_frame(animation_ns: u64, palette: u32, should_exit: bool) -> GpFrame {
    let steps = sweep_q16(animation_ns) as f32;
    let center_x = SWEEP_HALF * (2.0 * steps / PHASE_STEPS as f32 - 1.0);
    GpFrame {
        clear: CLEAR,
        positions: [
            [center_x - HALF_WIDTH, -HALF_HEIGHT, 0.0, 1.0],
            [center_x + HALF_WIDTH, -HALF_HEIGHT, 0.0, 1.0],
            [center_x, HALF_HEIGHT, 0.0, 1.0],
        ],
        colors: PALETTES[(palette % PALETTE_COUNT) as usize],
        should_exit: u32::from(should_exit),
        reserved: [0; 3],
    }
}

// Panic calls C `abort` through libnx/newlib, as the other probes do.
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
// it. Without the shim a host C linker stops on `_rust_eh_personality`; the
// abort-built Switch core does not need or include it.
#[cfg(all(not(test), not(target_os = "horizon")))]
#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

#[cfg(test)]
mod tests {
    use super::*;
    use core::ptr;

    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
    const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

    fn filled_frame() -> GpFrame {
        GpFrame {
            clear: [f32::NAN; 4],
            positions: [[f32::NAN; 4]; 3],
            colors: [[f32::NAN; 4]; 3],
            should_exit: u32::MAX,
            reserved: [u32::MAX; 3],
        }
    }

    fn run(state: &mut GpState, elapsed_ns: u64, buttons: u32) -> GpFrame {
        let mut frame = filled_frame();
        let rc = unsafe { craftrustnx_gpu_update(state, elapsed_ns, buttons, &mut frame) };
        assert_eq!(rc, 0);
        frame
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= 1e-5,
            "expected {expected}, got {actual}"
        );
    }

    fn assert_vertices(frame: &GpFrame, center_x: f32) {
        let p = frame.positions;
        assert_close(p[0][0], center_x - 0.28125);
        assert_close(p[1][0], center_x + 0.28125);
        assert_close(p[2][0], center_x);
        for vertex in p {
            assert_close(vertex[1].abs(), 0.5);
            assert_eq!(vertex[2], 0.0);
            assert_eq!(vertex[3], 1.0);
        }
        assert_close(p[0][1], -0.5);
        assert_close(p[1][1], -0.5);
        assert_close(p[2][1], 0.5);
    }

    fn frame_is_zero(frame: &GpFrame) -> bool {
        frame.clear.iter().all(|value| value.to_bits() == 0)
            && frame
                .positions
                .iter()
                .flatten()
                .all(|value| value.to_bits() == 0)
            && frame
                .colors
                .iter()
                .flatten()
                .all(|value| value.to_bits() == 0)
            && frame.should_exit == 0
            && frame.reserved == [0; 3]
    }

    #[test]
    fn abi_layout_and_signature_match_the_c_header() {
        assert_eq!(size_of::<GpState>(), 32);
        assert_eq!(align_of::<GpState>(), 8);
        assert_eq!(offset_of!(GpState, frames), 0);
        assert_eq!(offset_of!(GpState, animation_ns), 8);
        assert_eq!(offset_of!(GpState, previous_ns), 16);
        assert_eq!(offset_of!(GpState, paused), 24);
        assert_eq!(offset_of!(GpState, palette), 28);

        assert_eq!(size_of::<GpFrame>(), 128);
        assert_eq!(align_of::<GpFrame>(), 4);
        assert_eq!(offset_of!(GpFrame, clear), 0);
        assert_eq!(offset_of!(GpFrame, positions), 16);
        assert_eq!(offset_of!(GpFrame, colors), 64);
        assert_eq!(offset_of!(GpFrame, should_exit), 112);
        assert_eq!(offset_of!(GpFrame, reserved), 116);

        assert_eq!((GP_PAUSE, GP_PALETTE, GP_RESET, GP_EXIT), (1, 2, 4, 8));
        let _: unsafe extern "C" fn(*mut GpState, u64, u32, *mut GpFrame) -> i32 =
            craftrustnx_gpu_update;
    }

    #[test]
    fn zero_state_produces_the_initial_frame() {
        let mut state = GpState::ZERO;
        let frame = run(&mut state, 0, 0);

        assert_eq!(
            state,
            GpState {
                frames: 1,
                animation_ns: 0,
                previous_ns: 0,
                paused: 0,
                palette: 0,
            }
        );
        assert_eq!(frame.clear, [0.02, 0.03, 0.06, 1.0]);
        // Phase zero puts the centre at the left end of the sweep.
        assert_vertices(&frame, -0.5);
        assert_eq!(frame.colors, [RED, GREEN, BLUE]);
        assert_eq!(frame.should_exit, 0);
        assert_eq!(frame.reserved, [0; 3]);
    }

    #[test]
    fn triangle_sweeps_horizontally_at_device_pixel_proportions() {
        // Seed the phase and timestamp so delta is zero and only geometry is
        // exercised. Independent expected centres: -0.25, 0, +0.25, +0.5.
        for &(animation_ns, center_x) in &[
            (1_250_000_000u64, 0.25f32),
            (500_000_000, 0.0),
            (1_750_000_000, -0.25),
            (1_000_000_000, 0.5),
            (0, -0.5),
        ] {
            let mut state = GpState {
                animation_ns,
                previous_ns: 5_000_000_000,
                ..GpState::ZERO
            };
            let frame = run(&mut state, 5_000_000_000, 0);
            assert_eq!(state.animation_ns, animation_ns);
            assert_vertices(&frame, center_x);
        }
    }

    #[test]
    fn pause_toggle_freezes_animation_but_keeps_time_base_moving() {
        let mut state = GpState::ZERO;
        let first = run(&mut state, 100_000_000, GP_PAUSE);
        // Elapsed time before the pause press is counted, then the sweep stops.
        assert_eq!(state.animation_ns, 100_000_000);
        assert_eq!(state.paused, 1);
        assert_eq!(state.previous_ns, 100_000_000);

        let frozen = run(&mut state, 600_000_000, 0);
        assert_eq!(state.animation_ns, 100_000_000);
        assert_eq!(state.previous_ns, 600_000_000);
        assert_eq!(frozen.positions, first.positions);

        // Unpause: the paused interval must not be added.
        run(&mut state, 700_000_000, GP_PAUSE);
        assert_eq!(state.paused, 0);
        assert_eq!(state.animation_ns, 100_000_000);
        assert_eq!(state.previous_ns, 700_000_000);

        run(&mut state, 800_000_000, 0);
        assert_eq!(state.animation_ns, 200_000_000);
        assert_eq!(state.frames, 4);
    }

    #[test]
    fn backwards_timestamp_contributes_zero_and_is_not_recounted() {
        let mut state = GpState {
            animation_ns: 400_000_000,
            previous_ns: 1_000_000_000,
            ..GpState::ZERO
        };

        let frame = run(&mut state, 500_000_000, 0);
        assert_eq!(state.animation_ns, 400_000_000);
        assert_eq!(state.previous_ns, 1_000_000_000);
        assert_vertices(&frame, -0.1);

        // Only the 100 ms forward of the old anchor is new; the 500 ms that the
        // clock moved backwards is not counted twice.
        run(&mut state, 1_100_000_000, 0);
        assert_eq!(state.animation_ns, 500_000_000);
    }

    #[test]
    fn large_timestamp_jump_is_capped_at_250_ms() {
        let mut state = GpState::ZERO;
        let frame = run(&mut state, 10_000_000_000, 0);
        assert_eq!(state.animation_ns, MAX_DELTA_NS);
        assert_eq!(state.previous_ns, 10_000_000_000);
        assert_vertices(&frame, -0.25);

        run(&mut state, 10_100_000_000, 0);
        assert_eq!(state.animation_ns, 350_000_000);
    }

    #[test]
    fn animation_wraps_at_two_seconds() {
        let mut state = GpState {
            animation_ns: 1_900_000_000,
            previous_ns: 0,
            ..GpState::ZERO
        };
        run(&mut state, 250_000_000, 0);
        assert_eq!(state.animation_ns, 150_000_000);
        assert!(state.animation_ns < ANIMATION_PERIOD_NS);
    }

    #[test]
    fn palette_cycles_and_reset_restores_the_start() {
        let mut state = GpState::ZERO;
        let initial = run(&mut state, 0, 0);
        assert_eq!(state.palette, 0);

        let warm = run(&mut state, 0, GP_PALETTE);
        assert_eq!(state.palette, 1);
        assert_eq!(
            warm.colors,
            [
                [1.0, 0.5, 0.0, 1.0],
                [1.0, 0.0, 0.5, 1.0],
                [0.5, 0.0, 1.0, 1.0],
            ]
        );

        for expected in [2, 3, 0] {
            run(&mut state, 0, GP_PALETTE);
            assert_eq!(state.palette, expected);
        }
        assert_eq!(
            run(&mut state, 0, 0).colors,
            initial.colors,
            "palette must wrap back to the default"
        );

        // Palette and animation return to zero; pause and frame count do not.
        state.animation_ns = 1_500_000_000;
        state.paused = 1;
        let before = state.frames;
        run(&mut state, 1_500_000_000, GP_PALETTE);
        assert_eq!(state.palette, 1);
        let reset = run(&mut state, 1_500_000_000, GP_RESET);
        assert_eq!(state.animation_ns, 0);
        assert_eq!(state.palette, 0);
        assert_eq!(state.paused, 1);
        assert_eq!(state.frames, before + 2);
        assert_eq!(reset.colors, initial.colors);
        assert_vertices(&reset, -0.5);
    }

    #[test]
    fn unknown_buttons_are_ignored_and_exit_is_per_frame() {
        let mut state = GpState::ZERO;
        let frame = run(&mut state, 0, 0xffff_fff0);
        assert_eq!(
            state,
            GpState {
                frames: 1,
                ..GpState::ZERO
            }
        );
        assert_eq!(frame.should_exit, 0);
        assert_eq!(frame.reserved, [0; 3]);

        let frame = run(&mut state, 0, GP_EXIT);
        assert_eq!(frame.should_exit, 1);
        assert_eq!(frame.reserved, [0; 3]);
        assert_eq!(state.paused, 0);
        assert_eq!(state.palette, 0);

        assert_eq!(run(&mut state, 0, 0).should_exit, 0);
    }

    #[test]
    fn null_pointers_report_error_without_running_the_frame() {
        let mut state = GpState::ZERO;
        let mut frame = filled_frame();
        let rc =
            unsafe { craftrustnx_gpu_update(ptr::null_mut(), 5, GP_EXIT | GP_PAUSE, &mut frame) };
        assert_eq!(rc, -1);
        assert!(frame_is_zero(&frame));
        assert_eq!(state, GpState::ZERO);

        let untouched = filled_frame();
        let rc =
            unsafe { craftrustnx_gpu_update(&mut state, 5, GP_EXIT | GP_PAUSE, ptr::null_mut()) };
        assert_eq!(rc, -1);
        assert_eq!(state, GpState::ZERO);
        assert_eq!(untouched.should_exit, u32::MAX);
        assert_eq!(untouched.reserved, [u32::MAX; 3]);

        let rc = unsafe { craftrustnx_gpu_update(ptr::null_mut(), 5, 0, ptr::null_mut()) };
        assert_eq!(rc, -1);
    }

    #[test]
    fn every_palette_and_phase_stays_finite_and_inside_the_clip_volume() {
        for palette in 0..PALETTE_COUNT {
            for &animation_ns in &[
                0u64,
                1,
                250_000_000,
                999_999_999,
                1_000_000_000,
                1_500_000_000,
                1_999_999_999,
            ] {
                let mut state = GpState {
                    frames: 9,
                    animation_ns,
                    previous_ns: 7_000_000_000,
                    paused: 0,
                    palette,
                };
                let frame = run(&mut state, 7_000_000_000, 0);
                assert_eq!(state.animation_ns, animation_ns);
                assert_eq!(state.palette, palette);

                for value in frame
                    .clear
                    .iter()
                    .chain(frame.positions.iter().flatten())
                    .chain(frame.colors.iter().flatten())
                {
                    assert!(value.is_finite());
                }
                assert_eq!(frame.clear[3], 1.0);
                assert!((0.0..=1.0).contains(&frame.clear[0]));
                for vertex in frame.positions {
                    assert!((-1.0..=1.0).contains(&vertex[0]));
                    assert!((-1.0..=1.0).contains(&vertex[1]));
                    assert_eq!(vertex[2], 0.0);
                    assert_eq!(vertex[3], 1.0);
                }
                for color in frame.colors {
                    for channel in color {
                        assert!((0.0..=1.0).contains(&channel));
                    }
                    assert_eq!(color[3], 1.0);
                }
                assert_eq!(frame.reserved, [0; 3]);
                assert!(frame.should_exit <= 1);
            }
        }
    }
}
