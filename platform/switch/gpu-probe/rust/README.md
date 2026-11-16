# Rust frame logic for the Switch GPU triangle probe

Standalone `no_std` staticlib implementing the Rust half of
[`../include/gpu_probe.h`](../include/gpu_probe.h). It owns the clear colour,
triangle geometry, animation, pause, palette, reset and exit decisions. The
native bridge owns only deko3d/libnx: it initializes the GPU, uploads these
uniforms and draws. No SDK call, allocation, network, mutable global, libm call
or recursion happens here.

The crate is its own `[workspace]`, has no dependencies, and builds
`libcraftrustnx_gpu_probe.a`.

## Exported ABI

```rust
pub unsafe extern "C" fn craftrustnx_gpu_update(
    state: *mut GpState,
    elapsed_ns: u64,
    buttons_down: u32,
    frame: *mut GpFrame,
) -> i32
```

`GpState` is 32 bytes/align 8 (`frames`, `animation_ns`, `previous_ns`,
`paused`, `palette`); `GpFrame` is 128 bytes/align 4 (`clear[4]`,
`positions[3][4]`, `colors[3][4]`, `should_exit`, `reserved[3]`). Compile-time
assertions pin every size, alignment and field offset to the header. The result
is `0` on success and `-1` on a null pointer. A null `state` zeroes a non-null
frame; a null `frame` leaves the state untouched.

## Caller safety

The caller guarantees that non-null pointers are aligned, valid, writable and
non-overlapping for the whole call. `GpState` must be initialized; zeroes are
the documented initial value. `GpFrame` need not be initialized because every
byte is written on success. Arbitrary non-null pointers cannot be validated from
inside the library, so they are undefined behaviour rather than an error code.

`buttons_down` is expected to contain newly pressed buttons (libnx `padGetButtonsDown`
style), so holding A does not toggle pause repeatedly. Unknown bits are ignored.

## Behaviour

| Input | Effect |
|---|---|
| `GP_PAUSE` | Toggles `paused`. |
| `GP_PALETTE` | Advances `palette` through four palettes, wrapping. |
| `GP_RESET` | Sets `animation_ns = 0` and `palette = 0`; pause and frame count stay. Reset wins over a palette cycle in the same frame. |
| `GP_EXIT` | Sets `should_exit = 1` for this frame. |
| anything else | Ignored. |

Time is advanced before buttons are applied, using the pause state from the
start of the frame:

- `delta = min(elapsed_ns - previous_ns, 250 ms)`, saturating on a backwards
  timestamp (zero delta, no underflow).
- The 250 ms cap prevents a suspend/resume or debugger pause from jumping the
  animation.
- `previous_ns` advances on every successful call, including paused frames, and
  is clamped to a monotonic maximum so a backwards timestamp is not recounted
  later.
- Animation is `(animation_ns + delta) mod 2 s`; `GP_RESET` and the modulo keep
  it bounded. The phase is integer-only: it rises from 0 to 1 s, falls back
  during the second second, and repeats.

The triangle is 360x360 device pixels on a 1280x720 target. Clip-space x spans
`-1..1` over 1280 px while y spans `-1..1` over 720 px, so the half-width is
`0.28125` (a `0.5625` base) and the half-height is `0.5`. Its centre sweeps
horizontally from `x = -0.5` to `+0.5`; the shape stays inside the clip volume.
Vertices are `(cx - 0.28125, -0.5)`, `(cx + 0.28125, -0.5)`, `(cx, 0.5)`, each
with `z = 0` and `w = 1`. Clear colour is opaque dark blue; palettes are RGB,
warm, cool and grayscale, all with alpha 1. Every generated float is finite and
bounded.

## Build and test (host, offline)

The host checks use a private git-ignored target directory and one job; they
never touch a shared `target/`:

```sh
cargo fmt --all -- --check
cargo test --offline --jobs 1 --target-dir build/host-target
cargo clippy --offline --jobs 1 --all-targets --target-dir build/host-target -- -D warnings
cargo build --offline --jobs 1 --release --target-dir build/host-target
```

Eleven tests cover the C ABI offsets and signature, the independent initial
triangle coordinates, the horizontal sweep with square pixels, pause and
unpause transitions with a frozen animation, backwards timestamps, the 250 ms
cap, two-second wraparound, palette cycling/reset, unknown buttons and exit,
null-pointer handling, and finite/bounded output for every palette and phase.

Release uses `panic = "abort"`, `debug = 0`, `lto = false`,
`codegen-units = 1`; dev and test use `debug = 0` with incremental compilation
off. Panics call C `abort` through libnx/newlib; a host-only
`rust_eh_personality` stub is exported because precompiled `core` can keep
unwind-table references.

## Switch cross-build

The parent build drives this crate with the existing `rust-probe` core cache
(no new toolchain, no downloads):

```sh
cd platform/switch/gpu-probe/rust
CARGO_TARGET_DIR=../../rust-probe/target \
RUSTFLAGS='-C target-cpu=cortex-a57 -C relocation-model=pic' \
RUSTC_BOOTSTRAP=1 cargo -Z build-std=core build --offline --locked \
  --target aarch64-nintendo-switch-freestanding --release --jobs 1
```

Archive:
`platform/switch/rust-probe/target/aarch64-nintendo-switch-freestanding/release/libcraftrustnx_gpu_probe.a`.
The C bridge and `.nro` packaging stay in the parent probe; this crate changes
none of them.
