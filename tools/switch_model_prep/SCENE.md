# Diagnostic scene export (`export_probe_scene`)

`examples/export_probe_scene.rs` writes a small **untextured, static** scene of
real baked Minecraft block models as the Switch terrain probe's `CTMESH01`
mesh. It exists to give probe0.8 real depth/vertex/camera work: 86 upstream
quads, 172 triangles, 516 vertices in one indexed-free triangle list.

**It is a diagnostic fixture, not a renderer result.** There are no textures,
tints, biome colors, block or sky light, transparency, fluids, AO or vanilla
random parity. Colors are flat opaque per-sample values dimmed per face
direction. Weighted model options are resolved by an explicit first-option
choice for this diagnostic only; no RNG is consumed and nothing here claims how
vanilla would sample.

Positions are the upstream `ModelQuad` positions plus a 3x3 grid translation
(spacing 2.0, centered on the origin), triangulated as `[0,1,2, 0,2,3]`. Every
vertex comes from `craftrustnx_model_prep::bake_requested`; there is no
invented cube and no solid fallback. A sample whose state bakes no geometry
fails the export instead of being replaced.

## Inputs (all optional, all bounded, all local)

| Flag | Default |
|---|---|
| `--facts FILE` | `.local/switch-state-facts/26.3/state_facts.json` |
| `--selection FILE` | `.local/switch-models/26.3/model-selection.json` |
| `--assets DIR` | `.local/switch-assets/26.3` |
| `--output FILE` | `.local/switch-models/26.3/terrain-probe-mesh.bin` |

A single positional argument is accepted as `--output`. The report is written
beside the output, replacing the `.bin` suffix with `.json`; for the default it
is `.local/switch-models/26.3/terrain-probe-mesh.json`.

Validation before any baking:

- facts, plan and manifest pin `26.3`, protocol `777`, client SHA1
  `e877b6a07acd633fb3bb475002175cec036e7b87`;
- `plan.facts_sha256` equals the SHA256 of the facts file and
  `plan.asset_manifest_sha256` equals the verified manifest hash;
- every manifest-listed asset's size and SHA256, unsafe paths, symlinks, case
  collisions and the aggregate byte count;
- contiguous, unique state ids in both facts and plan, canonical string
  properties, and plan rows exactly covering the facts ids;
- bytes: 12 MiB facts, 12 MiB plan, 8 MiB manifest, 2 MiB per asset, 8 MiB of
  assets in total.

The tool never opens a game jar, JVM, network socket or device and runs no git
command.

## Selected states (by name and complete property tuple, never by assumed id)

| Grid | Block | Properties |
|---|---|---|
| 0 | `minecraft:stone` | — |
| 1 | `minecraft:oak_stairs` | facing north, half bottom, shape straight, waterlogged false |
| 2 | `minecraft:cobblestone_wall` | up true, north low, east none, south none, west none, waterlogged false |
| 3 | `minecraft:oak_fence` | north true, east true, south false, west false, waterlogged false |
| 4 | `minecraft:oak_slab` | type bottom, waterlogged false |
| 5 | `minecraft:grass_block` | snowy false |
| 6 | `minecraft:dandelion` | — |
| 7 | `minecraft:glass` | — |
| 8 | `minecraft:torch` | — |

Each must match exactly one facts state. Zero or several matches fail the
export with the block name and properties; no state is silently substituted.
All parts of the matching plan row are kept; each part contributes its first
weighted option. Requests are deduplicated before one `bake_requested` call.

## Binary format

Exactly what `platform/switch/terrain-probe/include/terrain_app.h` decodes:

```text
offset 0   ASCII "CTMESH01"            (8 bytes)
offset 8   LE u32 protocol 777
offset 12  LE u32 format 1
offset 16  LE u32 vertex count (multiple of 3, <= TP_MAX_VERTICES 262140)
offset 20  LE u32 reserved 0
offset 24  six LE f32: min XYZ, max XYZ over all vertices
offset 48  count * 16-byte vertices: 3 LE f32 position + LE u32 RGBA
```

`RGBA` is memory byte order R,G,B,A (low byte R) with alpha 255. Total length is
exactly `48 + 16 * count`. Bounds cover every vertex, coordinates must be
finite with `|x| <= 1024`, and the vertex count must be a non-zero multiple of
three.

Both the mesh and its private JSON report are published with a sibling temp
file and a no-replace hard link; existing files are refused, not overwritten.
Only the tool's own temporary files are removed.

## Real export

```sh
cargo run --offline --locked --jobs 1 \
  --manifest-path tools/switch_model_prep/Cargo.toml \
  --target-dir tools/switch_model_prep/build/host-target \
  --example export_probe_scene
```

Verified local run against the pinned inputs: 9 samples, 12 unique baked
requests, 86 quads, 172 triangles, 516 vertices, mesh 8304 bytes, SHA256
`594e84b5dded66d7e0b0216ebc115b0298823f24b6a6ee587126f271eacfad87`, bounds
`(-2, 0, -2)..(3, 1, 3)`. This is a mesh-byte and sample-count result, not a
gameplay, rendering, correctness-of-appearance or performance proof.

The report records the state ids, properties, part/option counts, model
signatures, aggregate counts, mesh hash and bounds. It stays under ignored
`.local`; stdout carries only aggregate counts and the mesh hash, never model
names, facts or quads. No model or texture payload is checked into the
repository.

## Tests

```sh
cargo test --offline --locked --jobs 1 \
  --manifest-path tools/switch_model_prep/Cargo.toml \
  --target-dir tools/switch_model_prep/build/host-target \
  --example export_probe_scene
```

Seven independent tests cover the exact 48-byte header and 16-byte vertex
layout with the `[0,1,2,0,2,3]` triangle order and known bounds/colors, refusal
of empty/ragged/non-finite/oversized meshes, refusal of empty samples instead
of a fallback, exact-one state matching, empty parts and missing model options,
no-replace publication, and manifest corruption detection. They use synthetic
meshes and a synthetic manifest, not the local Minecraft install.
