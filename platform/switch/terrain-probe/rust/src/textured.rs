//! Allocation-free static textured scene decoder and camera-space quad sort.
//! No SDK, filesystem, texture decoding or world semantics.
use super::{float, valid_bounds, word, MeshInfo, MAX_VERTICES};
use core::{mem, slice};

pub const MAX_DRAWS: usize = 256;
pub const MAX_FILE: usize = 16 * 1024 * 1024;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextureVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub rgba: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Draw {
    pub first: u32,
    pub count: u32,
    pub mode: u32,
    pub reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SceneInfo {
    pub mesh: MeshInfo,
    pub draw_count: u32,
    pub width: u32,
    pub height: u32,
    pub atlas_offset: u32,
    pub atlas_bytes: u32,
}
const _: () = {
    assert!(mem::size_of::<TextureVertex>() == 24);
    assert!(mem::offset_of!(TextureVertex, rgba) == 20);
    assert!(mem::size_of::<Draw>() == 16);
    assert!(mem::size_of::<SceneInfo>() == 48);
};

fn valid_draws(draws: &[Draw], count: usize) -> bool {
    let mut end = 0usize;
    let mut previous = None;
    for d in draws {
        if d.first as usize != end
            || d.count == 0
            || !d.count.is_multiple_of(3)
            || d.mode > 2
            || d.reserved != 0
            || previous.is_some_and(|p| d.mode <= p)
            || (d.mode == 2 && !d.count.is_multiple_of(6))
        {
            return false;
        }
        end += d.count as usize;
        if end > count {
            return false;
        }
        previous = Some(d.mode);
    }
    end == count
}

/// Decode only after validating every byte and all output capacities.
///
/// # Safety
/// data is readable for len bytes; outputs are aligned, writable for their
/// capacities and disjoint from each other and input. No retained pointers.
#[no_mangle]
pub unsafe extern "C" fn craftrustnx_texture_decode(
    data: *const u8,
    len: usize,
    vertices: *mut TextureVertex,
    vertex_capacity: usize,
    draws: *mut Draw,
    draw_capacity: usize,
    info: *mut SceneInfo,
) -> i32 {
    if info.is_null() {
        return -1;
    }
    unsafe { info.write(SceneInfo::default()) };
    if data.is_null() || vertices.is_null() || draws.is_null() {
        return -1;
    }
    if !(64..=MAX_FILE).contains(&len) {
        return -3;
    }
    let b = unsafe { slice::from_raw_parts(data, len) };
    if &b[..8] != b"CTTEX001"
        || word(b, 8) != 777
        || word(b, 12) != 1
        || word(b, 32) != 0
        || word(b, 36) != 0
    {
        return -3;
    }
    let count = word(b, 16) as usize;
    let draw_count = word(b, 20) as usize;
    let width = word(b, 24);
    let height = word(b, 28);
    if count == 0
        || count > MAX_VERTICES
        || !count.is_multiple_of(3)
        || draw_count == 0
        || draw_count > MAX_DRAWS
        || !width.is_power_of_two()
        || !height.is_power_of_two()
        || width > 1024
        || height > 1024
    {
        return -3;
    }
    let draw_offset = 64 + count * 24;
    let atlas_offset = draw_offset + draw_count * 16;
    let atlas_bytes = width as usize * height as usize * 4;
    if len != atlas_offset + atlas_bytes {
        return -3;
    }
    if vertex_capacity < count || draw_capacity < draw_count {
        return -2;
    }
    let mesh = MeshInfo {
        vertex_count: count as u32,
        min: core::array::from_fn(|i| float(b, 40 + i * 4)),
        max: core::array::from_fn(|i| float(b, 52 + i * 4)),
    };
    if !valid_bounds(&mesh) {
        return -3;
    }
    let mut ranges = [Draw::default(); MAX_DRAWS];
    for (i, d) in ranges[..draw_count].iter_mut().enumerate() {
        let off = draw_offset + i * 16;
        *d = Draw {
            first: word(b, off),
            count: word(b, off + 4),
            mode: word(b, off + 8),
            reserved: word(b, off + 12),
        };
    }
    if !valid_draws(&ranges[..draw_count], count) {
        return -3;
    }
    let vertex_at = |i: usize| TextureVertex {
        position: core::array::from_fn(|axis| float(b, 64 + i * 24 + axis * 4)),
        uv: [float(b, 64 + i * 24 + 12), float(b, 64 + i * 24 + 16)],
        rgba: word(b, 64 + i * 24 + 20),
    };
    for i in 0..count {
        let v = vertex_at(i);
        if !(0..3).all(|a| {
            v.position[a].is_finite()
                && v.position[a] >= mesh.min[a]
                && v.position[a] <= mesh.max[a]
        }) || !v
            .uv
            .iter()
            .all(|f| f.is_finite() && (0.0..=1.0).contains(f))
        {
            return -3;
        }
    }
    for i in 0..count {
        unsafe { vertices.add(i).write(vertex_at(i)) };
    }
    unsafe {
        core::ptr::copy_nonoverlapping(ranges.as_ptr(), draws, draw_count);
        info.write(SceneInfo {
            mesh,
            draw_count: draw_count as u32,
            width,
            height,
            atlas_offset: atlas_offset as u32,
            atlas_bytes: atlas_bytes as u32,
        });
    }
    0
}

fn depth(q: &[TextureVertex; 6], m: &[f32; 16]) -> f32 {
    [0, 1, 2, 5]
        .iter()
        .map(|&i| {
            let p = q[i].position;
            (m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15]) * 0.25
        })
        .sum()
}

/// Sort translucent quads from far to near; no allocation or state retention.
///
/// # Safety
/// All pointers are aligned, valid for the declared counts, non-overlapping;
/// vertices are writable. Only canonical decoder-approved draw layouts pass.
#[no_mangle]
pub unsafe extern "C" fn craftrustnx_texture_sort(
    vertices: *mut TextureVertex,
    count: u32,
    draws: *const Draw,
    draw_count: u32,
    mvp: *const f32,
) -> i32 {
    if vertices.is_null() || draws.is_null() || mvp.is_null() {
        return -1;
    }
    if count == 0 || count as usize > MAX_VERTICES || draw_count as usize > MAX_DRAWS {
        return -3;
    }
    let m = unsafe { *(mvp.cast::<[f32; 16]>()) };
    let ranges = unsafe { slice::from_raw_parts(draws, draw_count as usize) };
    if !m.iter().all(|f| f.is_finite()) || !valid_draws(ranges, count as usize) {
        return -3;
    }
    let v = unsafe { slice::from_raw_parts_mut(vertices, count as usize) };
    if !v.iter().all(|v| {
        v.position
            .iter()
            .all(|f| f.is_finite() && f.abs() <= 1024.0)
    }) {
        return -3;
    }
    for d in ranges.iter().filter(|d| d.mode == 2) {
        let range = d.first as usize..(d.first + d.count) as usize;
        if !v[range]
            .as_chunks::<6>()
            .0
            .iter()
            .all(|q| depth(q, &m).is_finite())
        {
            return -3;
        }
    }
    for d in ranges.iter().filter(|d| d.mode == 2) {
        let range = d.first as usize..(d.first + d.count) as usize;
        v[range]
            .as_chunks_mut::<6>()
            .0
            .sort_unstable_by(|a, b| depth(b, &m).partial_cmp(&depth(a, &m)).unwrap());
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> std::vec::Vec<u8> {
        let mut b = b"CTTEX001".to_vec();
        for n in [777u32, 1, 6, 1, 1, 1, 0, 0] {
            b.extend(n.to_le_bytes());
        }
        for f in [0.0f32, 0.0, 0.0, 1.0, 1.0, 1.0] {
            b.extend(f.to_le_bytes());
        }
        for _ in 0..6 {
            for f in [0.0f32, 0.0, 0.0, 0.5, 0.5] {
                b.extend(f.to_le_bytes());
            }
            b.extend(0xffff_ffffu32.to_le_bytes());
        }
        for n in [0u32, 6, 2, 0] {
            b.extend(n.to_le_bytes());
        }
        b.extend([1, 2, 3, 255]);
        b
    }
    #[test]
    fn exact_scene_decodes_and_all_invalid_inputs_are_transactional() {
        let good = fixture();
        let mut v = [TextureVertex::default(); 6];
        let mut d = [Draw::default(); 1];
        let mut i = SceneInfo::default();
        assert_eq!(
            unsafe {
                craftrustnx_texture_decode(
                    good.as_ptr(),
                    good.len(),
                    v.as_mut_ptr(),
                    6,
                    d.as_mut_ptr(),
                    1,
                    &mut i,
                )
            },
            0
        );
        assert_eq!(i.atlas_offset, 224);
        assert_eq!(i.atlas_bytes, 4);
        assert_eq!(v[0].uv, [0.5; 2]);
        assert_eq!(d[0].mode, 2);
        let mut bad = std::vec::Vec::new();
        for n in 0..good.len() {
            bad.push(good[..n].to_vec());
        }
        for (off, n) in [
            (8, 776u32),
            (24, 3),
            (32, 1),
            (64, f32::NAN.to_bits()),
            (76, 2.0f32.to_bits()),
            (208, 3),
            (216, 9),
        ] {
            let mut b = good.clone();
            b[off..off + 4].copy_from_slice(&n.to_le_bytes());
            bad.push(b);
        }
        for b in bad {
            let before_v = v;
            let before_d = d;
            assert_ne!(
                unsafe {
                    craftrustnx_texture_decode(
                        b.as_ptr(),
                        b.len(),
                        v.as_mut_ptr(),
                        6,
                        d.as_mut_ptr(),
                        1,
                        &mut i,
                    )
                },
                0
            );
            assert_eq!(v, before_v);
            assert_eq!(d, before_d);
            assert_eq!(i, SceneInfo::default());
        }
        assert_eq!(
            unsafe {
                craftrustnx_texture_decode(
                    good.as_ptr(),
                    good.len(),
                    v.as_mut_ptr(),
                    5,
                    d.as_mut_ptr(),
                    1,
                    &mut i,
                )
            },
            -2
        );
    }
    #[test]
    fn sorting_moves_whole_translucent_quads_and_preserves_opaque() {
        let mut v = [TextureVertex::default(); 18];
        for (n, q) in v.as_chunks_mut::<6>().0.iter_mut().enumerate() {
            for p in q {
                p.position[2] = n as f32;
                p.rgba = n as u32;
            }
        }
        let d = [
            Draw {
                first: 0,
                count: 6,
                mode: 0,
                reserved: 0,
            },
            Draw {
                first: 6,
                count: 12,
                mode: 2,
                reserved: 0,
            },
        ];
        let mut m = [0.0; 16];
        m[11] = 1.0;
        assert_eq!(
            unsafe { craftrustnx_texture_sort(v.as_mut_ptr(), 18, d.as_ptr(), 2, m.as_ptr()) },
            0
        );
        assert!(v[..6].iter().all(|p| p.rgba == 0));
        assert!(v[6..12].iter().all(|p| p.rgba == 2));
        assert!(v[12..].iter().all(|p| p.rgba == 1));
        let before = v;
        m[0] = f32::NAN;
        assert_eq!(
            unsafe { craftrustnx_texture_sort(v.as_mut_ptr(), 18, d.as_ptr(), 2, m.as_ptr()) },
            -3
        );
        assert_eq!(v, before);
    }
}
