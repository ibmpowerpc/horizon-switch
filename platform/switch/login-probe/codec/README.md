# Login and configuration codec (protocol 777)

Bounded, allocation-free Rust codec for the login and configuration states of
Java 26.3 (protocol 777), built as a `staticlib`
(`libcraftrustnx_login_codec.a`) for the CraftRustNX Switch login probe. The C
ABI is exactly [`../include/login_codec.h`](../include/login_codec.h) and
[`../include/configuration_codec.h`](../include/configuration_codec.h)
(read-only here); the C source-login worker, build scripts and hardware runs
live beside it.

It is a diagnostic only: it *describes* the exchange. It does not connect,
resolve names, authenticate, send packets, store cookies, download resource
packs, follow transfers, install registries or world data, accept a code of
conduct, enter play, log or allocate. Everything is `no_std`, no floats,
caller-owned buffers.

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

## `craftrustnx_login_key`

Writes one complete TCP frame: `EncryptionResponse` (packet ID **1**) with the
body `VarInt` secret length + encrypted secret + `VarInt` challenge length +
encrypted challenge. Both ciphertexts must be the **same length, 128 or 256
bytes** (the RSA key sizes `login_crypto.h` accepts); unequal lengths, any other
length, a null pointer and `compression < -1` are `LP_BAD_ARGUMENT`. The frame
carries no reply beyond this packet: the cipher switch that follows is the
transport worker's job.

`compression` is the threshold in force, exactly as `craftrustnx_login_decode`
takes it:

- `-1`: raw outer frame, `VarInt(body)` + body;
- `>= 0`: the frame carries the envelope, and only a **data length 0** envelope
  is written, which Vanilla accepts only when the body is *below* the
  threshold. At or above it a real server needs a compressed send, so the call
  returns `LP_UNSUPPORTED`: this probe never compresses what it sends and has no
  deflate path.

On every error `*written == 0` and `out` is untouched; the complete frame length
is checked against `capacity` before anything is serialized.

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
| 1 | `LP_ENCRYPTION_REQUEST` (`should_authenticate` exposed as 0/1, with the DER public key and challenge regions) | server id (≤20 UTF-16 units), key byte array, challenge byte array, bool; the attempt stops here whatever the bool says. `public_key_offset/length` and `challenge_offset/length` locate the two byte arrays inside `scratch` (the whole body, packet ID included); they are validated, not capped beyond the body |
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

`struct lp_event` is `#[repr(C)]` with the header's field order and size (88
bytes: `uuid` at 20, `name` at 36, `session` at 53, `should_authenticate` at 69,
`public_key_offset/length` at 72/76, `challenge_offset/length` at 80/84).
Callers guarantee valid,
aligned, non-overlapping buffers and correct lengths; null pointers and
out-of-range lengths are checked *before* any slice is formed and are
`LP_BAD_ARGUMENT`. Malformed input cannot panic: every read is bounds-checked,
and a panic aborts through the `#[panic_handler]` instead of unwinding into C.
A host-only `rust_eh_personality` shim (as in the status probe's FFI crate)
keeps a plain C linker working; the Switch target's abort-built core does not
need it.

## `craftrustnx_configuration_decode`

Decodes **one** complete clientbound frame from `data` into `scratch` and fills
`struct cf_event` (32 bytes: `kind`, `consumed`, `body_length`, `packet_id`,
`payload_offset`, `payload_length`, `reply_length`, `flags`). It shares the
login codec's frame reader, envelope and inflater with configuration limits
instead of duplicating packet framing:

- outer frame body `1..=2097151` (`CF_MAX_FRAME`) plus at most a five-byte
  prefix, so `CF_MAX_WIRE` 2097156 on the wire;
- an envelope may declare `0..=8388608` (`CF_MAX_BODY`) bytes of output; an
  oversized prefix or declared length is `CF_BAD_FRAME` from that length alone,
  **before** any buffering wait or inflation;
- a compressed stream must inflate to exactly its declared length, consume its
  frame exactly and pass the Adler-32 trailer (strict zlib, `miniz_oxide` core).

`compression` is `-1` (raw frames) or the announced threshold; `phase` is `0`
(configuration) or `1` (play). Only one frame is consumed, so a coalesced tail
stays for the caller; an incomplete outer frame (including an incomplete prefix)
is `CF_NEED_MORE`, a complete but malformed one is `CF_BAD_FRAME`.

Replies are **unframed bodies** for the caller to frame and cipher; every reply
length is checked against `CF_REPLY_BODY` 1024 and then against `reply_capacity`
before a byte is written (`CF_BAD_FRAME` / `CF_OUTPUT_SMALL`), so no partial
reply is ever produced.

| Wire ID | Kind | Validation / reply |
|---|---|---|
| 3 `FinishConfiguration` | `CF_FINISH` | empty body; reply `[3]` |
| 4 `KeepAlive` | `CF_KEEP_ALIVE` | exactly one long; reply `[4]` + the same 8 bytes |
| 5 `Ping` | `CF_PING` | exactly one int; reply `[5]` + the same 4 bytes |
| 15 `SelectKnownPacks` | `CF_KNOWN_PACKS` | `0..=256` triples of strings (≤32767 UTF-16 units each), full body; reply is always `[7,0]`: this probe has no packs and never claims resources |
| 0 `CookieRequest` | `CF_COOKIE_REQUEST` | identifier key, nothing else; reply `[1]` + key + `false` (no payload, nothing stored) |
| 9 `ResourcePackPush` | `CF_RESOURCE_PACK` | UUID, URL, hash (≤40 units), required, optional prompt NBT (opaque but must carry bytes); reply `[6]` + UUID + `VarInt` 1 = **declined**; `flags` bit 0 mirrors `required`; nothing is downloaded |
| 2 `Disconnect` | `CF_DISCONNECT` | raw NBT payload, bounded and opaque; not JSON-parsed and no text claimed |
| 20 `CodeOfConduct` | `CF_USER_ACTION` | one UTF-8 string; reported, never accepted |
| 12 `Transfer` | `CF_TRANSFER` | host string and port; reported, never followed |
| 1, 6, 7, 8, 10, 11, 13, 14, 16, 17, 18, 19 | `CF_METADATA` | counters only (`packet_id`, `body_length`, `payload_offset/length`); payload stays opaque: no NBT validation, no registry or world install, no "ready" claim |
| any other, incl. negative | — | `CF_UNSUPPORTED` |

In the play phase only the non-negative packet ID is read: the body is reported
as `CF_PLAY_PACKET` with bounded opaque counters and no gameplay schema or world
claim.

`payload_offset`/`payload_length` locate the bytes after the packet ID inside
`scratch` (the whole body, packet ID included, starts at 0).

## `craftrustnx_configuration_frame`

Frames one client body of `1..=1024` bytes. `-1` writes a raw frame; `>= 0`
writes the compression envelope, raw when the body is below the threshold and
compressed at or above it. There is no compressor dependency: the compressed
form is a single **stored DEFLATE block** inside a zlib wrapper (`78 01`) with
an exact Adler-32 trailer, so thresholds 0 and 1 work for small client
acknowledgements and every inflater, Python's `zlib` included, accepts it. The
complete wire length is checked against `capacity` before serializing:
failures give `CF_OUTPUT_SMALL` (or `CF_BAD_ARGUMENT` for an empty/oversized
body or `compression < -1`), with `*written == 0` and `out` untouched. Ciphering
is the caller's job.

### Status codes

`CF_OK 0`, `CF_NEED_MORE 1`, `CF_BAD_ARGUMENT -1`, `CF_OUTPUT_SMALL -2`,
`CF_BAD_FRAME -3`, `CF_UNSUPPORTED -5`. Every failure zeroes all 32 bytes of
`cf_event` — padding included.

## Tests

Unit tests exercise the C ABI functions directly (real `extern "C"` entry
points, not a re-implementation) and pin:

- independent wire vectors for the request (`localhost:25565`, `Notch`,
  protocol 777) and the offline UUIDs from published MD5 vectors;
- independent `EncryptionResponse` vectors for both 128- and 256-byte
  ciphertexts (deterministic test bytes, not a real key), raw and envelope
  framing, the at/above-threshold `LP_UNSUPPORTED` path, short capacity,
  unequal lengths and null arguments, all through the C ABI;
- a `Hello` whose key and challenge regions are sliced back out of `scratch`,
  with `should_authenticate` checked at both 0 and 1;
- a `LoginFinished` with a property and a session UUID, its truncation at every
  length, trailing bytes and each profile limit;
- Python-`zlib`-generated static frames (`78 da …`), the Adler-32 trailer,
  trailing bytes inside the frame, threshold and declared-length corruption,
  zip-bomb shapes, coalesced frames and partial outer frames;
- event layout/offsets, output reset and untouched-buffer rules, and a panic
  sweep over two-byte inputs, every bit flip of two valid frames and
  pseudo-random buffers;
- configuration control vectors through the ABI: finish, keepalive and pong
  bytes, always-empty known packs (including a 256-pack offer), cookie reply
  with no payload, declined resource pack with the required bit, code of
  conduct and transfer (reported, not followed), disconnect bytes;
- the 8 MiB body and 2 MiB frame caps rejected from the prefix/envelope before
  buffering or inflation, opaque metadata accepted but not parsed, play IDs
  reported, reply cap/capacity, and frame vectors byte-identical to
  Python-`zlib`-verified stored-block streams at thresholds 0, 1 and above.

The crate was also checked by compiling and running a C program against the
normative header and the built archive (`_Static_assert`s on the layout and
enum values, request bytes, raw and compressed decode, corrupted trailers,
need-more and null-argument paths): all checks pass.

The 50 Rust tests (`cargo test --offline --locked`) pass; the release archive
exports exactly `craftrustnx_login_start`, `craftrustnx_login_key`,
`craftrustnx_login_decode`, `craftrustnx_configuration_decode` and
`craftrustnx_configuration_frame`.
