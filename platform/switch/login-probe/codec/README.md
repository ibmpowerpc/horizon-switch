# Login, configuration and play codec (protocol 777)

Bounded, allocation-free Rust codec for the login, configuration and first play
packets of Java 26.3 (protocol 777), built as a `staticlib`
(`libcraftrustnx_login_codec.a`) for the CraftRustNX Switch login probe. The C
ABI is exactly [`../include/login_codec.h`](../include/login_codec.h),
[`../include/configuration_codec.h`](../include/configuration_codec.h) and
[`../include/play_codec.h`](../include/play_codec.h)
(read-only here); the C source-login worker, build scripts and hardware runs
live beside it.

It is a diagnostic only: it *describes* the exchange. It does not connect,
resolve names, authenticate, send packets, store cookies, download resource
packs, follow transfers, install registries or world data, accept a code of
conduct, enter play, log or allocate. Everything is `no_std`, caller-owned
buffers; the play decoder does its own IEEE position arithmetic but never
simulates a world.

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

## `craftrustnx_play_decode_body`

Decodes **one** complete play body the caller already unframed and inflated
through `craftrustnx_configuration_decode` in the play phase. The body starts
with the packet ID, is `1..=8388608` bytes and is in memory already: this entry
point never reads a frame prefix, inflates or buffers, so the crate keeps a
single framing implementation. A reply, when there is one, is an **unframed**
body of at most 1024 bytes for the caller to frame and cipher once.

`struct pl_state` is 320 bytes (alignment 8, `char dimension[257]`); a new
connection zero-initializes it. It changes only on success, written field by
field so the struct's tail padding is never touched. `struct pl_event` is 32
bytes; **every failure zeroes all of it** — padding included — and leaves both
`state` and `reply` exactly as they were. Cap checks, null checks and reply
capacity are all checked before any output or state write.

| Wire ID | Kind | Validation / reply |
|---|---|---|
| 50 `Login` | `PL_LOGIN` | full body: player id, hardcore, `0..=256` level identifiers, non-negative max players/radius/simulation distance, `CommonPlayerSpawnInfo` (`OptVarInt` previous game type 0..=4, `Opt<GlobalPos>` = identifier + 8 packed bytes), online and secure-chat flags; game mode 0..=3; dimension ≤256 bytes (a longer *valid* identifier is `PL_UNSUPPORTED`). Installs player id, dimension type, game mode and the NUL-padded dimension name, sets `login_seen`, clears `position_seen`. No reply. |
| 73 `PlayerPosition` | `PL_POSITION` | teleport `VarInt >= 0`, three `f64` position, three `f64` delta, `f32` yaw and pitch, big-endian relative flags; exact end; every number finite. Only `0x1ff` flag bits are accepted; deltas are never applied. X/Y, yaw or pitch relative bits need a prior absolute pose (`PL_UNSUPPORTED`). Resolves relative fields against the stored pose, stores raw IEEE bits, and replies `0` (AcceptTeleportation) + teleport + x/y/z `f64` + yaw/pitch `f32` — Java 26.3's full pose echo. |
| 45 `KeepAlive` | `PL_KEEP_ALIVE` | exactly one long; reply `28` + the same 8 bytes |
| 62 `Ping` | `PL_PING` | exactly one int; reply `45` + the same 4 bytes |
| 21 `CookieRequest` | `PL_COOKIE` | identifier key, nothing else; reply `21` + the same key + `false` (nothing stored) |
| 82 `ResourcePackPush` | `PL_RESOURCE_PACK` | UUID, URL, hash (≤40 units), required, optional prompt NBT (opaque but non-empty); reply `50` + UUID + `VarInt` 1 = **declined**; `flags` bit 0 mirrors `required`; nothing is downloaded |
| 11 `ChunkBatchFinished` | `PL_BATCH_END` | non-negative declared size; reply `11` + `f32` 1.0 (a receipt, not a load) |
| 12 `ChunkBatchStart` | `PL_BATCH_START` | empty body |
| 46 `LevelChunkWithLight` | `PL_CHUNK` | x and z `i32` big-endian, then at least one payload byte; `value`/`aux` are the column x/z; chunk and light data stay opaque |
| 32 `Disconnect` | `PL_DISCONNECT` | raw NBT payload, non-empty and opaque; not JSON-parsed |
| 120 `StartConfiguration` | `PL_RECONFIGURE` | empty body |
| 132 `Transfer` | `PL_TRANSFER` | non-empty host string and port `1..=65535`; reported, never followed |
| 84 `Respawn` | `PL_RESPAWN` | reported as an unsupported continuation; body opaque, nothing simulated |
| 0 `BundleDelimiter` | `PL_METADATA` | empty body |
| every other 1..=143 | `PL_METADATA` | counters only (`packet_id`, `payload_offset/length`); the payload stays opaque: no registry load, no world install, no "ready" claim |
| any other, incl. negative | — | `PL_UNSUPPORTED` (negative IDs are `PL_BAD_FRAME`) |

`payload_offset`/`payload_length` locate the bytes after the packet ID in the
body that was passed in. `value` is the teleport ID, the declared batch size or
the chunk column x; `aux` is the chunk column z. The decoder never fabricates
`PlayerLoaded`, `ClientTickEnd` or movement packets, and never claims a loaded
world.

### Safety contract

Callers guarantee valid, aligned, non-overlapping `body`/`state`/`reply`/`event`
regions and honest lengths. Null pointers and out-of-range lengths are checked
before any slice is formed (`PL_BAD_ARGUMENT`), and a reply that does not fit
1024 bytes or the caller's capacity is `PL_BAD_FRAME`/`PL_OUTPUT_SMALL` before a
byte of it is written. Malformed input cannot panic: every read is
bounds-checked and a panic aborts through the `#[panic_handler]`.

### Status codes

`PL_OK 0`, `PL_BAD_ARGUMENT -1`, `PL_OUTPUT_SMALL -2`, `PL_BAD_FRAME -3`,
`PL_UNSUPPORTED -5`. There is no "need more": the body handed in is complete.

## NBT structural scanner (`nbt`, Rust only)

`src/nbt.rs` is the groundwork for the dimension registry and chunk decode: a
bounded, allocation-free walk over Java's **network** NBT form, where a value is
one tag type byte plus that tag's payload and has **no** root name
(`NbtIo.readAnyTag`). It has no C ABI yet — no export and no header change — so
the C worker is unaffected.

```rust
skip_unnamed(input: &[u8], limits: Limits) -> Result<usize, NbtError>
RootView::parse(input: &[u8], limits: Limits) -> Result<RootView, NbtError>
```

`skip_unnamed` consumes exactly one root value and returns how many leading
bytes it used; a root `End` is one byte. Bytes after the value — the rest of a
packet — are never read or charged to the byte limit, so a caller can continue
with `&input[consumed..]`.

`RootView` borrows that same value and exposes only the root compound's **direct**
members: `fields()` iterates `Field { name, value }` pairs in wire order with
duplicates kept, and `find`/`find_i32`/`find_byte` look one up by raw name bytes
with vanilla's map semantics (**last** duplicate wins). A nested member with the
same name is never matched. `TagView` is `(type_id, payload)` with `payload()`
the exact payload bytes of that value (no type byte, no name) and typed getters
`as_byte`/`as_i32` (big-endian) — enough for `min_y`, `height` and
`has_skylight` without building a tree.

Wire rules followed: types 0..12 with `End` terminating a compound; big-endian
fixed-width scalars; strings and names `u16`-byte-length + bytes; lists a type
byte + `i32` count; byte/int/long arrays a signed `i32` count. Names and string
values stay **raw Java modified UTF-8 bytes**: overlong NUL (`c0 80`), unpaired
surrogates and NUL bytes are preserved, never decoded or validated. Floats and
doubles are skipped as bits. 26.3's mixed-list wrapper (a one-member compound
with an empty name inside a `List`) is structurally ordinary NBT and is not given
an extra type.

Limits (`Limits`, clamped to the hard caps by `Limits::clamped`): at most
**8 MiB** consumed, nesting depth **128** (root container at depth 0), and
**65,536** tags with list elements counted individually. Work is iterative over
an explicit 128-entry stack under 1 KiB of stack state, with no recursion and no
allocation; primitive lists and arrays are skipped with one checked
`count * width` bounds test instead of a per-element walk. `NbtError`
distinguishes `Truncated` (bytes missing), `Invalid` (unknown type, negative
length, a non-empty list without an element type, length overflow) and `Limit`
(a caller budget exhausted), each with the input offset.

## World data decoder (`world`, C ABI)

`src/world.rs` keeps the selected configuration registries and validates the
26.3 chunk and light wire data; `src/world_ffi.rs` is exactly
[`../include/world_codec.h`](../include/world_codec.h). It adds one export,
`craftrustnx_world_decode_body`, and the 528-byte `struct wc_state` (four
counters and 32 × `wc_dimension`, align 4) and 72-byte `struct wc_event`
(align 4). It reuses the NBT scanner and the wire `Reader`, allocates nothing,
recurses nowhere and keeps `unsafe` inside the FFI module.

```c
int32_t craftrustnx_world_decode_body(const uint8_t *body, uint32_t length,
    uint32_t phase, struct wc_state *state, const struct pl_state *play,
    struct wc_event *event);
```

**Configuration `RegistryData` (id 7).** Every entry is
`identifier, bool data, unnamed NBT`, and every NBT value is walked even for
opaque registries. Only `minecraft:dimension_type` and
`minecraft:worldgen/biome` are retained; any other registry (or a non-
`minecraft` namespace) is `WC_METADATA` with its entry count. A selected entry
without data is `WC_UNSUPPORTED`. Dimension roots must be compounds with direct
`min_y` (int), `height` (int) and `has_skylight` (byte): `min_y % 16 == 0`,
`0 < height <= 4096`, `height % 16 == 0` and `min_y + height` must not
overflow; the table index is the entry order and `has_skylight` is kept as the
zero-extended byte. More than 32 dimension or 65,536 biome entries is
`WC_UNSUPPORTED`. Identifiers are borrowed, never copied.

**Play `LevelChunkWithLight` (46).** x/z are big-endian ints and the packet
needs a selected `pl_state.dimension_type` plus a retained biome count. The
heightmap count is `<= 16`; each kind is non-negative and its long array count
must equal `storage_len(ceil_log2(height + 1), 256)`, with every packed value
`<= height` and the bits past value 256 treated as padding. The chunk buffer is
a `VarInt`-length byte slice that must end exactly after `height / 16` sections;
a section is two big-endian signed shorts (`0..=4096`) and two paletted
containers. Containers have **no long-count prefix**: one wire bit width, the
local palette when the strategy uses one, then exactly `storage_len(bits,
entries)` big-endian longs. Blocks: 0 is one value, 1..=4 normalise to four
bits with a local palette, 5..=8 keep the wire width with a local palette, and
9..=32 are global with the wire width. Biomes: 0 is one value, 1..=3 are local,
otherwise the width is `max(wire, ceil_log2(biome_count))`, at most 32. Palette
counts are `1..=2^bits`, palette ids are non-negative, every cell's local index
is inside the palette, global block ids are `<= i32::MAX` and every biome id is
below the retained count. Block-state names are not installed, so a global block
palette has no registry-width cross-check. Then follow `<= 4096` block entities
(packed xz byte, short y, non-negative `VarInt` type, bounded unnamed NBT with
`End` allowed) whose y must be inside the dimension, and the light data.

**Light.** Four `BitSet`s use `ByteBufCodecs.BIT_SET`: a `VarInt` byte length
of at most `ceil((sections + 2) / 8)`, then Java `BitSet.toByteArray` bytes
(low bit and low byte first), with no bit at or past `sections + 2`. Present and empty masks must not overlap for the same kind.
The sky and block layer counts must equal the masks' population counts, and each
layer is a `VarInt` length of exactly 2048 bytes. The packet ends exactly.
`LightUpdate` (49) is x/z `VarInt`s plus the same light data.

**Sample.** When `pl_state` has an acknowledged pose, the chunk packet resolves
the block **below** it (finite `f64` bits, a checked integer floor without
`std`/`libm`, checked `y - 1`): only for the matching chunk column and inside
the dimension does it report `sample_x/y/z`, the raw `state_id` and the raw
`biome_id` at the local coordinates. There are no block or biome names.
Otherwise `sample_valid` is 0. `wc_event` reports the chunk x/z, sections,
heightmap and block-entity counts, provided sky/block layer counts and the sums
of the section `non_empty` and `fluids` headers.

**Failure.** State is built aside as a 528-byte copy and committed only on
success; every `wc_event` byte is zero on failure. Bad pointers, lengths,
phase or nulls are `WC_BAD_ARGUMENT`, a malformed body is `WC_BAD_FRAME`, a
valid but unsupported registry/palette/entity cap is `WC_UNSUPPORTED`, and an
exhausted NBT budget is `WC_LIMIT`. Other packet ids stay opaque metadata.

The decoder adds 20 tests (103 with the existing 83): hand-built dimension and
biome registries, opaque-registry NBT walking and its tag-budget `WC_LIMIT`,
dimension field/bounds/overflow validation and the unsupported-table path, all
block and biome palette modes (including the no-prefix layout and palette bounds),
the two signed shorts, heightmap width/value/padding/count rules, light mask
range/overlap/layer-count/2048-byte lengths, positive and negative sample
coordinates, no-pose/other-column/outside/NaN samples, block-entity bounds,
`LightUpdate`, every strict prefix of a chunk and registry body, VarInt overflow,
trailing bytes, the ABI statuses and the transactional state/event bytes.

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
  Python-`zlib`-verified stored-block streams at thresholds 0, 1 and above;
- play vectors through the ABI: a complete `Login` with every optional field
  (level list, previous game type, last death, both endings) and its installed
  state with the tail padding untouched, a long dimension (`PL_UNSUPPORTED`),
  out-of-range counts/distance/game mode/previous game mode, truncation and
  trailing bytes, an absolute `PlayerPosition` acknowledged with the exact
  teleport/position/yaw/pitch bytes, relative x/yaw added to the prior pose,
  delta-only flags, a relative first teleport (`PL_UNSUPPORTED`), NaN/Inf and
  overflowing relative sums, illegal flag bits, a short reply (`PL_OUTPUT_SMALL`
  with the state unchanged), the keepalive/ping/batch/cookie/pack replies, the
  chunk header and bundle delimiter, disconnect/reconfigure/transfer/respawn
  events, opaque known IDs, an unknown ID and a negative ID, null/oversized
  arguments and a malformed-input sweep;
- the NBT scanner: hand-assembled network vectors for all twelve tag types
  (including an empty list that names no element type), nested compounds and
  lists, a root `End`, trailing packet bytes, every strict prefix of four
  vectors as `Truncated`, negative and `i32::MAX` lengths, unknown types and
  bad list headers, the byte/depth/tag caps at their exact boundaries, duplicate
  and nested field names, big-endian `min_y`/`height` and byte `has_skylight`,
  raw `c0 80`/surrogate/NUL name and string bytes, a mixed-list wrapper, and a
  pseudo-random plus bit-flip sweep that must not panic.

The crate was also checked by compiling and running a C program against the
normative header and the built archive (`_Static_assert`s on the layout and
enum values, request bytes, raw and compressed decode, corrupted trailers,
need-more and null-argument paths): all checks pass.

The 103 Rust tests (`cargo test --offline --locked`: the 64 below the NBT
module, 19 for it and 20 for the world decoder) pass; the release archive
exports exactly `craftrustnx_login_start`, `craftrustnx_login_key`,
`craftrustnx_login_decode`, `craftrustnx_configuration_decode`,
`craftrustnx_configuration_frame`, `craftrustnx_play_decode_body` and
`craftrustnx_world_decode_body`.

The native play-session caller additionally validates login ordering, chunk-batch
boundaries/counts and bundle completion. Pitch is clamped to [-90,90] after
relative resolution, matching the inspected `position_packet_pitch` function.
Optional previous game type is validated as raw code 0..4 before subtraction;
malformed signed values, including i32::MIN, cannot overflow a debug decoder.
Chunk credit 1.0 is a probe flow-control choice, not a measured load rate or a
claim about vanilla's default. No PlayerLoaded or world simulation is generated.

## Normalized column copy (Rust + C ABI)

`column.rs` / `column_ffi.rs` implement `craftrustnx_column_copy` from
`include/column_codec.h`. The existing world parser now shares a bounded cell
sink for sampling, validation and full output; it is not a second wire parser.
The first pass validates the complete play46 packet (including heightmaps,
NBT and light) without output writes. After capacity checks, the second pass
normalizes single/local/global palettes into caller-owned u32 arrays. Both
passes read identical immutable inputs. No allocation, SDK call or persistence.

The 24-byte `col_info` reports chunk x/z, min Y, sections and used element counts.
Output order is bottom-up sections, then Y/Z/X: 4096 block states and 64 biome
cells per section. Only used prefixes are written. Statuses 0/-1/-2/-3/-5/-6
mean success/argument/short output/malformed/unsupported/NBT budget. On failure
info is zero, arrays and input states unchanged. Pointer validity, alignment and
non-overlap are caller contracts. Maximum arrays are 256 sections; only raw IDs
are produced, with no air, opacity, block name or readiness semantics.

## `craftrustnx_column_copy`

Validates one complete play id 46 body (same structural checks as the world
decoder, including heightmaps, NBT and light) and copies normalized block-state
and biome ids into caller-owned `u32` arrays. Only the first `sections * 4096`/
`sections * 64` elements are written; tail is untouched. Statuses
`COL_OK 0`, `COL_BAD_ARGUMENT -1`, `COL_OUTPUT_SMALL -2`, `COL_BAD_FRAME -3`,
`COL_UNSUPPORTED -5`, `COL_LIMIT -6`.

Parent verification: 109 tests pass (the existing 103 plus six column-copy
tests), fmt and clippy `-D warnings` pass. New tests check full mixed-palette
columns at negative coordinates, cell order, malformed light tails, capacities,
ABI/null bounds and invalid palettes/trailing bytes. The parent corrected an
additional overlong local-palette test buffer so its valid prefix is well formed.

## Bounded column snapshot format (`snapshot`)

`src/snapshot.rs` and `src/snapshot_ffi.rs` implement the **CTCOL001** binary
format for normalized column data, for later Rust meshing. Pure Rust and C ABI
entry points encode and decode complete snapshots from raw u32 state/biome IDs.

### Format specification

- **Magic**: `"CTCOL001"` (8 bytes)
- **Protocol**: `777` (u32 LE)
- **Format**: `1` (u32 LE)
- **chunk_x**: `i32` LE
- **chunk_z**: `i32` LE
- **min_y**: `i32` LE (must be multiple of 16)
- **sections**: `u32` LE (1..=256)
- **blocks_written**: `u32` LE (sections * 4096)
- **biomes_written**: `u32` LE (sections * 64)
- **reserved**: 8 bytes, all zero
- **Data**: exactly `sections * 4096` LE `u32` block IDs, then `sections * 64` LE
  `u32` biome IDs

Total header: **48 bytes**. Data: `sections * (4096 + 64) * 4` bytes.
Maximum file size: `48 + 256 * (4096 + 64) * 4 = 4,259,888` bytes.

### Rust functions

- `encode_snapshot(chunk_x, chunk_z, min_y, sections, blocks, biomes, out) -> Result<usize, i32>`
- `decode_snapshot(data, blocks, biomes) -> Result<(i32, i32, i32, u32, u32, u32), i32>`
- `header_size(sections) -> usize`
- `MAX_FILE_SIZE`, `MAX_BLOCKS`, `MAX_BIOMES`, `MAX_SECTIONS`, `PROTOCOL`, `SNAP_OK`, `SNAP_BAD_ARGUMENT`, `SNAP_OUTPUT_SMALL`, `SNAP_BAD_FRAME`

All functions validate bounds before any write. `min_y` must be a multiple of 16,
and `min_y + sections * 16` must fit `i32`. Headers validate reserved bytes are
zero. On failure, outputs are untouched; status codes match the column codec.

### C ABI

[`../include/column_snapshot.h`](../include/column_snapshot.h) exports:

```c
int32_t craftrustnx_snapshot_decode_header(const uint8_t *data, uint32_t length, struct snap_info *info);
int32_t craftrustnx_snapshot_encode(int32_t chunk_x, int32_t chunk_z, int32_t min_y,
    uint32_t sections, const uint32_t *blocks, uint32_t blocks_written,
    const uint32_t *biomes, uint32_t biomes_written, uint8_t *out, uint32_t capacity, uint32_t *written);
int32_t craftrustnx_snapshot_decode(const uint8_t *data, uint32_t length,
    uint32_t *blocks, uint32_t block_capacity,
    uint32_t *biomes, uint32_t biome_capacity, struct snap_info *info);
uint32_t craftrustnx_snapshot_max_file_size(void);
```

`struct snap_info` mirrors `struct col_info` (24 bytes, same field order).
Inputs are validated *before* any slice is formed; null pointers and out-of-range
lengths are `SNAP_BAD_ARGUMENT`. Encoder counts must exactly match the section
count; invalid section/min-Y inputs are also `SNAP_BAD_ARGUMENT`. Malformed or
truncated nonempty files are `SNAP_BAD_FRAME`; full decode rejects trailing
bytes. Header-only decode accepts the 48-byte header without a body. Decoder
array capacities may exceed the required count up to `SNAP_MAX_*`; smaller
capacities return `SNAP_OUTPUT_SMALL`. On failure, `info`/`written` is zeroed
and output arrays/bytes are untouched.

### Tests

Rust tests exercise the C ABI and pure Rust functions:

- encode/decode roundtrip at negative chunk coordinates and min_y
- min_y bounds (multiple of 16, i32 overflow check)
- reserved bytes must be zero, wrong magic/protocol/format return `-3`
- empty input returns `-1`, truncated/malformed files `-3`, short output arrays `-2`
- exact array counts, complete file length and transactional rejection before writes
- output tails (beyond written extent) remain untouched
- header size calculation matches the spec

All tests pass offline (`cargo test --offline --locked`). No allocation, std,
or unsafe outside FFI. No network or device required.
