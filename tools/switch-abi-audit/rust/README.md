# Rust ABI measurement: libc 0.2.189 on Switch/newlib

Compile-only comparison of **compiler-measured** Rust layouts against the
already measured Switch C ABI (`../build/abi.json`). The C table came from
devkitA64 GCC; this probe never links or runs target code.

Two modes share one measurement source, one profile, one `RUSTFLAGS` string and
one `CARGO_TARGET_DIR`:

| mode | libc source | command | outputs |
| --- | --- | --- | --- |
| baseline (unpatched) | offline registry `libc-0.2.189` | `bash tools/switch-abi-audit/rust/run.sh` | `build/` |
| patched verification | local copy + `patches/libc-switch/` | `bash tools/switch-abi-audit/rust/run-patched.sh` | `build/patched/` |

Both work from any current directory. The baseline is unchanged and re-runnable;
the patched run never overwrites its evidence.

## Toolchain and exact flags (identical in both modes)

- rustc/cargo 1.100.0 (Homebrew), matching rust-src from `rustc --print sysroot`
- target `aarch64-nintendo-switch-freestanding`
- process-local cfg, not global config:
  `--cfg unix --cfg target_family="unix" --cfg target_env="newlib"`
  with `-Aexplicit_builtin_cfgs_in_flags`
- `RUSTC_BOOTSTRAP=1 cargo -Z build-std=core build --offline --locked
  --target aarch64-nintendo-switch-freestanding --release --jobs 1`
- `RUSTFLAGS="-C target-cpu=cortex-a57 -C relocation-model=pic --cfg unix
  --cfg target_family=\"unix\" --cfg target_env=\"newlib\"
  -Aexplicit_builtin_cfgs_in_flags"`
- shared `CARGO_TARGET_DIR=platform/switch/rust-probe/target`; release without
  debug info and without LTO
- section extracted from the staticlib member with devkitA64
  `aarch64-none-elf-objcopy --dump-section .switch_abi=...` in the existing
  `devkitpro/devkita64:20260219` image, `--pull=never --network none`

`run-patched.sh` compares its `RUSTFLAGS` with `build/env.txt` and refuses to
run if they differ, and it reuses the baseline target dir so `build-std=core`
artifacts are shared rather than rebuilt. The patched crate differs only in its
`libc` path dependency and a `patched-libc` feature (a feature, not an extra
`--cfg`, because a new `RUSTFLAGS` string would change the core fingerprint);
its staticlib is named `libswitch_abi_probe_rust_patched.a` so the baseline
artifact is not replaced. `patched/Cargo.lock` is generated offline.

## Measurement

`src/lib.rs` writes 69 records in `abi_items.def` order: kind 1 = size/align,
kind 2 = field offset, kind 3 = no public Rust field (no value invented). The
record count and order are the same in both modes; only the six records that
upstream cannot express switch between kind 3 and a measured offset. The decoder
reads the raw little-endian table, verifies magic/count, reads the ELF machine
word from the object (must be AArch64/183), and compares only with
`abi.json["measured"]`. The source-derived model in `../decode.py` is not used.

`--strict` additionally exits non-zero if any covered size/alignment/offset
still differs; unmeasured items stay allowed but are listed. `run-patched.sh`
always passes `--strict`.

## Result: unpatched baseline (2026-11-15)

Type size/align matches 15/28; field offsets 18/35; **30 mismatches; 6 fields
unmeasured**. Full list in `build/comparison.txt`.

## Result: patched verification (2026-11-15)

```
Summary: type size/align 28/28; field offsets 41/41; 0 mismatches; 0 unmeasured
Strict verification (patched-libc): PASS - 69/69 covered checks match
```

The six previously unmeasured C fields (`sa_len`, `sin_len`, `ss_len`,
`__ss_pad1`, `__ss_align`, `__ss_pad2`) are now measured offsets, which is why
the field-offset denominator rises from 35 to 41. Evidence:
`build/patched/comparison.txt`, `build/patched/rust-abi.json`,
`build/patched/rust_abi_table.bin`, `build/patched/rust_abi_probe.o`,
`build/patched/env.txt`, `build/patched/object-header.txt`.

The strict gate is not self-certifying: `build/patched/strict-negative-check.txt`
runs the same decoder with `--strict` on the *unpatched* table and records
`exit_status=1` / `FAIL - 33/63 covered checks match`, so a remaining covered
mismatch both fails the run and is visible.

## Initializer / runtime gaps (sizes are not support)

Compiled checks for things `abi_items.def` does not measure:

- `sockaddr_in6` = 28 bytes, align 4, `sin6_family` at 0, `sin6_addr` at 8:
  libnx's struct has no `sin6_len`, so the `sa_family_t` width change does not
  move it. Enforced by a compile-time assertion in `src/lib.rs`.
- `patches/libc-switch/verify-headers.sh` asserts the pthread object
  composition from the real headers and dumps initializer bytes:
  `pthread_mutex_t` all zero (12 bytes), `pthread_cond_t`
  `clock_id = CLOCK_REALTIME (1)` then `cond = 0` (16 bytes).
  `PTHREAD_MUTEX_INITIALIZER` stays all-zero; only
  `PTHREAD_COND_INITIALIZER` needed a non-zero first byte.

No runtime support is implied. In `devkitpro/devkita64:20260219`, neither
`libnx.a` nor newlib `libc.a` defines `pthread_create`,
`pthread_mutex_lock`, `pthread_cond_wait`, `__libc_lock_acquire` or
`__libc_cond_wait`, and `libpthread.a` is an 820-byte stub archive. libnx
exposes its own `mutexLock`/`condvar*`/`threadCreate` API instead. Correct
sizes/alignments are therefore ABI-only: Rust `std` file/thread/network use on
Horizon still has no linked implementation behind it.

## Constraints

Rust build artifacts stay in the shared experiment target dir and are checked
against the 512 MiB cap; the 10 GiB host-free reserve is checked before and
after. Only `tools/switch-abi-audit/rust/` and `patches/libc-switch/` are
written. No downloads, installs, registry or `rust-src` edits, console transfer
or target execution.
