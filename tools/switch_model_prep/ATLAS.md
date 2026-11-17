# Static atlas and textured preview

The host Rust library decodes only the sprites referenced by baked model quads.
`build_static_atlas` takes a manifest-verified asset map, sprite IDs and
`AtlasLimits`. It returns straight RGBA8 pixels and inner sprite rectangles.
It performs no filesystem access, gamma conversion or premultiplication.

Supported PNG forms are 8-bit RGB, RGBA, gray, gray-alpha and indexed+tRNS;
sub-byte forms expanded by the PNG decoder are accepted too. Sixteen-bit PNG,
APNG and `.mcmeta` containing animation are refused. Unsupported data never
silently becomes frame zero or a placeholder. This is a static subset, not a
resource-pack renderer.

Default bounds: 1024-pixel atlas side, 256 distinct sprites and 64 MiB of decoded
sprite RGBA. Sources are at most 2 MiB and 4096 pixels per side. These are input
and allocation bounds, not hardware memory measurements. Packing chooses the
smallest fitting power-of-two square using deterministic shelves and canonical
sprite IDs. Two pixels of edge/corner extrusion surround each sprite; unused
space is transparent zero. `SpriteRect` excludes that gutter.

`has_alpha` means at least one texel has alpha below 255; `has_partial_alpha`
means at least one alpha is between 0 and 255. Neither is a complete rendering
layer decision. Model `render_type` and `force_translucent` can override it.

## Verify the real scene without writing files

From the repository root:

```sh
cargo run --offline --locked --jobs 1 \
  --manifest-path tools/switch_model_prep/Cargo.toml \
  --target-dir tools/switch_model_prep/build/host-target \
  --example atlas_check
```

Optional `--mesh FILE` and `--manifest FILE` select the scene report and manifest;
assets are resolved relative to that manifest. The tool checks the actual schema:
manifest `files` is an array of `{path,size,sha256}`, and scene
`samples[].signatures` contains `{model,x,y,uvlock}` objects. It verifies the
26.3/protocol-777/client-SHA1 pins, scene-to-manifest hash, every listed asset's
size/hash, path safety, collisions, symlinks and bounded reads. The combined
manifest and asset bytes are limited to 8 MiB, individual assets to 2 MiB and the
entry count to 12,000.

The checker calls the shared Horizon baker and reports aggregate JSON to stdout.
It does not write reports or textures. Its UV range describes original baked
quad coordinates, including floating-point excursions; it does not describe
atlas rectangle positions.

Verified local nine-model scene: 12 baked requests, 86 quads, 10 sprites,
128-by-128 RGBA atlas (65,536 bytes), four alpha sprites, no partial-alpha sprites,
six forced-translucent quads. Eight quads have tiny negative UVs from rotation;
the minimum is approximately -5.96e-8. None exceed the unit interval by 1e-6.

## Export the separate Switch preview

```sh
cargo run --offline --locked --jobs 1 \
  --manifest-path tools/switch_model_prep/Cargo.toml \
  --target-dir tools/switch_model_prep/build/host-target \
  --example export_texture_scene
```

Options: `--assets DIR`, `--scene REPORT`, `--output FILE`. Defaults read the same
private inputs and publish `.local/switch-models/26.3/texture-probe-scene.bin`
plus its JSON report. Existing outputs are refused. Each file is published using
a temporary sibling and an atomic, no-replace hard link; the pair is not an atomic
transaction. All generated data stays ignored and must not be committed.

The scene retains the nine sample positions and explicit first weighted choices
from probe 0.8. Baked UVs within 1e-6 of a unit boundary are clamped to that boundary
before atlas mapping; larger excursions are refused, not treated as repetition.
All model parts remain. Vertices use neutral white, including faces that would
normally receive biome tint. No world lighting, animation, fluids or gameplay
are implied.

Draw groups follow declared render types, then forced translucency/partial alpha,
then binary alpha cutout, otherwise opaque. Their vertex counts are 396 opaque,
84 cutout and 36 translucent (516 total). The preview does not implement full
region-opacity analysis or face culling against neighbouring blocks. Rust sorts
whole translucent quads by camera depth when the camera changes. Native code
handles texture upload, sampling, depth and blending.

`CTTEX001` has a 64-byte little-endian header: magic; protocol, format,
vertex/draw counts, atlas width/height, two zero words; six bound floats at byte
40. Then come 24-byte position/UV/RGBA vertices, 16-byte draw ranges and tightly
packed RGBA pixels. The current private file is 78,032 bytes. Runtime validation
and controls are Rust; a small deko3d bridge owns native resources.

This verifies data preparation. Physical texture sampling, alpha, colour output
and frame timing require the separate 0.9 device run.
