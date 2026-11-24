//! Private, bounded model templates for the allocation-free Switch mesher.
//! Reuses the section emitter's exact UV/mode/triangulation implementation.
use crate::{section::mesh_section, Atlas};
use horizon_assets::bake::ModelQuad;
use std::collections::BTreeMap;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn encode_model_bank(models: &BTreeMap<u32, Vec<ModelQuad>>, atlas: &Atlas) -> Result<Vec<u8>> {
    if models.is_empty() || models.len() > 65536 {
        return Err("model bank state count".into());
    }
    let mut records = Vec::with_capacity(models.len() * 32);
    let mut templates = Vec::new();
    let mut shared = BTreeMap::<Vec<u8>, u32>::new();
    for (&id, quads) in models {
        // A synthetic origin cell plus explicitly empty padding reuses the
        // tested emitter without duplicating geometry/UV/alpha math. These are
        // tool-local keys, never vanilla IDs or an inference about ID0/ID1.
        let mut cells = [1; 4096];
        cells[0] = 0;
        let local = BTreeMap::from([(0, quads.clone()), (1, Vec::new())]);
        let mesh = mesh_section(&cells, &local, atlas)?;
        records.extend(id.to_le_bytes());
        for group in &mesh.groups {
            let mut bytes = Vec::with_capacity(group.len() * 24);
            for vertex in group {
                for value in vertex.p.into_iter().chain(vertex.uv) {
                    bytes.extend(value.to_le_bytes());
                }
                bytes.extend(vertex.rgba.to_le_bytes());
            }
            let first = if bytes.is_empty() {
                0
            } else if let Some(first) = shared.get(&bytes) {
                *first
            } else {
                let first = templates.len() / 24;
                if first + group.len() > 262140 {
                    return Err("model bank template limit".into());
                }
                templates.extend_from_slice(&bytes);
                shared.insert(bytes, first as u32);
                first as u32
            };
            records.extend(first.to_le_bytes());
            records.extend((group.len() as u32).to_le_bytes());
        }
        records.extend(0u32.to_le_bytes());
    }
    let size = 64 + records.len() + templates.len() + atlas.rgba.len();
    if size > 16 * 1024 * 1024 {
        return Err("model bank file limit".into());
    }
    let mut out = Vec::with_capacity(size);
    out.extend(b"CTBANK01");
    for word in [
        777,
        1,
        models.len() as u32,
        (templates.len() / 24) as u32,
        atlas.width,
        atlas.height,
        32,
        24,
        0,
        0,
        atlas.rgba.len() as u32,
        0,
        0,
        0,
    ] {
        out.extend(word.to_le_bytes());
    }
    out.extend(records);
    out.extend(templates);
    out.extend_from_slice(&atlas.rgba);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SpriteRect;
    use horizon_assets::{
        bake::RenderType,
        model::{Direction, Material},
    };
    #[test]
    fn exact_bank_layout_and_template_aliases() {
        let atlas = Atlas {
            width: 2,
            height: 2,
            rgba: (1..=16).collect(),
            sprites: BTreeMap::from([(
                "test:a".to_owned(),
                SpriteRect {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 2,
                    has_alpha: false,
                    has_partial_alpha: false,
                },
            )]),
        };
        let q = ModelQuad {
            positions: [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]],
            uv: [[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
            facing: Direction::North,
            cull: None,
            material: Material {
                sprite: "test:a".into(),
                force_translucent: false,
            },
            tint_index: -1,
            shade_direction_override: None,
            light_emission: 0,
            light: [0, 0],
            color: None,
            ambient_occlusion: None,
            render_type: Some(RenderType::Solid),
            shade: true,
        };
        let models = BTreeMap::from([(0, vec![q.clone()]), (17, vec![q]), (u32::MAX, vec![])]);
        let b = encode_model_bank(&models, &atlas).unwrap();
        assert_eq!(b.len(), 320);
        assert_eq!(&b[..8], b"CTBANK01");
        assert_eq!(&b[16..24], &[3, 0, 0, 0, 6, 0, 0, 0]);
        assert_eq!(&b[32..40], &[32, 0, 0, 0, 24, 0, 0, 0]);
        assert_eq!(&b[64..76], &[0, 0, 0, 0, 0, 0, 0, 0, 6, 0, 0, 0]);
        assert_eq!(&b[96..108], &[17, 0, 0, 0, 0, 0, 0, 0, 6, 0, 0, 0]);
        assert_eq!(&b[128..132], &[255; 4]);
        assert_eq!(&b[304..], &(1..=16u8).collect::<Vec<_>>());
        assert!(encode_model_bank(&BTreeMap::new(), &atlas).is_err());
    }
}
