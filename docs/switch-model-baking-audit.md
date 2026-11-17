# Switch block-model preparation: source review

2026-11-17, current fork source. Parent reviewed the Kiro investigation against
actual `horizon-assets` source and corrected dependency, RNG and memory claims.
This document is a reuse plan, not a renderer or performance result.

## Existing Rust code

`crates/horizon-assets/Cargo.toml` depends on **horizon-core**, log, serde and
serde_json with `raw_value`; horizon-core also needs libm. It has no wgpu/winit
requirement. Cached compatible source packages are present, but this review did
not build the upstream crate or prove the entire workspace resolves offline.
The standalone model-selector tool currently uses cached serde_json and sha2.

Verified entry points:

- `model::CuboidModel::from_json`: optional parent, textures, elements, display
  transforms, render_type and loader metadata. Absent elements inherit; an
  explicit empty list replaces parent geometry. Null elements/faces need
  diagnosis rather than being turned into cubes.
- `blockstate::BlockStateDefinition::from_json`: ordered variants and multipart.
  `blockstate::bind` binds selectors against the property schema and canonical
  states. Vanilla's overlap recovery is order-sensitive; the current standalone
  strict selector rejects overlap rather than trying to reproduce that recovery.
- `resolve::ModelLibrary::{new, resolves, resolve}`: immutable shared geometry,
  complete parent traversal and texture aliases. `ResolvedCuboidModel` exposes
  elements, resolved materials and unresolved aliases. Child texture slots win.
  `TextureSlot` supports references and `Material { sprite, force_translucent }`.
- `bake::bake_block_model(resolved, BlockModelTransform)`: block geometry,
  including the special treatment of zero-extent elements. General
  `bake_model` is a distinct entry point and should not replace it blindly.
- `bake::ModelQuad`: four positions/UVs, facing, optional cull direction, material,
  tint, light/emission, shading, render type and optional AO/color overrides.
- `bake::BlockModelTransform::new([x,y,z], uv_lock)`: 64 cached quarter-turn
  orientations; UV locking is a separate transform choice. Preserve the upstream
  unfused floating-point arithmetic rather than inventing approximate rotations.

These APIs use std/alloc, Arc and, in the resolver, Mutex. They suit offline host
preparation; direct reuse in a no_std Switch runtime is not established.

## Selection and random choices

`blockstate/choice.rs` uses checked signed-32-bit weight prefix sums and a Java
bounded random draw. A weighted array with just one element still consumes that
draw; a plain single-model object does not. Multipart choices also have random
consumption rules. The preparer must retain the array/object distinction and all
alternatives. It does not choose a random model or establish RNG parity itself.

Multipart property values can contain `|` terms and `!` negation. Variant keys
are comma-separated property equalities, not OR expressions. The source decoder
also understands z rotation, primitive coercions and recovery cases that the
current strict x/y-only preparation subset does not implement. The pinned local
blockstate set uses only model/x/y/uvlock/weight option keys; processing that set
successfully does not imply general resource-pack compatibility.

## Inputs and empty geometry

The local schema-2 facts contain 35,723 numeric state IDs and canonical property
maps, with explicit is_air/fluid_empty/render_shape flags. The render shapes are
MODEL, INVISIBLE and ENTITYBLOCK_ANIMATED. Geometry must come from resource
models, not collision boxes or a nonzero-ID rule. Fluid geometry is separate.

The verified local selection contains 64 MODEL states whose wall multipart
conditions select no parts (up=false and no arms). This is valid empty geometry,
unrelated to OBJ loaders or missing resources. Do not synthesize solid cubes.
The upstream resolver has a missing-model fallback, but the preparation tool
currently fails on missing listed assets or bad hashes so resource defects stay
visible. The generated missing texture/animation policy remains to be designed.

## Bounded next geometry milestone

Use the existing Rust resolver/baker on the host to prepare deduplicated model
and rotation combinations, retaining face materials, cull direction, tint and
render layer. First verify inherited cuboids, rotated stairs and multipart walls
against independent synthetic geometry expectations and upstream fixtures.
Persist versioned, bounded data under ignored `.local`, with an explicit limit
and unsupported outcome. Do not ship Mojang resource files in Git.

A later Rust runtime should consume that compact geometry and own meshing,
position-dependent choices and camera transforms. It also needs actual retained
light/biomes, neighbouring cells, block/light updates, unload invalidation and
world-change resets. JSON preparation does not supply those runtime systems.

## GPU and memory boundaries

The tested deko3d bridge draws three vertices from uniform data; it has no depth
image, arbitrary vertex upload, atlas binding or MVP yet. Extend the SDK bridge
with explicit vertex attributes, depth clear/test, bounded uploads and safe GPU
buffer lifetimes. Rust remains responsible for terrain/camera logic. Do not pass
Rust Vec ownership across C ABI; pass borrowed buffers with documented lifetimes.

The existing upstream TerrainVertex is 24 bytes. For **six-faced debug cubes
only**, 4096 cells × 6 faces × 6 vertices × 24 bytes = 3,538,944 bytes (3.375 MiB)
before staging. Real models may have many more faces; this is not a general
section maximum. Bound the emitted vertex count and handle overflow explicitly.
A 2048×2048 RGBA atlas is 16 MiB per level-zero image, before mipmaps, padding,
staging and additional textures. Its size/coverage has not been proven adequate.
No measured 2 MiB model library or 23 MiB section-memory bound is established.
