//! Bounded host-side mesh emitter for one static 16x16x16 block section.
//!
//! The caller supplies the section's raw block ids, the baked quads of every
//! mapped id (every multipart component already resolved by the caller) and one
//! verified static atlas. This module only translates the original quad
//! positions by the cell's `[x, y, z]`, picks the render mode exactly as
//! `examples/export_texture_scene.rs` does and maps each UV into its sprite
//! rectangle. It subtracts no hidden faces, assumes no cube for a bare id,
//! treats no id (0 included) as air, guesses no fluid, light or tint colour and
//! samples no randomness: every vertex is an original baked quad corner with a
//! white RGBA, and every failure returns no mesh at all.
//!
//! [`encode_scene`] writes exactly the existing `CTTEX001` static capture
//! format that `export_texture_scene` produces and the Switch decoder reads;
//! the only difference is the mesh it describes. An empty mesh is a valid
//! result of [`mesh_section`] but not an encodable scene, because the decoder
//! refuses zero vertices.

use crate::{Atlas, SpriteRect};
use horizon_assets::bake::{ModelQuad, RenderType};
use std::{collections::BTreeMap, fmt};

/// A section is always exactly 16x16x16 cells.
pub const SECTION_BLOCKS: usize = 4096;
/// Hard cap on vertices in one scene file.
pub const MAX_VERTICES: usize = 262_140;
/// Largest accepted absolute coordinate on any axis, in block units.
pub const MAX_COORDINATE: f32 = 1024.0;
/// Largest accepted power-of-two atlas width or height.
pub const MAX_ATLAS_DIMENSION: u32 = 1024;
/// Hard cap on one encoded scene file.
pub const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
/// The only accepted excursion outside `[0, 1]` before a UV is clamped to the
/// sprite rectangle. Anything larger is a repeating or invalid UV.
pub const UV_ROUNDOFF: f32 = 1e-6;

/// One scene vertex in the encoded layout: three little-endian `f32` position
/// components, two little-endian `f32` atlas UVs and one little-endian RGBA
/// word.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub p: [f32; 3],
    pub uv: [f32; 2],
    pub rgba: u32,
}

/// One emitted section, split by render mode: index 0 solid, 1 cutout, 2
/// translucent. Each group holds whole six-vertex quads in emission order.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionMesh {
    pub groups: [Vec<Vertex>; 3],
    /// Cells whose mapped model list was not empty.
    pub cells_with_geometry: usize,
    /// Cells whose mapped model list was empty; still looked up.
    pub empty_cells: usize,
    /// Quads taken from the supplied models, tinted or not.
    pub source_quads: usize,
    /// Source quads with a non-negative `tint_index`. They stay white here.
    pub tinted_quads: usize,
}

/// Why a mesh or scene was refused. Every variant leaves nothing behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SectionError {
    /// The block id slice is not exactly [`SECTION_BLOCKS`] long.
    BlockCount(usize),
    /// The atlas is not one of the bounded static atlases this format carries.
    Atlas(String),
    /// A raw block id has no entry in the supplied model map, id 0 included.
    MissingModel(u32),
    /// A quad names a sprite the atlas does not hold.
    MissingSprite(String),
    /// A sprite rectangle is empty or leaves the atlas image.
    SpriteOutsideAtlas(String),
    /// A quad declares a render type this static format cannot place.
    UnsupportedRenderType { block: u32, sprite: String },
    /// A quad corner is non-finite or beyond [`MAX_COORDINATE`] after
    /// translation.
    InvalidPosition { block: u32, sprite: String },
    /// A quad corner UV repeats beyond the accepted roundoff.
    InvalidUv { block: u32, sprite: String },
    /// Emitting the next quad would exceed [`MAX_VERTICES`].
    VertexLimit(usize),
    /// A mode group does not hold complete six-vertex quads.
    RaggedGroup(usize),
    /// A mesh with no vertices cannot be encoded; the decoder refuses zero.
    EmptyMesh,
    /// A hand-built vertex field is not finite or out of range.
    InvalidVertex(&'static str),
    /// The encoded scene would exceed [`MAX_FILE_BYTES`].
    FileLimit(usize),
}

impl fmt::Display for SectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BlockCount(got) => write!(
                formatter,
                "a section carries exactly {SECTION_BLOCKS} block ids, got {got}"
            ),
            Self::Atlas(reason) => write!(
                formatter,
                "atlas is not a bounded static RGBA8 atlas: {reason}"
            ),
            Self::MissingModel(id) => {
                write!(formatter, "block id {id} has no baked model entry")
            }
            Self::MissingSprite(sprite) => {
                write!(formatter, "sprite `{sprite}` is not in the atlas")
            }
            Self::SpriteOutsideAtlas(sprite) => write!(
                formatter,
                "sprite `{sprite}` has an empty or out-of-bounds atlas rectangle"
            ),
            Self::UnsupportedRenderType { block, sprite } => write!(
                formatter,
                "block id {block} sprite `{sprite}` declares an unsupported render type"
            ),
            Self::InvalidPosition { block, sprite } => write!(
                formatter,
                "block id {block} sprite `{sprite}` emits a non-finite or out-of-range position"
            ),
            Self::InvalidUv { block, sprite } => write!(
                formatter,
                "block id {block} sprite `{sprite}` emits a UV outside its sprite"
            ),
            Self::VertexLimit(reached) => write!(
                formatter,
                "{reached} vertices exceed the {MAX_VERTICES}-vertex limit"
            ),
            Self::RaggedGroup(mode) => write!(
                formatter,
                "mode {mode} does not hold complete six-vertex quads"
            ),
            Self::EmptyMesh => formatter.write_str("an empty mesh is not an encodable scene"),
            Self::InvalidVertex(field) => {
                write!(
                    formatter,
                    "mesh vertex {field} is not finite or out of range"
                )
            }
            Self::FileLimit(bytes) => write!(
                formatter,
                "{bytes} scene bytes exceed the {MAX_FILE_BYTES}-byte limit"
            ),
        }
    }
}

impl std::error::Error for SectionError {}

/// Emit one static section mesh from raw block ids and their baked quads.
///
/// `blocks` is exactly [`SECTION_BLOCKS`] raw ids in Y/Z/X order: cell `index`
/// is `x = index % 16`, `z = index / 16 % 16`, `y = index / 256`. Every id is
/// looked up, id 0 included; a missing entry is an error even for an id whose
/// model would be empty. Every quad of a mapped id is emitted in its supplied
/// order, translated by the cell position and triangulated as
/// `[0, 1, 2, 0, 2, 3]` with unchanged winding. Tint is counted, never applied:
/// every vertex is white.
pub fn mesh_section(
    blocks: &[u32],
    models: &BTreeMap<u32, Vec<ModelQuad>>,
    atlas: &Atlas,
) -> Result<SectionMesh, SectionError> {
    if blocks.len() != SECTION_BLOCKS {
        return Err(SectionError::BlockCount(blocks.len()));
    }
    check_atlas(atlas)?;

    let mut groups: [Vec<Vertex>; 3] = core::array::from_fn(|_| Vec::new());
    let mut cells_with_geometry = 0usize;
    let mut empty_cells = 0usize;
    let mut source_quads = 0usize;
    let mut tinted_quads = 0usize;
    let mut vertices = 0usize;

    for (index, id) in blocks.iter().enumerate() {
        let quads = models.get(id).ok_or(SectionError::MissingModel(*id))?;
        if quads.is_empty() {
            empty_cells += 1;
            continue;
        }
        cells_with_geometry += 1;
        let x = (index % 16) as f32;
        let z = (index / 16 % 16) as f32;
        let y = (index / 256) as f32;

        for quad in quads {
            let sprite = &quad.material.sprite;
            let rect = atlas
                .sprites
                .get(sprite)
                .copied()
                .ok_or_else(|| SectionError::MissingSprite(sprite.clone()))?;
            if rect.width == 0
                || rect.height == 0
                || rect
                    .x
                    .checked_add(rect.width)
                    .is_none_or(|right| right > atlas.width)
                || rect
                    .y
                    .checked_add(rect.height)
                    .is_none_or(|bottom| bottom > atlas.height)
            {
                return Err(SectionError::SpriteOutsideAtlas(sprite.clone()));
            }
            let mode = render_mode(quad, &rect, *id)?;

            source_quads += 1;
            if quad.tint_index >= 0 {
                tinted_quads += 1;
            }

            let next = vertices + 6;
            if next > MAX_VERTICES {
                return Err(SectionError::VertexLimit(next));
            }

            for corner in [0usize, 1, 2, 0, 2, 3] {
                let source = quad.positions[corner];
                let position = [source[0] + x, source[1] + y, source[2] + z];
                if !position
                    .iter()
                    .all(|coordinate| coordinate.is_finite() && coordinate.abs() <= MAX_COORDINATE)
                {
                    return Err(SectionError::InvalidPosition {
                        block: *id,
                        sprite: sprite.clone(),
                    });
                }
                let uv = quad.uv[corner];
                if !uv.iter().all(|value| {
                    value.is_finite() && (-UV_ROUNDOFF..=1.0 + UV_ROUNDOFF).contains(value)
                }) {
                    return Err(SectionError::InvalidUv {
                        block: *id,
                        sprite: sprite.clone(),
                    });
                }
                groups[mode].push(Vertex {
                    p: position,
                    uv: [
                        (rect.x as f32 + uv[0].clamp(0.0, 1.0) * rect.width as f32)
                            / atlas.width as f32,
                        (rect.y as f32 + uv[1].clamp(0.0, 1.0) * rect.height as f32)
                            / atlas.height as f32,
                    ],
                    rgba: 0xffff_ffff,
                });
            }
            vertices = next;
        }
    }

    Ok(SectionMesh {
        groups,
        cells_with_geometry,
        empty_cells,
        source_quads,
        tinted_quads,
    })
}

/// Encode one mesh in the existing static capture scene format.
///
/// The bytes are the same `CTTEX001` file `examples/export_texture_scene.rs`
/// writes: a 64-byte header (magic, protocol 777, format 1, vertex count, draw
/// count, atlas width and height, zero reserved words, six little-endian bounds
/// `f32`s), 24-byte little-endian vertices, one 16-byte draw record per
/// non-empty mode (`first vertex`, `vertex count`, `mode`, zero reserved) and
/// the atlas RGBA bytes. An empty mesh, a ragged mode group or an atlas that is
/// not a power of two up to [`MAX_ATLAS_DIMENSION`] is refused before any
/// allocation grows past [`MAX_FILE_BYTES`].
pub fn encode_scene(mesh: &SectionMesh, atlas: &Atlas) -> Result<Vec<u8>, SectionError> {
    check_atlas(atlas)?;

    let mut count = 0usize;
    for (mode, group) in mesh.groups.iter().enumerate() {
        if !group.len().is_multiple_of(6) {
            return Err(SectionError::RaggedGroup(mode));
        }
        count = count
            .checked_add(group.len())
            .ok_or(SectionError::VertexLimit(usize::MAX))?;
        if count > MAX_VERTICES {
            return Err(SectionError::VertexLimit(count));
        }
    }
    if count == 0 {
        return Err(SectionError::EmptyMesh);
    }

    let draws = mesh.groups.iter().filter(|group| !group.is_empty()).count();
    let expected = 64usize
        .checked_add(count * 24)
        .and_then(|size| size.checked_add(draws * 16))
        .and_then(|size| size.checked_add(atlas.rgba.len()))
        .ok_or(SectionError::FileLimit(usize::MAX))?;
    if expected > MAX_FILE_BYTES {
        return Err(SectionError::FileLimit(expected));
    }

    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for vertex in mesh.groups.iter().flatten() {
        for (axis, coordinate) in vertex.p.iter().enumerate() {
            if !coordinate.is_finite() || coordinate.abs() > MAX_COORDINATE {
                return Err(SectionError::InvalidVertex("position"));
            }
            low[axis] = low[axis].min(*coordinate);
            high[axis] = high[axis].max(*coordinate);
        }
        if !vertex
            .uv
            .iter()
            .all(|value| value.is_finite() && (-UV_ROUNDOFF..=1.0 + UV_ROUNDOFF).contains(value))
        {
            return Err(SectionError::InvalidVertex("UV"));
        }
    }

    // The runtime orbit camera rejects an entirely collapsed bounding box.
    if !(0..3).any(|axis| low[axis] < high[axis]) {
        return Err(SectionError::InvalidVertex("degenerate bounds"));
    }
    let mut out = Vec::with_capacity(expected);
    out.extend_from_slice(b"CTTEX001");
    for word in [
        777u32,
        1,
        count as u32,
        draws as u32,
        atlas.width,
        atlas.height,
        0,
        0,
    ] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    for coordinate in low.into_iter().chain(high) {
        out.extend_from_slice(&coordinate.to_le_bytes());
    }
    for vertex in mesh.groups.iter().flatten() {
        for coordinate in &vertex.p {
            out.extend_from_slice(&coordinate.to_le_bytes());
        }
        // The same roundoff tolerance is clamped here as in `mesh_section`, so
        // a hand-built vertex never samples outside its sprite rectangle.
        for value in &vertex.uv {
            out.extend_from_slice(&value.clamp(0.0, 1.0).to_le_bytes());
        }
        out.extend_from_slice(&vertex.rgba.to_le_bytes());
    }
    let mut first = 0u32;
    for (mode, group) in mesh.groups.iter().enumerate() {
        if group.is_empty() {
            continue;
        }
        for word in [first, group.len() as u32, mode as u32, 0] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        first += group.len() as u32;
    }
    out.extend_from_slice(&atlas.rgba);

    debug_assert_eq!(out.len(), expected);
    Ok(out)
}

/// Mode selection copied from `examples/export_texture_scene.rs`: a declared
/// render type wins; without one the atlas sprite's alpha flags decide.
fn render_mode(quad: &ModelQuad, rect: &SpriteRect, block: u32) -> Result<usize, SectionError> {
    let mode = match quad.render_type {
        Some(RenderType::Translucent | RenderType::Tripwire) => 2,
        Some(RenderType::Solid) => 0,
        Some(RenderType::Cutout | RenderType::CutoutMipped) => 1,
        Some(RenderType::Unsupported) => {
            return Err(SectionError::UnsupportedRenderType {
                block,
                sprite: quad.material.sprite.clone(),
            })
        }
        None => {
            if quad.material.force_translucent || rect.has_partial_alpha {
                2
            } else if rect.has_alpha {
                1
            } else {
                0
            }
        }
    };
    Ok(mode)
}

/// Reject an atlas whose image cannot be the exact RGBA8 payload this format
/// carries.
fn check_atlas(atlas: &Atlas) -> Result<(), SectionError> {
    let (width, height) = (atlas.width, atlas.height);
    if width == 0
        || height == 0
        || !width.is_power_of_two()
        || !height.is_power_of_two()
        || width > MAX_ATLAS_DIMENSION
        || height > MAX_ATLAS_DIMENSION
    {
        return Err(SectionError::Atlas(format!(
            "{width}x{height} is not a power-of-two atlas up to {MAX_ATLAS_DIMENSION}"
        )));
    }
    let expected = width as usize * height as usize * 4;
    if atlas.rgba.len() != expected {
        return Err(SectionError::Atlas(format!(
            "{width}x{height} needs {expected} RGBA8 bytes, got {}",
            atlas.rgba.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_assets::model::{Direction, Material};

    fn rect(x: u32, y: u32, width: u32, height: u32) -> SpriteRect {
        SpriteRect {
            x,
            y,
            width,
            height,
            has_alpha: false,
            has_partial_alpha: false,
        }
    }

    fn atlas(width: u32, height: u32, sprites: &[(&str, SpriteRect)]) -> Atlas {
        Atlas {
            width,
            height,
            rgba: vec![255; width as usize * height as usize * 4],
            sprites: sprites
                .iter()
                .map(|(id, rect)| ((*id).to_owned(), *rect))
                .collect(),
        }
    }

    fn quad(
        positions: [[f32; 3]; 4],
        uv: [[f32; 2]; 4],
        sprite: &str,
        render_type: Option<RenderType>,
        tint_index: i32,
    ) -> ModelQuad {
        ModelQuad {
            positions,
            uv,
            facing: Direction::North,
            cull: None,
            material: Material {
                sprite: sprite.to_owned(),
                force_translucent: false,
            },
            tint_index,
            shade_direction_override: None,
            light_emission: 0,
            light: [0, 0],
            color: None,
            ambient_occlusion: None,
            render_type,
            shade: true,
        }
    }

    fn models(entries: &[(u32, Vec<ModelQuad>)]) -> BTreeMap<u32, Vec<ModelQuad>> {
        entries.iter().cloned().collect()
    }

    fn vertex(p: [f32; 3], uv: [f32; 2]) -> Vertex {
        Vertex {
            p,
            uv,
            rgba: 0xffff_ffff,
        }
    }

    fn quad_vertices(points: [[f32; 3]; 4], uvs: [[f32; 2]; 4]) -> Vec<Vertex> {
        [0usize, 1, 2, 0, 2, 3]
            .into_iter()
            .map(|corner| vertex(points[corner], uvs[corner]))
            .collect()
    }

    const UNIT: [[f32; 3]; 4] = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
    ];
    const UNIT_UV: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

    fn le_u32(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    fn le_f32(bytes: &[u8], offset: usize) -> f32 {
        f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    fn le_vertex(vertex: &Vertex) -> Vec<u8> {
        let mut out = Vec::new();
        for coordinate in vertex.p.into_iter().chain(vertex.uv) {
            out.extend_from_slice(&coordinate.to_le_bytes());
        }
        out.extend_from_slice(&vertex.rgba.to_le_bytes());
        out
    }

    /// A cell at `(x, y, z)` translates every corner by exactly that offset, and
    /// several source quads keep their supplied order inside one cell.
    #[test]
    fn cells_translate_in_yzx_order_and_keep_quad_order() {
        let atlas = atlas(4, 4, &[("minecraft:block/a", rect(0, 0, 4, 4))]);
        let first = quad(
            UNIT,
            UNIT_UV,
            "minecraft:block/a",
            Some(RenderType::Solid),
            -1,
        );
        let second = quad(
            [
                [0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
                [1.0, 0.0, 0.0],
            ],
            UNIT_UV,
            "minecraft:block/a",
            Some(RenderType::Solid),
            -1,
        );
        let models = models(&[(0, Vec::new()), (1, vec![first, second])]);

        let mut blocks = [0u32; SECTION_BLOCKS];
        // y = 2, z = 3, x = 5; Y/Z/X order would confuse this with other cells.
        blocks[2 * 256 + 3 * 16 + 5] = 1;

        let mesh = mesh_section(&blocks, &models, &atlas).unwrap();
        assert_eq!(mesh.cells_with_geometry, 1);
        assert_eq!(mesh.empty_cells, SECTION_BLOCKS - 1);
        assert_eq!(mesh.source_quads, 2);
        assert_eq!(mesh.tinted_quads, 0);
        assert!(mesh.groups[1].is_empty() && mesh.groups[2].is_empty());
        assert_eq!(mesh.groups[0].len(), 12);

        let expected = [
            [5.0, 2.0, 3.0],
            [6.0, 2.0, 3.0],
            [6.0, 2.0, 4.0],
            [5.0, 2.0, 3.0],
            [6.0, 2.0, 4.0],
            [5.0, 2.0, 4.0],
            // The second source quad translates by the same cell offset.
            [5.0, 2.0, 3.0],
            [5.0, 3.0, 3.0],
            [6.0, 3.0, 3.0],
            [5.0, 2.0, 3.0],
            [6.0, 3.0, 3.0],
            [6.0, 2.0, 3.0],
        ];
        // A full-atlas sprite keeps the source UVs; both quads use `UNIT_UV`.
        let expected_uv = [
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.0, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.0, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
        ];
        for ((vertex, position), uv) in mesh.groups[0].iter().zip(expected).zip(expected_uv) {
            assert_eq!(vertex.p, position);
            assert_eq!(vertex.uv, uv);
            assert_eq!(vertex.rgba, 0xffff_ffff);
        }
    }

    /// Render types choose the mode group exactly as the texture scene does,
    /// the UV rectangle maps with the same formula, and triangle corners keep
    /// the `[0,1,2,0,2,3]` order.
    #[test]
    fn render_modes_uv_rects_and_triangle_order_match_the_texture_scene() {
        let atlas = atlas(
            8,
            4,
            &[
                ("minecraft:block/solid", rect(1, 2, 2, 1)),
                ("minecraft:block/cut", rect(3, 0, 2, 2)),
                ("minecraft:block/trans", rect(5, 1, 3, 2)),
            ],
        );
        let uv = [[0.25, 0.5], [0.75, 0.5], [0.75, 1.0], [0.25, 1.0]];
        let models = models(&[
            (0, Vec::new()),
            (
                1,
                vec![quad(
                    UNIT,
                    uv,
                    "minecraft:block/solid",
                    Some(RenderType::Solid),
                    3,
                )],
            ),
            (
                2,
                vec![quad(
                    UNIT,
                    uv,
                    "minecraft:block/cut",
                    Some(RenderType::Cutout),
                    -1,
                )],
            ),
            (
                3,
                vec![quad(
                    UNIT,
                    uv,
                    "minecraft:block/cut",
                    Some(RenderType::CutoutMipped),
                    -1,
                )],
            ),
            (
                4,
                vec![quad(
                    UNIT,
                    uv,
                    "minecraft:block/trans",
                    Some(RenderType::Translucent),
                    -1,
                )],
            ),
            (
                5,
                vec![quad(
                    UNIT,
                    uv,
                    "minecraft:block/trans",
                    Some(RenderType::Tripwire),
                    -1,
                )],
            ),
        ]);

        let mut blocks = [0u32; SECTION_BLOCKS];
        for (index, id) in [1u32, 2, 3, 4, 5].into_iter().enumerate() {
            blocks[index] = id;
        }
        let mesh = mesh_section(&blocks, &models, &atlas).unwrap();

        assert_eq!(mesh.cells_with_geometry, 5);
        assert_eq!(mesh.empty_cells, SECTION_BLOCKS - 5);
        assert_eq!(mesh.source_quads, 5);
        assert_eq!(mesh.tinted_quads, 1);
        assert_eq!(mesh.groups[0].len(), 6);
        assert_eq!(mesh.groups[1].len(), 12);
        assert_eq!(mesh.groups[2].len(), 12);

        let corners = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ];
        for (vertex, corner) in mesh.groups[0].iter().zip(corners) {
            assert_eq!(vertex.p, corner);
        }
        // Solid sprite: (1,2) 2x1 inside an 8x4 atlas.
        let expected_uv = [(1.0 + 0.25 * 2.0) / 8.0, (2.0 + 0.5 * 1.0) / 4.0];
        assert_eq!(mesh.groups[0][0].uv, [expected_uv[0], expected_uv[1]]);

        // Two cutout quads from adjacent cells, both translated on x only.
        for (offset, base) in [(1.0, 0), (2.0, 6)] {
            let quad = &mesh.groups[1][base..base + 6];
            assert_eq!(quad[0].p, [offset, 0.0, 0.0]);
            assert_eq!(quad[1].p, [offset + 1.0, 0.0, 0.0]);
            assert_eq!(quad[2].p, [offset + 1.0, 0.0, 1.0]);
            assert_eq!(quad[3].p, [offset, 0.0, 0.0]);
            assert_eq!(quad[4].p, [offset + 1.0, 0.0, 1.0]);
            assert_eq!(quad[5].p, [offset, 0.0, 1.0]);
            // Cutout sprite: (3,0) 2x2 inside an 8x4 atlas.
            assert_eq!(
                quad[0].uv,
                [(3.0 + 0.25 * 2.0) / 8.0, (0.0 + 0.5 * 2.0) / 4.0]
            );
        }

        // Translucent sprite: (5,1) 3x2 inside an 8x4 atlas.
        assert_eq!(
            mesh.groups[2][0].uv,
            [(5.0 + 0.25 * 3.0) / 8.0, (1.0 + 0.5 * 2.0) / 4.0]
        );
        assert!(mesh
            .groups
            .iter()
            .flatten()
            .all(|vertex| vertex.rgba == 0xffff_ffff));
    }

    /// Without a declared render type the atlas sprite's alpha flags and the
    /// material's forced translucency choose the mode, as in the scene exporter.
    #[test]
    fn absent_render_type_uses_atlas_alpha_and_force_translucent() {
        let atlas = atlas(
            4,
            4,
            &[
                ("minecraft:block/opaque", rect(0, 0, 1, 1)),
                (
                    "minecraft:block/cut",
                    SpriteRect {
                        has_alpha: true,
                        ..rect(1, 0, 1, 1)
                    },
                ),
                (
                    "minecraft:block/mixed",
                    SpriteRect {
                        has_alpha: true,
                        has_partial_alpha: true,
                        ..rect(2, 0, 1, 1)
                    },
                ),
            ],
        );
        let opaque = quad(UNIT, UNIT_UV, "minecraft:block/opaque", None, -1);
        let cut = quad(UNIT, UNIT_UV, "minecraft:block/cut", None, -1);
        let mixed = quad(UNIT, UNIT_UV, "minecraft:block/mixed", None, -1);
        let mut forced = quad(UNIT, UNIT_UV, "minecraft:block/opaque", None, -1);
        forced.material.force_translucent = true;

        let models = models(&[
            (0, Vec::new()),
            (1, vec![opaque]),
            (2, vec![cut]),
            (3, vec![mixed]),
            (4, vec![forced]),
        ]);
        let mut blocks = [0u32; SECTION_BLOCKS];
        for (index, id) in [1u32, 2, 3, 4].into_iter().enumerate() {
            blocks[index] = id;
        }

        let mesh = mesh_section(&blocks, &models, &atlas).unwrap();
        assert_eq!(mesh.groups[0].len(), 6);
        assert_eq!(mesh.groups[1].len(), 6);
        assert_eq!(mesh.groups[2].len(), 12);
    }

    /// Missing entries (id 0 included), missing sprites, unsupported render
    /// types, bad positions, repeating UVs and the vertex cap all fail the whole
    /// mesh; roundoff UVs clamp to the sprite instead.
    #[test]
    fn missing_and_invalid_inputs_are_refused() {
        let atlas = atlas(2, 2, &[("minecraft:block/a", rect(0, 0, 2, 2))]);
        let blocks = [1u32; SECTION_BLOCKS];

        assert!(matches!(
            mesh_section(&blocks, &BTreeMap::new(), &atlas),
            Err(SectionError::MissingModel(1))
        ));
        assert!(matches!(
            mesh_section(&[0u32; SECTION_BLOCKS], &BTreeMap::new(), &atlas),
            Err(SectionError::MissingModel(0))
        ));
        assert!(matches!(
            mesh_section(&blocks[..SECTION_BLOCKS - 1], &models(&[]), &atlas),
            Err(SectionError::BlockCount(count)) if count == SECTION_BLOCKS - 1
        ));

        let bad_atlas = Atlas {
            width: 3,
            height: 2,
            rgba: vec![0; 24],
            sprites: BTreeMap::new(),
        };
        assert!(matches!(
            mesh_section(
                &[0u32; SECTION_BLOCKS],
                &models(&[(0, Vec::new())]),
                &bad_atlas
            ),
            Err(SectionError::Atlas(_))
        ));

        let missing_sprite = models(&[(
            1,
            vec![quad(
                UNIT,
                UNIT_UV,
                "minecraft:block/absent",
                Some(RenderType::Solid),
                -1,
            )],
        )]);
        assert!(matches!(
            mesh_section(&blocks, &missing_sprite, &atlas),
            Err(SectionError::MissingSprite(sprite)) if sprite == "minecraft:block/absent"
        ));

        let outside = Atlas {
            width: 2,
            height: 2,
            rgba: vec![0; 16],
            sprites: BTreeMap::from([("minecraft:block/a".to_owned(), rect(1, 1, 2, 2))]),
        };
        assert!(matches!(
            mesh_section(
                &blocks,
                &models(&[(
                    1,
                    vec![quad(UNIT, UNIT_UV, "minecraft:block/a", Some(RenderType::Solid), -1)]
                )]),
                &outside
            ),
            Err(SectionError::SpriteOutsideAtlas(sprite)) if sprite == "minecraft:block/a"
        ));

        assert!(matches!(
            mesh_section(
                &blocks,
                &models(&[(
                    1,
                    vec![quad(
                        UNIT,
                        UNIT_UV,
                        "minecraft:block/a",
                        Some(RenderType::Unsupported),
                        -1
                    )]
                )]),
                &atlas
            ),
            Err(SectionError::UnsupportedRenderType { .. })
        ));

        let mut non_finite = UNIT;
        non_finite[0][1] = f32::NAN;
        assert!(matches!(
            mesh_section(
                &blocks,
                &models(&[(
                    1,
                    vec![quad(
                        non_finite,
                        UNIT_UV,
                        "minecraft:block/a",
                        Some(RenderType::Solid),
                        -1
                    )]
                )]),
                &atlas
            ),
            Err(SectionError::InvalidPosition { .. })
        ));

        let mut beyond = UNIT;
        beyond[2][0] = MAX_COORDINATE + 0.5;
        assert!(matches!(
            mesh_section(
                &blocks,
                &models(&[(
                    1,
                    vec![quad(
                        beyond,
                        UNIT_UV,
                        "minecraft:block/a",
                        Some(RenderType::Solid),
                        -1
                    )]
                )]),
                &atlas
            ),
            Err(SectionError::InvalidPosition { .. })
        ));

        for bad in [1.01f32, -0.01, f32::NAN] {
            let mut uv = UNIT_UV;
            uv[1][0] = bad;
            assert!(
                matches!(
                    mesh_section(
                        &blocks,
                        &models(&[(
                            1,
                            vec![quad(
                                UNIT,
                                uv,
                                "minecraft:block/a",
                                Some(RenderType::Solid),
                                -1
                            )]
                        )]),
                        &atlas
                    ),
                    Err(SectionError::InvalidUv { .. })
                ),
                "UV {bad} was accepted"
            );
        }

        // Roundoff just outside the sprite clamps instead of failing.
        let mut roundoff = UNIT_UV;
        roundoff[0][0] = -5e-7;
        roundoff[1][0] = 1.0 + 5e-7;
        let mesh = mesh_section(
            &blocks,
            &models(&[(
                1,
                vec![quad(
                    UNIT,
                    roundoff,
                    "minecraft:block/a",
                    Some(RenderType::Solid),
                    -1,
                )],
            )]),
            &atlas,
        )
        .unwrap();
        assert_eq!(mesh.groups[0][0].uv, [0.0, 0.0]);
        assert_eq!(mesh.groups[0][1].uv, [1.0, 0.0]);

        // 4096 cells with eleven quads each would pass 262140 vertices.
        let heavy = models(&[(
            1,
            (0..11)
                .map(|_| {
                    quad(
                        UNIT,
                        UNIT_UV,
                        "minecraft:block/a",
                        Some(RenderType::Solid),
                        -1,
                    )
                })
                .collect(),
        )]);
        assert!(matches!(
            mesh_section(&blocks, &heavy, &atlas),
            Err(SectionError::VertexLimit(reached)) if reached > MAX_VERTICES
        ));
    }

    /// The encoded bytes are the existing `CTTEX001` layout, field by field.
    #[test]
    fn encoded_scene_matches_the_existing_cttex_layout_byte_for_byte() {
        let rgba: Vec<u8> = (1..=16).collect();
        let atlas = Atlas {
            width: 2,
            height: 2,
            rgba,
            sprites: BTreeMap::new(),
        };
        let mut groups: [Vec<Vertex>; 3] = core::array::from_fn(|_| Vec::new());
        groups[0] = quad_vertices(
            [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0],
            ],
            [[0.0, 0.0], [0.5, 0.0], [0.5, 0.5], [0.0, 0.5]],
        );
        groups[2] = quad_vertices(
            [
                [0.0, 0.0, 0.0],
                [-1.0, 0.0, 0.0],
                [-1.0, 2.0, 0.0],
                [0.0, 2.0, 0.0],
            ],
            [[0.0, 1.0], [0.5, 1.0], [0.5, 0.5], [0.0, 0.5]],
        );
        let mesh = SectionMesh {
            groups,
            cells_with_geometry: 2,
            empty_cells: SECTION_BLOCKS - 2,
            source_quads: 2,
            tinted_quads: 0,
        };

        let bytes = encode_scene(&mesh, &atlas).unwrap();
        assert_eq!(bytes.len(), 400);
        assert_eq!(&bytes[..8], b"CTTEX001");
        assert_eq!(le_u32(&bytes, 8), 777);
        assert_eq!(le_u32(&bytes, 12), 1);
        assert_eq!(le_u32(&bytes, 16), 12);
        assert_eq!(le_u32(&bytes, 20), 2);
        assert_eq!(le_u32(&bytes, 24), 2);
        assert_eq!(le_u32(&bytes, 28), 2);
        assert_eq!(le_u32(&bytes, 32), 0);
        assert_eq!(le_u32(&bytes, 36), 0);
        for (offset, expected) in [40, 44, 48, 52, 56, 60]
            .into_iter()
            .zip([-1.0f32, 0.0, 0.0, 1.0, 2.0, 1.0])
        {
            assert_eq!(le_f32(&bytes, offset), expected);
        }

        let vertices = mesh.groups.iter().flatten().collect::<Vec<_>>();
        assert_eq!(&bytes[64..88], le_vertex(vertices[0]).as_slice());
        assert_eq!(
            &bytes[64 + 11 * 24..64 + 12 * 24],
            le_vertex(vertices[11]).as_slice()
        );

        assert_eq!(
            &bytes[352..368],
            &[0, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            &bytes[368..384],
            &[6, 0, 0, 0, 6, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(&bytes[384..400], (1..=16u8).collect::<Vec<u8>>().as_slice());
    }

    /// Encoding refuses an empty mesh, ragged mode groups, invalid vertices and
    /// an atlas that cannot be the exact RGBA8 payload; roundoff UVs clamp.
    #[test]
    fn encode_refuses_empty_ragged_and_invalid_meshes() {
        let atlas = atlas(2, 2, &[]);
        let mut mesh = SectionMesh {
            groups: core::array::from_fn(|_| Vec::new()),
            cells_with_geometry: 0,
            empty_cells: SECTION_BLOCKS,
            source_quads: 0,
            tinted_quads: 0,
        };
        assert!(matches!(
            encode_scene(&mesh, &atlas),
            Err(SectionError::EmptyMesh)
        ));

        mesh.groups[1] = vec![vertex([0.0; 3], [0.0; 2]); 3];
        assert!(matches!(
            encode_scene(&mesh, &atlas),
            Err(SectionError::RaggedGroup(1))
        ));

        mesh.groups[1] = vec![vertex([0.0; 3], [0.0; 2]); 6];
        assert!(matches!(
            encode_scene(&mesh, &atlas),
            Err(SectionError::InvalidVertex("degenerate bounds"))
        ));

        let mut non_finite = quad_vertices(UNIT, UNIT_UV);
        non_finite[0].p[2] = f32::NAN;
        mesh.groups[1] = non_finite;
        assert!(matches!(
            encode_scene(&mesh, &atlas),
            Err(SectionError::InvalidVertex("position"))
        ));

        let mut beyond = quad_vertices(UNIT, UNIT_UV);
        beyond[5].p[0] = -(MAX_COORDINATE + 1.0);
        mesh.groups[1] = beyond;
        assert!(matches!(
            encode_scene(&mesh, &atlas),
            Err(SectionError::InvalidVertex("position"))
        ));

        let mut repeating = quad_vertices(UNIT, UNIT_UV);
        repeating[3].uv[0] = 1.001;
        mesh.groups[1] = repeating;
        assert!(matches!(
            encode_scene(&mesh, &atlas),
            Err(SectionError::InvalidVertex("UV"))
        ));

        // Roundoff clamps in the encoded bytes.
        let mut roundoff = quad_vertices(UNIT, UNIT_UV);
        roundoff[0].uv[0] = -5e-7;
        roundoff[1].uv[0] = 1.0 + 5e-7;
        mesh.groups[1] = roundoff;
        let bytes = encode_scene(&mesh, &atlas).unwrap();
        assert_eq!(le_f32(&bytes, 64 + 12), 0.0);
        assert_eq!(le_f32(&bytes, 64 + 24 + 12), 1.0);

        let oversized = Atlas {
            width: 2048,
            height: 2,
            rgba: vec![0; 2048 * 2 * 4],
            sprites: BTreeMap::new(),
        };
        mesh.groups[1] = quad_vertices(UNIT, UNIT_UV);
        assert!(matches!(
            encode_scene(&mesh, &oversized),
            Err(SectionError::Atlas(_))
        ));

        let short = Atlas {
            width: 2,
            height: 2,
            rgba: vec![0; 15],
            sprites: BTreeMap::new(),
        };
        assert!(matches!(
            encode_scene(&mesh, &short),
            Err(SectionError::Atlas(_))
        ));
    }
}
