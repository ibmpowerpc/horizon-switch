# Static section mesh (`section`)

`src/section.rs` turns one 16x16x16 section's raw block ids into a bounded,
indexed-free triangle mesh and encodes it in the existing `CTTEX001` static
capture scene format. It is host-side preparation code for a native Switch
static capture preview: no device, network, game jar or asset file is touched
here.

**What it does:** for every cell, look up the caller's baked quads for that raw
id, translate the original quad positions by the cell's `[x, y, z]`, pick a
render mode, map each corner UV into its atlas sprite and emit the triangle
corners `[0, 1, 2, 0, 2, 3]` with white `RGBA`.

**What it never does:** subtract hidden faces, assume a cube for a bare id,
treat id 0 (or any id) as air, invent a fluid, light or tint colour, choose
weights with RNG, or jitter a coordinate. Every multipart component must
already be in the supplied quad list. Tinted quads are counted, not coloured.
Geometry, UV and winding stay the baker's.

## Public API

```rust
pub struct Vertex { pub p: [f32; 3], pub uv: [f32; 2], pub rgba: u32 }

pub struct SectionMesh {
    pub groups: [Vec<Vertex>; 3], // 0 solid, 1 cutout, 2 translucent
    pub cells_with_geometry: usize,
    pub empty_cells: usize,
    pub source_quads: usize,
    pub tinted_quads: usize,
}

pub fn mesh_section(
    blocks: &[u32],
    models: &BTreeMap<u32, Vec<horizon_assets::bake::ModelQuad>>,
    atlas: &Atlas,
) -> Result<SectionMesh, SectionError>;

pub fn encode_scene(mesh: &SectionMesh, atlas: &Atlas) -> Result<Vec<u8>, SectionError>;
```

`SectionError` implements `Display` and `std::error::Error`, and distinguishes
a wrong id count, a missing model entry, a missing or out-of-bounds sprite, an
unsupported render type, a bad position or UV, the vertex cap, a ragged mode
group, an empty encodable mesh, a bad atlas and the file cap. A failed call
returns no mesh and no bytes: the owned result is all-or-error.

## Inputs and ordering

- `blocks` is exactly `SECTION_BLOCKS` (4096) raw ids in **Y/Z/X** order:
  cell `index` is `x = index % 16`, `z = index / 16 % 16`, `y = index / 256`.
- Every id is looked up, id 0 included; an id with no entry is an error, not
  empty geometry. An entry with an empty quad list is a valid empty cell and
  counts in `empty_cells`.
- Cells are visited in index order; a cell's quads keep their supplied order;
  corners keep `[0, 1, 2, 0, 2, 3]`.
- The atlas is the crate's verified static [`Atlas`](ATLAS.md): its sprite
  rectangle gives the UV sub-rect. `has_alpha` / `has_partial_alpha` and
  `material.force_translucent` choose the mode only when the quad declares no
  render type, exactly as `examples/export_texture_scene.rs` does:

  | Declared | Mode |
  |---|---|
  | `Solid` | 0 |
  | `Cutout`, `CutoutMipped` | 1 |
  | `Translucent`, `Tripwire` | 2 |
  | `Unsupported` | error |
  | none, `force_translucent` or partial alpha | 2 |
  | none, alpha | 1 |
  | none, opaque | 0 |

- UVs may leave `[0, 1]` by at most `1e-6`; that roundoff is clamped into the
  sprite rectangle. Anything further is a repeating or invalid UV and fails.

## Hard limits

| Limit | Value |
|---|---|
| Section cells | exactly 4096 |
| Scene vertices | `MAX_VERTICES` 262140 |
| Position | finite, `|coordinate| <= 1024` |
| Atlas | power-of-two width and height, each `<= 1024`, `rgba.len() == width*height*4` |
| Scene file | `MAX_FILE_BYTES` 16 MiB |
| Mode groups | complete six-vertex quads |

The vertex and file bounds are checked before the next allocation grows, so a
too-large section fails without building a partial mesh or file.

## Binary format

Unchanged from `export_texture_scene` and the Switch decoder; an empty mesh is
**not** encodable because the decoder refuses zero vertices.

```text
offset 0   ASCII "CTTEX001"                 8 bytes
offset 8   LE u32 protocol 777
offset 12  LE u32 format 1
offset 16  LE u32 vertex count
offset 20  LE u32 non-empty mode draw count
offset 24  LE u32 atlas width (power of two, <= 1024)
offset 28  LE u32 atlas height (power of two, <= 1024)
offset 32  LE u32 reserved 0
offset 36  LE u32 reserved 0
offset 40  six LE f32: min XYZ, max XYZ over every vertex
offset 64  count * 24-byte vertices:
           3 LE f32 position + 2 LE f32 atlas UV + LE u32 RGBA
then       draws * 16-byte records, one per non-empty mode:
           LE u32 first vertex, LE u32 vertex count, LE u32 mode, LE u32 0
then       atlas RGBA8 bytes (width * height * 4)
```

Total length is exactly `64 + 24*count + 16*draws + atlas.rgba.len()`; RGBA is
memory byte order R, G, B, A (low byte R). This module always emits white
`0xffff_ffff` for the section's own vertices.

## Tests

Six independent synthetic tests (no local Minecraft install, no assets) cover:
Y/Z/X cell translation with several source quads in one cell, declared render
modes and the atlas UV-rect formula with triangle order, alpha/forced defaults,
missing ids (id 0 included) and bad count/sprites/render types/positions/UVs/
vertex cap, the exact 64-byte header, 24-byte vertices, 16-byte draw records,
bounds and length, and encoding refusals with UV roundoff clamping.

```sh
cargo test --offline --locked --jobs 1 \
  --manifest-path tools/switch_model_prep/Cargo.toml \
  --target-dir tools/switch_model_prep/build/host-target
```

The CLI that loads block ids, model maps and the atlas belongs to the caller;
this module reads no file and adds no dependency, `Cargo.toml` or upstream
change. The old 0.9 texture exporter is untouched.
