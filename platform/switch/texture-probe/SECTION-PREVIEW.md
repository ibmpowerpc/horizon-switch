# Captured Section Probe 0.11.0

A separate static preview of one real 16×16×16 section captured by Column
Export Probe 0.10.0. Host Rust reads the console's `CTCOL001` format using the
same pure decoder, maps raw IDs through the prepared Java 26.3 state tables,
preserves multipart models and reuses Horizon's model baker and static atlas.
Rust on Switch validates the scene and controls the camera; the existing
libnx/deko3d shim uploads and draws it. This is a step toward a connected world
renderer, not a live network client.

On 2026-11-24 the user confirmed that the captured section renders correctly on
Switch Lite. This closes visual acceptance for the transferred 0.11.0 scene;
geometry was prepared on the host, and no frame timings were reported.

## Prepare private data

Keep the capture, prepared resources, generated scene and report under ignored
`.local/`. They are not public repository/release assets. The existing local
26.3 assets, facts and model-selection plan must match the pinned client JAR.

```sh
cargo run --manifest-path tools/switch_model_prep/Cargo.toml \
  --offline --locked --jobs 1 --target-dir tools/switch_model_prep/build/host-target \
  --example export_section_scene -- \
  --capture .local/switch-captures/2026-11-24/capture-0000/column.ctcol \
  --section 8
bash scripts/build-switch.sh docker section
```

The section index is explicitly selected, counting from the bottom of the
captured dimension. With min Y -64, index 8 means cells at world Y64…79. The
scene places their models in local coordinates X/Z0…15, Y0…15. Surfaces at the
section boundary remain visible; neighboring sections/columns are not drawn.
No numeric ID is assumed to mean air or a solid cube. An unknown ID, bad input
pin, unsupported resource or exceeded mesh/atlas limit fails before publication.
Outputs are not replaced: choose a new `--output FILE` for another export.

The first weighted alternative in each part is an explicit static policy,
without vanilla RNG/jitter parity. Invisible states have empty geometry from
the actual prepared render-shape facts. Every selected multipart component
retains its original quads, winding and UVs. Nonempty fluids and unsupported
render shapes are rejected; animated/APNG/unsupported textures are refused by
the atlas builder. There is no cube or frame-zero substitute.

## Run on Switch

Copy these files into `sdmc:/switch/craftrustnx/`:

- `craftrustnx-section-probe.nro` (or its versioned 0.11.0 copy)
- `captured-section-scene.bin`
- `craftrustnx-section-notices.txt`

Launch **CraftRustNX Captured Section Probe**, then press A to enter 3D.
A toggles orbit, B changes the background, X resets, the right stick rotates,
ZL/ZR zoom, and Plus returns to the results screen (Plus again exits).
The Rust camera fits the actual scene bounds, so a complete section can be
viewed from outside. This is an orbit view rather than player movement.

Check the captured ground, tree and plant models, cutout holes and depth while
rotating. Foliage uses neutral white vertex tint: biome coloring, world light,
fluid geometry, animated sprites, block entities, live block/light updates,
collision and gameplay remain absent. No `PlayerLoaded` is sent. The static
preview emits all model quads without neighbor occlusion culling. Opaque and
cutout meshes remain uploaded while the camera moves; blended quads still sort
and re-upload on camera changes, as in the previous texture probe.

## Verify

The exporter writes a private JSON report beside the scene, recording source
capture/facts/selection/manifest/scene hashes, the explicit section, cell and
quad counts, atlas and mode sizes, and scope. Generated scene bytes use the
existing `CTTEX001` contract; no new runtime decoder or rendering backend is
introduced. The 0.9.1 nine-model scene and its SD files are kept separate.

```sh
bash platform/switch/texture-probe/tests/run-host.sh \
  .local/switch-models/26.3/captured-section-scene.bin
```

This links the real runtime Rust scene decoder/camera/sorter with independent
C fixtures and the prepared scene. C is ASan/UBSan-instrumented; the Rust static
archive is not. Hardware visual acceptance remains a separate user check.
