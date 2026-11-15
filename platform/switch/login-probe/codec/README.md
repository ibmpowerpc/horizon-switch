# Login codec (protocol 777)

Bounded, allocation-free Rust codec for the login state of Java 26.3
(protocol 777), built as a `staticlib` (`libcraftrustnx_login_codec.a`) for the
CraftRustNX Switch login probe. The C ABI is exactly
[`../include/login_codec.h`](../include/login_codec.h) (read-only here); the C
source-login worker, build scripts and hardware runs live beside it.

It is a diagnostic only: it *describes* the exchange. It does not connect,
resolve names, authenticate, reply, acknowledge configuration, enter play, log
or allocate. Everything is `no_std`, no floats, caller-owned buffers.

```sh
cargo test  --offline --locked            # host, ignored build dir
cargo clippy --offline --locked --all-targets -- -D warnings
cargo build --offline --locked --release  # libcraftrustnx_login_codec.a
```

`Cargo.lock` is committed and resolves offline to `miniz_oxide 0.8.9`
(`default-features = false`, no-alloc) plus its `adler2 2.0.1`. No other
dependency, and nothing is fetched: builds use `--offline --locked`.

Host artifacts go to `build/host-target` (or `target/`); both are ignored and
kept well under the 20 MiB host budget. `release` is `panic = "abort"`,
`debug = 0`, `lto = false`, `codegen-units = 1`; `test` is `debug = 0`,
`incremental = false`.

## `craftrustnx_login_start`

Writes two complete TCP frames:

1. handshake: id 0, protocol **777**, host, big-endian port, next state **2**;
2. `LoginStart`: id 0, username, then the offline UUID — the 16-byte MD5 of
   `"OfflinePlayer:" + name` with version 3 and the IETF variant bits
   (`Notch` → `b50ad385-829d-3141-a216-7e7d7539ba7f`).

MD5 is implemented here for exactly the one padded 64-byte block this needs
(prefix 14 + name ≤ 16 = ≤ 30 bytes) with the fixed RFC 1321 constants: no
allocation and no `sin()`/floats.

Arguments: `1..=253` graphic ASCII host bytes, non-zero port, `1..=16`
username bytes of ASCII letters, digits and `_`. A rejected argument
(including a bad name) returns `LP_BAD_ARGUMENT`; a too-small `out` returns
`LP_OUTPUT_SMALL`. In both cases `*written == 0` and `out` is untouched.

## `craftrustnx_login_decode`

Decodes **one** complete clientbound frame from `data`, writes the packet body
(packet ID included) into `scratch`, and fills `struct lp_event`. Only that
frame is consumed, so a caller can retry with `data + event.consumed`; a
coalesced second frame is left alone.

`compression_threshold` is `-1` before Set Compression (raw frames) or the
announced threshold. With a threshold the frame carries the envelope:

- `declared == 0`: the rest of the frame is the raw body and must be **below**
  the threshold (impossible at threshold 0);
- `declared > 0`: must be `>= threshold` and `<= 65536`, and the rest of the
  frame is a zlib stream that must inflate to exactly `declared` bytes, must
  consume the rest of the frame exactly (no trailing bytes inside the frame),
  and its Adler-32 trailer is verified by `miniz_oxide`'s core inflater. The
  declared length bounds the expansion, so a zip bomb or garbage is
  `LP_BAD_FRAME` instead of work.

Caps: a body is `1..=65536` bytes and a frame `<= 65541` wire bytes. A
complete-but-malformed inner frame is `LP_BAD_FRAME`; an incomplete outer frame
(including an incomplete prefix, or a still-possible `VarInt`) is
`LP_NEED_MORE` and `data` is not consumed.

The zlib inflater keeps its ~10.3 KiB state on the stack inside this call (only
when a compressed body is unpacked), so the worker thread that calls it needs
room for that on top of `scratch`; raw framing uses none.

`event.data_offset`/`event.data_length` describe the **payload after the packet
ID** inside `scratch`, uniformly for every event, so length-prefixed fields
such as a disconnect reason or a cookie key stay inspectable without another
copy.

### Events

| ID | Event | Validated fields |
|---|---|---|
| 0 | `LP_DISCONNECT` | one length-prefixed UTF-8 JSON string, not syntax-parsed, no trailing bytes |
| 1 | `LP_ENCRYPTION_REQUEST` (`should_authenticate` exposed as 0/1) | server id (≤20 UTF-16 units), key byte array, challenge byte array, bool; the attempt stops here whatever the bool says |
| 2 | `LP_LOGIN_SUCCESS` | profile UUID, ASCII name (1..16 letters/digits/underscore, NUL-padded into `name[17]`), properties (≤64: name ≤64, value, optional signature ≤1024), then the Java 26.3 **session UUID** |
| 3 | `LP_SET_COMPRESSION` | threshold `VarInt >= -1` |
| 4 | `LP_CUSTOM_QUERY` | transaction `VarInt`, channel identifier, rest of the frame |
| 5 | `LP_COOKIE_REQUEST` | identifier key, nothing after it |

A structurally valid packet that is not modelled (any other ID, including a
negative one) returns `LP_UNSUPPORTED`. Identifiers follow Vanilla's
`ResourceLocation` characters (`namespace` `[a-z0-9_.-]`, path adds `/`, default
namespace `minecraft`). Profile-limit violations (empty/invalid or over-long name,
more than 64 properties, over-long property name or signature) are
`LP_BAD_PROFILE`; truncation, invalid UTF-8 and trailing bytes are
`LP_BAD_FRAME`.

### Status codes

`LP_OK 0`, `LP_NEED_MORE 1`, `LP_BAD_ARGUMENT -1`, `LP_OUTPUT_SMALL -2`,
`LP_BAD_FRAME -3`, `LP_BAD_PROFILE -4`, `LP_UNSUPPORTED -5`.

Every failure zeroes all `sizeof(struct lp_event)` bytes of the event —
padding included — so no stale field or padding byte can leak into a report.
`LP_OUTPUT_SMALL` means `scratch_capacity` cannot hold the whole body.

### Safety contract

`struct lp_event` is `#[repr(C)]` with the header's field order and size (72
bytes, `uuid` at 20, `name` at 36, `session` at 53, `should_authenticate` at 69). Callers guarantee valid,
aligned, non-overlapping buffers and correct lengths; null pointers and
out-of-range lengths are checked *before* any slice is formed and are
`LP_BAD_ARGUMENT`. Malformed input cannot panic: every read is bounds-checked,
and a panic aborts through the `#[panic_handler]` instead of unwinding into C.
A host-only `rust_eh_personality` shim (as in the status probe's FFI crate)
keeps a plain C linker working; the Switch target's abort-built core does not
need it.

## Tests

Unit tests exercise the C ABI functions directly (real `extern "C"` entry
points, not a re-implementation) and pin:

- independent wire vectors for the request (`localhost:25565`, `Notch`,
  protocol 777) and the offline UUIDs from published MD5 vectors;
- a `LoginFinished` with a property and a session UUID, its truncation at every
  length, trailing bytes and each profile limit;
- Python-`zlib`-generated static frames (`78 da …`), the Adler-32 trailer,
  trailing bytes inside the frame, threshold and declared-length corruption,
  zip-bomb shapes, coalesced frames and partial outer frames;
- event layout/offsets, output reset and untouched-buffer rules, and a panic
  sweep over two-byte inputs, every bit flip of two valid frames and
  pseudo-random buffers.

The crate was also checked by compiling and running a C program against the
normative header and the built archive (`_Static_assert`s on the layout and
enum values, request bytes, raw and compressed decode, corrupted trailers,
need-more and null-argument paths): all checks pass.
