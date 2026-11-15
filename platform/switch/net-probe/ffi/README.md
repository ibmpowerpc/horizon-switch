# Status codec C ABI

Thin, allocation-free `staticlib` wrapper that exposes the probe's `no_std`
Java status codec (`../codec`) through the exact C ABI in
`../include/status_codec.h`. It adds no packet logic: every byte written or
parsed comes from `craftrustnx_net_probe_codec`, so wire behaviour has one
source of truth. Nothing here opens a socket, logs, allocates, reads JSON
syntax or runs on a console.

Output artifact: `libcraftrustnx_status_ffi.a` (crate type `staticlib`).

## Exports

| Symbol | Behaviour |
|---|---|
| `craftrustnx_status_encode(host, host_len, port, protocol, out, capacity, written)` | Writes the two request frames (handshake, next-state 1, then `[1, 0]`) and stores the byte count in `written`. |
| `craftrustnx_status_decode(data, length, json_offset, json_length, consumed)` | Parses one response frame; the JSON slice is `data[json_offset .. json_offset + json_length]`, and `consumed` excludes any following frame (pong). |
| `craftrustnx_status_max_frame_bytes()` | `16399`: the codec's `MAX_FRAME_BYTES` body plus the widest 5-byte outer length prefix. Size the receive buffer with this. |

Status codes match the header enum exactly:

| Code | Meaning |
|---|---|
| `0` | OK |
| `1` | Need more bytes (incomplete frame) |
| `-1` | Bad argument: null pointer, or a null output/`written` |
| `-2` | Output buffer too small |
| `-3` | Bad frame: VarInt overflow, bad length, bad id, bad UTF-8, inner mismatch |
| `-4` | Bad host: empty, more than 253 bytes, not valid UTF-8, or not printable ASCII |

## Error behaviour

- `encode` sets `*written = 0` first, so every failure leaves it `0` and no
  encoder failure touches `out`.
- `decode` resets all three scalar outputs to `0` first, so a failure never
  leaves stale offsets. Offsets are relative to `data`.
- A `host_len` outside `1..=253` returns `-4` without reading it. Decode requires
  valid input memory of the declared length; arbitrary oversized pointers cannot
  be checked. Null input with nonzero length returns `-1`.
- Empty input (`length == 0`, or `data == null` with `length == 0`) is
  `NeedMore`, the honest answer for an empty stream.
- `-1` versus `-4`: a null pointer is `-1`; a non-null host with bad content
  (including non-UTF-8 bytes) is `-4`.

## Memory contract

All buffers are caller-owned. The header's comment is the rule: every non-null
pointer must address valid, non-overlapping memory of the declared length,
readable for inputs and writable for outputs, and must stay valid for the whole
call. A pointer that breaks that contract is undefined behaviour, not an error
code; arbitrary pointer values cannot be validated from inside the library.

## Build and test

Host checks are offline and use a private target directory (`build/host-target`,
git-ignored), never a shared `target/`:

```sh
cargo test  --offline --jobs 1 --target-dir build/host-target
cargo clippy --offline --jobs 1 --all-targets --target-dir build/host-target -- -D warnings
cargo build --offline --jobs 1 --release --target-dir build/host-target
```

Seven tests exercise the real `extern "C"` symbols: the independent
`localhost:25565` / protocol `777` request vector, the response
`[4, 0, 2, 123, 125]` with offsets `3/2/5`, every incomplete prefix, malformed
and oversized frame lengths, small output buffers, null pointers, and an
oversized host length passed with a one-byte allocation to prove it is never
dereferenced. No network or console execution occurs.

The library is `no_std` outside `cfg(test)` and calls C `abort` on panic. The panic
handler is excluded from host tests, which use `std`. Release is `panic = "abort"`, `debug = 0`,
`lto = false`, `codegen-units = 1`; the test profile uses `debug = 0` and no
incremental compilation. No feature or allocator is pulled in.

A host-only `rust_eh_personality` stub is exported, because the precompiled `core`
can keep unwind-table references even under `panic = "abort"` and a `no_std`
archive has no std to satisfy them. It is what lets a plain C program link the
archive. Switch builds rebuild abort-only core and do not include this stub.

## Limitations

- Status only: no login, ping/pong timing, SRV or IDNA.
- The response JSON is returned as bytes; it is not parsed.
- A host string is validated by the codec (printable ASCII, ≤253 bytes); this
  wrapper only adds the UTF-8 check.
- Host tests are codec/ABI evidence, not proof of a live Java server.
