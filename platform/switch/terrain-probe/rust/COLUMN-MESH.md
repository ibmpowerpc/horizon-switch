# Allocation-free section mesher (0.12)

Rust side of the column-mesh probe: meshes one already captured `CTCOL001`
section on the CPU against a compact `CTBANK01` model bank. No SDK, filesystem,
allocation, RNG/jitter, occlusion culling, lighting, tint/colour guessing or
cube fallback. [column_mesh.h](../../texture-probe/include/column_mesh.h) is the normative ABI and format
contract; this note only records how the implementation reads it and what the
tests cover.

Module: `src/column_mesh.rs`, behind the `column-mesh` Cargo feature (which
enables `textured`). It reuses `textured::{TextureVertex, Draw, SceneInfo}` for
the 24-byte vertex / 16-byte draw record and the login probe's shared
`CTCOL001` codec for capture-header validation, so the snapshot rules do not
fork. The 4096 selected cells are read in place from the capture; nothing
allocates a whole column.

## Inputs

- Capture: a complete console snapshot. The header must decode through the
  shared codec (magic, protocol 777, format 1, reserved zero, section count and
  `blocks_written`/`biomes_written` exactly `sections*4096`/`sections*64`, `min_y`
  a multiple of 16 that fits the section range) and `capture_len` must equal the
  exact file size for its section count (`48 + sections*(4096 + 64)*4`,
  at most 4,259,888 bytes). `section` must be below `sections`.
- Bank: `CTBANK01`, at most 16 MiB, exact length
  `64 + states*32 + template_vertices*24 + atlas_bytes`. Header, state stride 32,
  vertex stride 24, zero words, atlas dimensions (power of two, 1..1024) and
  `atlas_bytes == width*height*4` are checked. Records must be strictly
  ascending unique raw IDs with a zero reserved word; each of the three mode
  ranges must start and end on whole six-vertex quads and stay inside the
  template array. Ranges may alias within and across records. Every template
  vertex must be finite, `|axis| <= 1024` and UV inside `[0, 1]`.
- Raw IDs carry no air/solid meaning. ID 0 is looked up like every other ID and
  a missing record is unsupported, never a cube or empty fallback.

## Output

- Vertices are whole six-vertex quads, grouped mode 0 (solid), 1 (cutout),
  2 (translucent); every non-empty group becomes one draw record with
  `first`/`count` covering its range and `reserved = 0`.
- Inside a group, cells are emitted in capture order (`index = y*256+z*16+x`, so
  `x = index%16`, `z = index/16%16`, `y = index/256`) and each cell's template
  range keeps bank order. A vertex is `template.position + [x, y, z]` with the
  template UV and RGBA word copied unchanged.
- `cm_info.scene.mesh` holds the exact vertex count and bounds over the emitted
  vertices; `scene.width`/`height`/`atlas_bytes` repeat the bank atlas and
  `scene.atlas_offset` is the atlas byte offset inside the bank (so copying it
  stays a native-shim operation). `section_min_y = min_y + section*16`,
  `section_index = section`, and `cells_with_geometry`/`empty_cells` count each
  of the 4096 cells once, a cell being empty exactly when all three ranges are
  empty.

## Limits and statuses

Absolute coordinates above 1024, non-finite positions, more than 262,140
vertices, an atlas offset/size that leaves the bank, or an entirely collapsed
bounding box (no axis with `min < max`) are refused. Status precedence:

1. `-1` bad pointer or argument shape: null `info`/inputs, a query that is not
   exactly both outputs null with both capacities zero, or a capacity field
   above 262,140 vertices / 256 draws. Lengths are checked before any slice, so
   a forged `size_t` never widens the read.
2. `-3` malformed capture or bank, including any length/format/range/UV/reserved
   violation, `section` out of range, and an over-limit result.
3. `-5` a selected cell's raw ID has no bank record, or the section produces no
   vertices at all.
4. `-2` (build only) capacity below the exact vertex or draw count.

Everything is validated in a plan pass before the first output write. `info` is
zeroed on every failure, vertex/draw buffers are untouched on every failure, and
on success only the emitted elements are written, leaving tails untouched. The
write pass re-reads the same validated inputs (immutable for the call) in mode
order and cannot fail under the documented safety contract.

## Verify

```sh
cargo test --offline --locked --features column-mesh --jobs 1 \
  --manifest-path platform/switch/terrain-probe/rust/Cargo.toml \
  --target-dir platform/switch/texture-probe/build/host-target
cargo fmt --check
cargo clippy --offline --locked --features column-mesh --jobs 1 --all-targets \
  --manifest-path platform/switch/terrain-probe/rust/Cargo.toml \
  --target-dir platform/switch/texture-probe/build/host-target -- -D warnings
cargo build --offline --locked --release --features column-mesh --jobs 1 \
  --manifest-path platform/switch/terrain-probe/rust/Cargo.toml \
  --target-dir platform/switch/texture-probe/build/host-target
```

The unit tests use hand-written capture and bank bytes only: query/build
metadata and ABI layout, Y/Z/X translation, group/template order, aliased and
high raw IDs (0 included), empty mode groups, section selection and `min_y`,
missing/empty sections, malformed headers/records/ranges/vertices/atlas, forged
lengths, capacity/null/query mistakes, tail preservation and every failure path
leaving outputs untouched. The real captured-column comparison against the
0.11 `CTTEX001` scene, the native shim, the host bank writer and the `.nro`
build are owned outside this module.
