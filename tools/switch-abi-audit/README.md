# Switch C ABI audit

Compile-only audit of the Nintendo Switch C ABI exposed by devkitA64, libnx
and newlib. It measures the layouts that a Rust `libc`-based std port would
have to match, and compares them with the Rust `libc` 0.2.189 definitions.

Nothing is linked or run on a Switch. The C probe is compiled to an object
file, a dedicated ELF section is dumped with `objcopy`, and Python decodes
the little-endian `uint64` table. No Rust code is built, and no dependencies
are fetched.

## Run

From any directory:

```sh
bash tools/switch-abi-audit/run.sh
```

Requirements: Docker with the existing `devkitpro/devkita64:20260219` image,
`python3` (stdlib only), and at least 10 GiB free on the build filesystem.
The script uses `docker run --pull=never --network none`, so it cannot pull or
download anything. Set `LIBC_SRC` only to override the libc source location.

## Output

All generated files stay in `build/` (git-ignored):

- `abi.json` - measured table plus source-derived Rust comparison
- `report.txt` - concise human-readable report
- `abi_probe.o`, `abi_table.bin` - compile-only artifacts
- `env.txt`, `image.txt` - compiler version, flags, image digest

## What is measured

- typedef sizes/alignments: `dev_t`, `ino_t`, `off_t`, `blksize_t`,
  `blkcnt_t`, `sa_family_t`, `socklen_t`, `time_t`, `mode_t`, `nlink_t`,
  `uid_t`, `gid_t`, `in_port_t`, `in_addr_t`
- `struct stat` size/alignment and offsets of the fields available in the
  installed headers
- `struct dirent`, `struct sockaddr`, `struct sockaddr_in`, `struct in_addr`,
  `struct sockaddr_storage`, `struct addrinfo`, `struct timespec`
- `pthread_mutex_t`, `pthread_cond_t`, `pthread_attr_t`, `pthread_key_t`,
  `pthread_mutexattr_t`, `pthread_condattr_t`

The record format is three `uint64` words: `kind, value_a, value_b`.
`kind 1` means `sizeof`/`_Alignof`; `kind 2` means `offsetof`. Names are kept
in `abi_items.def`, the single ordering source for C and Python.

## Rust comparison

Rust values are **source-derived, not measured**. `decode.py` reads the
installed `libc-0.2.189` files under
`~/.cargo/registry/src/*/libc-0.2.189/src/unix/` (`mod.rs`,
`newlib/mod.rs`, `newlib/generic.rs`, `newlib/horizon/mod.rs`), extracts the
type aliases and struct fields selected for `target_os = "horizon"`, and
computes C-like `repr(C)` layouts for aarch64. No Rust target code, spoofed
cfg, or ABI call is compiled or executed.

A size match is not enough: `stat` happens to match in total size while many
field offsets differ. The report lists both.

## Notes

- The installed libnx `horizon` definitions in Rust `libc` are for the
  ARMv6K Nintendo 3DS, not the Switch: `sa_family_t` is `u16`, `sockaddr`
  has no `sa_len`, and `sockaddr_storage` is 28 bytes instead of 128.
- `pthread_*` layout depends on newlib feature macros. The exact flags are
  recorded in `build/env.txt`; changing them can change the measured result.
- Cleanup only these artifacts with `rm -rf tools/switch-abi-audit/build`.
