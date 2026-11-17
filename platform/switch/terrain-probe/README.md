# Rust Model Mesh Probe 0.8.0

Native Switch diagnostic showing a static 3×3 scene of real baked block-model
geometry: stone, stairs, wall, fence, slab, grass block, flower, glass and torch.
The geometry is prepared from the user's pinned Java 26.3 resources using the
existing Horizon Rust resolver/baker. Rust validates mesh input and owns the
orbit camera and controls; a C shim handles libnx I/O and deko3d resource calls.

This is an **untextured geometry diagnostic**, with opaque diagnostic colors.
It does not implement texture alpha, fluids, biome/terrain lighting, entities,
live chunks, playable multiplayer or PlayerLoaded. The user confirmed all nine shapes and A/B/X/right-stick/ZL/ZR/Plus controls
on Switch Lite. This hardware check does not establish FPS or visual parity.

## Prepare and build

First prepare [assets](../resources/README.md), [state facts](../../../tools/switch_state_facts/README.md)
and the [model-selection plan](../../../tools/switch_model_prep/README.md).
Then export the private scene (existing outputs are refused):

```sh
cargo run --offline --locked --jobs 1 \
  --manifest-path tools/switch_model_prep/Cargo.toml \
  --target-dir tools/switch_model_prep/build/host-target \
  --example export_probe_scene
bash scripts/build-switch.sh docker terrain
```

The build uses installed Rust/rust-src and the existing devkitA64 Docker image,
with no tool/dependency downloads. It reports disk usage and retains useful
compiler caches. The general model library/JSON plan stays on the host; the
Switch reads only the small probe-specific binary.

Local outputs in ignored `dist/switch/craftrustnx/`:

- `craftrustnx-terrain-probe.nro`
- `terrain-probe-mesh.bin` (copied if the scene export exists)
- `craftrustnx-terrain-notices.txt`

The mesh contains geometry derived from the user's game resources. Keep it
local, outside Git and public release archives. The NRO embeds only original
shader programs, not Minecraft models/textures.

## Run on Switch

Copy all three files to `sdmc:/switch/craftrustnx/`. Launch **CraftRustNX Rust
Model Mesh Probe** in hbmenu. The startup screen should say Rust validated
516 vertices. Press A to enter graphics.

| Control | Behavior |
| --- | --- |
| A | Toggle automatic orbit |
| B | Cycle background |
| X | Reset camera/background |
| Right stick | Rotate camera |
| ZL / ZR | Zoom in / out |
| + | Return to summary; press + again to exit |

Expected: nine separated shapes, correct perspective/overlap while rotating,
and working controls. Check the wall/fence/stairs rather than just a cube.
Glass/flower are deliberately opaque in this diagnostic. The summary reports
SDK/Rust error status, submitted frames and allocated GPU memory blocks. These
are diagnostic counters, not a frame-rate/GPU performance benchmark.

## Implementation and checks

- [terrain_app.h](include/terrain_app.h): exact CTMESH01/FFI layout.
- [Rust](rust/src/lib.rs): two-pass transactional decoder and bounded camera.
  Explicit zero-to-one depth projection, matching SDK device defaults.
- [SDK bridge](README-SDK.md): aligned vertex/uniform allocations, depth
  clear/test/write, hardware normalized color attributes and queue-idle reuse.
- [scene exporter](../../../tools/switch_model_prep/SCENE.md): nine named states,
  all selected model parts, diagnostic-only first weighted option.

Six Rust runtime tests cover decoding, malformed/truncated input, C layouts,
transactional failures, projection and camera controls. A real C caller linked
to the host Rust archive checks every vertex byte, initial camera-frustum
coverage, controls and no writes on corrupt input, using ASan/UBSan for C.
Rust/the shared libraries are not sanitizer-instrumented by that check.
The cross-built NRO/ELF pass packaging and writable-relocation verification.

On 2026-11-17 all three files were copied through DBI/MTP to SD Card and read
back byte-for-byte. No previous probe file was replaced. The user subsequently confirmed visible shapes and working controls on Switch
Lite; see [verification](../../../docs/verification.md).
