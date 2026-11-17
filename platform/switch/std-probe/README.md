# Switch Rust std runtime probe

An isolated Rust 1.100.0 `std` static library linked into a C/libnx `.nro`.
The offline build and link succeed. **All six checks passed on the user's Switch Lite
on 2026-11-15:** allocation, time, main-thread TLS, thread spawn/join,
mutex/condition-variable synchronization and worker TLS destructors.
Earlier hardware input and `no_std` ABI tests passed on the user's Switch Lite.

From the repository root:

```sh
bash scripts/build-switch.sh docker std
```

Output: `dist/switch/craftrustnx/craftrustnx-std-probe.nro`.
In hbmenu choose **CraftRustNX std Probe**. Up/Down selects a check, A runs only
that check, and Plus exits. Start with **ALLOC**; report PASS, FAIL, INVALID RESULT,
or the name left on screen if the process stops. Other checks are TIME, main-thread
TLS, thread spawn/join, mutex/condition-variable operations, and worker TLS destructors.
No checks run automatically. No file or network access is tested.

## Reproducible build

- Installed Homebrew Rust 1.100.0, matching rust-src and its local vendor sources.
- `preflight.sh` checks source hashes, target cfg, offline dependency resolution
  and estimated build growth against actual available disk (no fixed cap or
  reserve, per the user's 2026-11-17 update). It stages ~19 MiB of
  local source; it never installs tools, fetches crates, compiles std or deletes caches.
- A project target spec adds Unix/newlib cfg and selects pthread-key TLS with
  `has-thread-local=false`. This is a deliberate experiment choice, **not** proof
  that native ELF TLS is impossible: the final C/libnx link uses libnx's linker script.
- A local directory-source replacement feeds the ABI-patched libc to **std's own**
  build graph; a workspace `[patch.crates-io]` alone left two libc packages in the
  tested graph. Preflight verifies the selected source and target ABI assertions.
- `build.sh` runs preflight, then the generated offline build command: one job,
  release, panic=abort, no debug/LTO. Installed rust-src and registry files are unchanged.
- The existing devkitA64 image links the result with libnx and libsysbase. Docker
  uses `--pull=never --network none`.

The SDK lacks `sysconf`. The probe's C adapter implements only `_SC_PAGESIZE=8`,
returning the Switch/libnx page size 4096; unsupported queries return -1/EINVAL.
Both C and Rust assert the query number. This adapter is not a general POSIX layer.
`getrandom`, process spawning, files, sockets and the Horizon engine remain out of scope.
No fabricated random keys or unconditional-success syscall stubs are provided.

## Result contract

`craftrustnx_std_probe_run(requested: u32)` returns a u64 containing requested,
passed and failed masks in successive 16-bit fields and signature `0x5354` in
the top field. Requested bits 0..5 identify the checks. Unknown representable bits
fail; requests above 0xffff return an invalid signature (zero). Unrequested checks
never pass. The C UI validates the signature and masks before showing a result.
Checks may abort or hang on an unsupported runtime; a missing return is not a PASS.
Main-thread TLS is reset between runs so repeated checks remain meaningful.

Host verification of the harness logic (not Switch execution):

```sh
cargo test --offline --manifest-path platform/switch/std-probe/Cargo.toml --target-dir platform/switch/std-probe/build/host-target --jobs 1
```

See [runtime notes](../../../docs/rust-runtime.md) and
[verification record](../../../docs/verification.md). Three task-created core
caches were removed after preserving NROs/evidence to make room for this isolated
std build; no user caches or installed dependencies were deleted.
