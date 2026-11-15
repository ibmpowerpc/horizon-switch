# Rust no_std staticlib experiment

This is a C ABI smoke test for native Nintendo Switch homebrew. It does not
provide the `std` runtime required by Horizon.

From the repository root:

```sh
bash scripts/build-switch.sh docker rust
```

To build only the static library:

```sh
bash platform/switch/rust-probe/build.sh
```

The script requires installed Rust 1.100.0 and its matching rust-src. It uses
process-local `RUSTC_BOOTSTRAP=1` to enable the experimental `build-std=core`
option, with offline resolution, a locked dependency graph, one job, Cortex-A57
code generation, PIC and `panic=abort`. No installation or download is attempted.
It checks the 10 GiB free-space reserve and 512 MiB artifact limit before and
after building; these checks are not a continuous filesystem quota.

Output: `target/aarch64-nintendo-switch-freestanding/release/libcraftrustnx_rust_probe.a`.

The exported function is:

```rust
pub extern "C" fn craftrustnx_rust_mix(input: u64) -> u64
```

It multiplies by `0x9e3779b97f4a7c15` with wrapping arithmetic, rotates left by
17 bits and XORs with `0x4352414654525553`. The linked C application checks fixed
vectors and frame/input-dependent values. Its screen must report `Rust ABI: PASS`,
increasing checks and zero errors. Panic calls C `abort()` through libnx/newlib.

Files, allocation, TLS, threads, sockets, HTTPS and Horizon gameplay remain untested.
See [runtime investigation](../../../docs/rust-runtime.md) and
[verification record](../../../docs/verification.md).
