# From network columns to terrain rendering

Source review against the current fork, 2026-11-16. GPU Probe 0.6.0 has a user-
confirmed triangle/input/exit pass on Switch Lite. World Data Probe 0.5.0 has a
confirmed WORLD_DATA_OBSERVED pass. Neither probe retains or renders terrain.
No preparation, full client build, source expansion or dependency download was run.

## Verified source boundaries

- `crates/horizon-world/src/registry.rs`: `BlockRegistry::from_facts_json_with_flags`
  constructs the state-ID registry and returns per-state `(air, fluid)` flags.
  `state`, `id` and `default_state` supply explicit mappings. The source's
  190,000-state example describes a large pack; it is not a measured vanilla
  state count or a Switch memory estimate.
- `crates/horizon-items/src/blocks.rs`: `BlockFacts::load` reads
  `block-behaviours.json` and `gameplay-facts.json`, checks the state count against
  `BlockRegistry`, and retains names, render shape, fluids, outline boxes, face
  sturdiness and occlusion facts. `StateFacts::outline` is picking geometry,
  not a substitute for a baked block model. Invisible block models and fluid
  rendering must be considered separately.
- `crates/horizon-client-render/src/mesher.rs`: `Mesher::mesh_reusing` uses a 3×3
  column neighbourhood and section index, with `Assets`, `RenderAssets`,
  `MeshTables`, biomes, face visibility and lighting options. A raw state array
  does not satisfy this interface. `TerrainVertex` fields sum to 24 bytes:
  position12, UV4, layer2, light2, color4. This is not the current GPU-probe ABI.
- `crates/horizon-client-render/src/chunk.rs`: `Section` contains paletted blocks,
  paletted biomes and `Light`; biome cells are 4×4×4, hence **64 per section**.
  `Light` has uniform, single-varying-layer and 4096-byte full forms. Chunk
  sections use `Vec<Arc<Section>>`; this is an upstream design reference, not
  a requirement to introduce Arc/std into the standalone no_std decoder.
- `platform/switch/gpu-probe/include/gpu_probe.h` and `native/gpu_native.c`:
  the tested bridge draws three vertices through a 96-byte uniform payload.
  It has **no depth target, vertex upload interface, atlas descriptors or MVP**.
  Shared command/uniform reuse is serialized by queue idle. Frame submission
  counts are not GPU timing measurements.

## Resource boundary

The pinned Java wire protocol supplies numeric built-in block-state IDs, not
all block models, textures or air/occlusion semantics. Do not turn every nonzero
state into a solid cube, identify air by an assumed numeric constant, or infer
render geometry from collision alone. The registry facts path above can provide
explicit air/fluid classification; correct model faces require model and render
resources as well.

`crates/horizon-prepare/src/main.rs` documents `--prepare`, `--server-jar`,
`--client-jar`, `--minecraft-dir`, and `--output`. Its library prepares files from
the user's pinned install and runs exporters; a separate decompile command is
not established as a prerequisite by this review. `lib.rs` also prepares
`render-definitions.json`, `occlusion.nbt` and gameplay facts. Importing resources
onto Switch needs a separately bounded manifest/version/size check and a compact
representation; loading the desktop object graph is not yet justified. No game
jars, exports, models or textures may be committed to the public fork.

A raw-ID grid or point visualization can truthfully display cells as data if it
makes no air/solid/shape claims. It must be labelled a diagnostic, not Minecraft
terrain. Untextured debug boxes classified by explicit facts are still not
accurate stairs, plants, fluids or transparent/cutout block models.

## Bounded next implementation

First normalize **one complete received column** into caller-owned Rust buffers,
using the existing validated decoder rather than a second protocol parser.
`include/column_codec.h` specifies bottom-up section order and Y/Z/X cell order.
Validate the complete packet, including light/NBT tails, before committing any
output. Retain coordinates, min Y, section count and raw IDs; no rendering or
PlayerLoaded claim follows from copying these arrays. The current task implements
this boundary; C owns any platform allocation, Rust owns interpretation.

For height384 (24 sections), normalized u32 arrays require exactly:

- Blocks: 24 × 4096 × 4 = 393,216 bytes.
- Biomes: 24 × 64 × 4 = 6,144 bytes.
- Combined: 399,360 bytes = 390 KiB, excluding metadata and staging.

At the 256-section cap this is 4,259,840 bytes (4.0625 MiB). Keeping both
2048-byte light arrays for all 26 light sections of a 384-high column would add
106,496 bytes (104 KiB), plus masks/metadata. The column-copy boundary does not
retain these light arrays. Compressed palettes and Arc overhead are deliberately
not estimated as fixed amounts.

For a later section-only debug mesh, emitting six faces × six unindexed vertices
for each of 4096 cells is a conservative upper bound: 3,538,944 bytes at the
upstream 24-byte vertex layout, before staging. A real mesher must reserve a
bounded output capacity, handle overflow and neighbour dependencies, and avoid
building this worst case for every section at once. These are arithmetic bounds,
not measured Switch allocations or a claim that all cells are visible cubes.

## Rendering and live-session work still required

Rust should own section meshing and a camera-relative MVP; the SDK boundary needs
bounded vertex uploads, explicit vertex attributes, a depth image/clear/test and
well-defined GPU buffer lifetimes. Accurate terrain also needs atlas/model
integration, transparency, biome tint and retained light. Queue-idle reuse is a
safe starting diagnostic, not a final throughput strategy.

A retained session must apply block/section and light updates, process unloads,
invalidate adjacent meshes at section/column boundaries, and clear state on
respawn/dimension/reconfiguration changes. Unknown IDs or missing resources need
an explicit unsupported/error outcome. Never keep a stale sample or expose a
partially decoded replacement column as current terrain. These requirements are
an integration plan, not implemented cache features. PlayerLoaded remains gated
on actual readiness; the previous probe's opaque or structure-only chunk receipt
is insufficient.

The next acceptance gate is normalized-column fixtures plus a live Java 26.3
copy check, followed by resource-table validation before terrain meshing. Native
GPU bring-up passed, but this review does not select the final Horizon backend
or prove the wgpu surface, performance, full terrain pipeline or vanilla parity.
