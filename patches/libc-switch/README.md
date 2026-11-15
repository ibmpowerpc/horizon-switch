# libc 0.2.189 — Switch (aarch64 newlib/libnx) layout patch

Project-local corrections to the `libc` crate's Nintendo Switch definitions, so
that Rust `size_of`/`align_of`/`offset_of!` match the C ABI that
`tools/switch-abi-audit` compiles from the real devkitA64 + libnx headers.

This is **not** a vendored dependency. Exactly one copy is staged from the
offline Cargo cache into an ignored build directory, patched there, and used
only by the audit's patched verification mode. The Cargo registry source,
`rust-src` and the installed toolchain are never modified.

```
patches/libc-switch/
  libc-0.2.189-switch-aarch64.patch   reviewable unified diff (-p1, crate root)
  apply.sh                            hashed staging + patch, idempotent
  verify-headers.sh                   compile-only check of the C header claims
  header-check.c                      _Static_asserts + initializer byte dump
```

## Provenance

| item | value |
| --- | --- |
| crate | `libc` 0.2.189 (crates.io) |
| registry checksum | `3eaf3ede3fee6db1a4c2ee091bf8a8b4dccdc6d17f656fb07896ee72867612f2` |
| cache source | `~/.cargo/registry/src/index.crates.io-*/libc-0.2.189` |
| upstream | <https://github.com/rust-lang/libc> (crate tarball, no VCS checkout) |
| licenses | `LICENSE-MIT`, `LICENSE-APACHE` are copied verbatim into the staged crate and left in place |
| files patched | `src/unix/newlib/mod.rs`, `src/unix/newlib/horizon/mod.rs` |

`apply.sh` refuses to run unless the crate name/version and the sha256 of both
pristine files match the recorded values, and it re-hashes the result:

| file | pristine sha256 | patched sha256 |
| --- | --- | --- |
| `src/unix/newlib/mod.rs` | `6bbe0874…ad66cd` | `3aa7a7a7…94362990` |
| `src/unix/newlib/horizon/mod.rs` | `329f41f1…32b122` | `8c35ee83…aa732c` |

The staged copy carries `.switch-patch-applied` with source path, both hashes
and the patch hash, so a re-run is a no-op instead of a double patch.

## Gate

Every change is one of:

- `#[cfg(all(target_arch = "aarch64", target_os = "horizon", target_env = "newlib"))]`
- a `cfg_if!` arm with that predicate, whose `else` arm is the original code
- a `#[cfg(not(…))]` complement that keeps the original definition

Nintendo3DS (`armv6k-nintendo-3ds`: `target_arch = "arm"`, same
`target_os = "horizon"`) therefore keeps its existing definitions byte for
byte, as does every other newlib target.

## Changes

C ground truth: devkitA64 GCC 15.2.0, newlib 4.6.0, libnx headers from
`devkitpro/devkita64:20260219` (`sys/_types.h`, `sys/stat.h`, `sys/dirent.h`,
`sys/_pthreadtypes.h`, `sys/lock.h`, `sys/_sockaddr_storage.h`, `sys/socket.h`,
`netinet/in.h`).

| item | upstream Rust | patched | C header reason |
| --- | --- | --- | --- |
| `dev_t` | `u32` | `c_short` | `typedef short __dev_t` |
| `ino_t` | `u32` | `c_ushort` | `typedef unsigned short __ino_t` |
| `blksize_t` | `i32` | `c_long` | `typedef long __blksize_t` (64-bit) |
| `blkcnt_t` | `i32` | `c_long` | `typedef long __blkcnt_t` |
| `sa_family_t` | `u16` | `u8` | `typedef __uint8_t __sa_family_t` |
| `sockaddr` | family + `[c_char; 26]` | `sa_len`, `sa_family`, `sa_data[14]` | libnx `sys/socket.h` |
| `sockaddr_in` | family-first | `sin_len` then family | libnx `netinet/in.h` |
| `sockaddr_storage` | family + 26 pad | `ss_len`, `ss_family`, `__ss_pad1[6]`, `__ss_align: i64`, `__ss_pad2[112]` | libnx `sys/_sockaddr_storage.h` |
| `pthread_mutex_t` | 40 / align 8 | 12 / align 4 | `int type` + `union{_LOCK_T, _LOCK_RECURSIVE_T}` |
| `pthread_cond_t` | 48 / align 8 | 16 / align 8 | `clockid_t` + `_COND_T` |
| `pthread_attr_t` | 56 / align 1 | 24 / align 8 | `void*`, `int stacksize`, `sched_param`, `int detachstate` |
| `pthread_mutexattr_t` | 8 / align 8 | 4 / align 4 | only `int type` is enabled |
| `pthread_condattr_t` | 4 / align 8 | 8 / align 8 | `clockid_t clock_id` |
| `PTHREAD_COND_INITIALIZER` | all-zero bytes | `clock_id = CLOCK_REALTIME (1)` | newlib `_PTHREAD_COND_INITIALIZER` |

`stat` and `dirent` need no structural edit: their field lists already match
newlib, so they become correct once the five scalar typedefs are correct.

`pthread_attr_t` is 24 bytes because the Switch feature set has
`_POSIX_THREADS = 1` and `_UNIX98_THREAD_MUTEX_ATTRIBUTES = 1` but **not**
`_POSIX_THREAD_PRIORITY_SCHEDULING`, `_POSIX_THREAD_CPUTIME` or
`_POSIX_THREAD_PROCESS_SHARED`. `PTHREAD_MUTEX_INITIALIZER` stays all-zero,
which matches newlib's `{PTHREAD_MUTEX_NORMAL, {.normal = 0}}`.

`verify-headers.sh` compiles `_Static_assert`s for these layouts and dumps the
initializer objects:

```
header-check: all _Static_assert checks passed (compile-only)
 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
```

First 12 bytes = `pthread_mutex_t` (all zero), followed by 4 alignment bytes;
`pthread_cond_t` starts at byte 16 (`clock_id = 1`, `cond = 0`). One containing
struct fixes the field order instead of relying on compiler ordering of globals.

## Deliberately not changed

- `pthread_rwlock_t`, `pthread_rwlockattr_t`, `pthread_barrier_t` keep the
  generic newlib sizes. They are not measured by `abi_items.def` and are
  therefore **unverified**, not claimed correct.
- `sockaddr_in6` needs no edit and is not in `abi_items.def`. libnx's struct has
  no `sin6_len`, so `sa_family` at offset 0 keeps 28/4 both before and after the
  `sa_family_t` width change. The Rust probe enforces the C layout with a
  compile-time assertion instead.
- No libc API is widened beyond the measured layouts. `sockaddr_storage`
  padding members are public only because `offset_of!` in an external crate
  cannot read private fields; they are documented as not-for-use.
- No runtime support is claimed. See the audit README for the missing pthread
  symbols in this devkitA64 image.

## Use

```sh
# verify the patch still applies and reproduces the recorded hashes
bash patches/libc-switch/apply.sh --check-only

# stage / refresh the patched copy (idempotent)
bash patches/libc-switch/apply.sh

# compile-only C header check (no execution)
bash patches/libc-switch/verify-headers.sh
```

Default staging directory:
`tools/switch-abi-audit/rust/build/patched/libc-0.2.189` (git-ignored, ~5.4 MiB).
