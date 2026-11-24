# Rust Column Mesh Probe 0.12.0

This separate prototype moves section mesh construction onto the Switch.
Host Rust still prepares version-pinned model templates and a static texture
atlas from the player's Java 26.3 resources. It does not expand those templates
into every cell of the device's scene. Allocation-free Rust on Switch reads a
saved raw-ID column and model bank, finds each selected cell's model, translates
the template vertices and produces the GPU mesh. The existing native bridge
handles file/pad and SDK resource I/O.

It is a saved-section test. Network receipt and rendering remain separate,
with no world light, biome tint, fluids, animation, block entities, live updates,
collision or gameplay. The first model alternative is the same explicit static
policy as 0.11; no vanilla RNG/jitter or occlusion-culling parity is claimed.

## Prepare and build

Use the private capture already checked by Column Export Probe. Generate a bank
for explicit section index 8 (Y64..79 for this capture):

```sh
cargo run --manifest-path tools/switch_model_prep/Cargo.toml \
  --offline --locked --jobs 1 --target-dir tools/switch_model_prep/build/host-target \
  --example export_section_scene -- \
  --capture .local/switch-captures/2026-11-24/capture-0000/column.ctcol \
  --section 8 --bank-output .local/switch-models/26.3/section-model-bank.bin
cp .local/switch-captures/2026-11-24/capture-0000/column.ctcol \
  .local/switch-models/26.3/column-mesh-capture.ctcol
bash scripts/build-switch.sh docker column-mesh
```

Keep all generated data private under ignored `.local/` and `dist/`. The tool
refuses to replace its bank/report; the explicit capture copy must be the same
file named in that private report. `--bank-output` leaves the earlier 0.11 scene
intact and records its reference hash. The current bank contains only the 17
state IDs in the selected section, including an explicit empty model for its
invisible state. It is not yet a complete 26.3 state/model resource pack.
Unknown states fail without a cube or guessed-air fallback. Geometry templates
are deduplicated, preserving all multipart parts and per-mode quad order.

Copy these files into `sdmc:/switch/craftrustnx/`:

- `craftrustnx-column-mesh-probe.nro` (or the versioned 0.12.0 copy)
- `section-model-bank.bin`
- `column-mesh-capture.ctcol`
- `craftrustnx-column-mesh-notices.txt`

The probe loads the bank and capture; it does not read
`captured-section-scene.bin`. The Rust query validates inputs and calculates
counts/bounds before native code allocates output storage. Rust then builds
transactionally into that caller-owned storage. Only the selected 4096 raw
cells are read directly from the column bytes; no expanded whole-column array
or Rust allocator is needed.

## Hardware check

Launch **CraftRustNX Rust Column Mesh Probe**. Before entering 3D, check:

- 30,816 vertices, 2 draws, atlas 128×128;
- section 8 / Y64, modeled 876, empty 3220;
- vertex fingerprint **`b7ac981c4e14246d`**.

The fingerprint is FNV-1a over generated vertex bytes for comparison with the
0.11 reference, not a security checksum. The screen also reports elapsed time
for Rust query + allocation + build, excluding file reads and the fingerprint
loop. No timing is assumed before the device reports it.

Press A to enter 3D. Orbit, background, reset, right stick, zoom and exit use the
same controls as 0.11. Check that the captured ground/tree/plant section looks
the same. Opaque/cutout geometry remains on the GPU while the camera moves.
This visual/fingerprint check is separate from host and SDK verification.

## Format and host checks

The public ABI and exact `CTBANK01` binary layout are in
[column_mesh.h](include/column_mesh.h). The bank holds sorted state records,
local per-mode vertex templates and the atlas. `CTCOL001` is unchanged. Header,
length, ranges, coordinates, UV and output capacities must validate before
output writes. Failure clears info and preserves output arrays; query and
build produce the same metadata. This probe reports a missing model or empty
section as unsupported rather than publishing an incomplete mesh.

```sh
bash platform/switch/texture-probe/tests/run-column-mesh-host.sh
```

This independent C caller links the real Rust mesher, reads the real capture
and bank, and compares every generated vertex/draw/atlas byte with the previous
0.11 scene. It also checks query/build agreement, guards and transactional
rejection. ASan/UBSan instrument C; the Rust static library is not instrumented.
Synthetic Rust fixtures independently cover the binary format and failure
boundaries. Host validation and successful SDK linking do not establish a
hardware visual or performance result.
