//! Allocation-free CPU section mesher for the Switch terrain probe.
//!
//! One `CTCOL001` capture section (4096 raw block-state IDs in Y/Z/X order) is
//! meshed through a bounded `CTBANK01` model bank: sorted per-ID records with
//! three template ranges and one shared static atlas. Quads stay whole;
//! positions are translated by the cell's local `[x, y, z]`, UVs and RGBA words
//! are copied unchanged. No allocation, SDK, filesystem, RNG/jitter, occlusion
//! culling, lighting, tint/colour guessing or cube fallback.
//!
//! `platform/switch/texture-probe/include/column_mesh.h` is the normative contract; this module only
//! implements it. The pure capture codec is the login probe's shared decoder,
//! so the `CTCOL001` header rules cannot fork here.

use super::textured::{Draw, SceneInfo, TextureVertex, MAX_DRAWS};
use super::{float, word, MeshInfo, MAX_VERTICES};
use core::{mem, slice};

// Shared with `login-probe/codec`; only the header rules and exact file size are
// used, the 4096 selected cells are read in place. Members this module does not
// need (encoders, capacity constants) stay allowed rather than forked.
#[allow(dead_code)]
#[path = "../../../login-probe/codec/src/snapshot.rs"]
mod snapshot;

/// `CTCOL001` header size; the selected section starts right after it.
const CAPTURE_HEADER: usize = snapshot::HEADER_SIZE;
/// Largest complete console snapshot (48 + 256 * (4096 + 64) * 4 bytes).
const MAX_CAPTURE: usize = snapshot::MAX_FILE_SIZE;
/// Hard cap on one encoded model bank.
const MAX_BANK: usize = 16 * 1024 * 1024;
/// `CTBANK01` header size.
const BANK_HEADER: usize = 64;
/// State record stride fixed by the header.
const STATE_RECORD: usize = 32;
/// Template vertex stride fixed by the header.
const TEMPLATE_VERTEX: usize = 24;
/// Largest accepted state count.
const MAX_STATES: usize = 65_536;
/// Largest accepted power-of-two atlas width or height.
const MAX_ATLAS_DIMENSION: u32 = 1024;
/// One section is exactly 16x16x16 cells.
const SECTION_CELLS: usize = 4096;
/// Largest accepted absolute coordinate on any axis, in block units.
const MAX_COORDINATE: f32 = 1024.0;

/// `struct cm_info` from `column_mesh.h`: the textured scene description plus
/// the section and per-cell counters. Field order, size (72) and offsets are
/// part of the C ABI.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CmInfo {
    pub scene: SceneInfo,
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub section_min_y: i32,
    pub section_index: u32,
    pub cells_with_geometry: u32,
    pub empty_cells: u32,
}
const _: () = {
    assert!(mem::size_of::<CmInfo>() == 72);
    assert!(mem::offset_of!(CmInfo, scene) == 0);
    assert!(mem::offset_of!(CmInfo, chunk_x) == 48);
    assert!(mem::offset_of!(CmInfo, chunk_z) == 52);
    assert!(mem::offset_of!(CmInfo, section_min_y) == 56);
    assert!(mem::offset_of!(CmInfo, section_index) == 60);
    assert!(mem::offset_of!(CmInfo, cells_with_geometry) == 64);
    assert!(mem::offset_of!(CmInfo, empty_cells) == 68);
};

/// Bank header fields that later passes rely on.
struct Bank {
    states: usize,
    template_offset: usize,
    atlas_offset: usize,
    atlas_bytes: usize,
    width: u32,
    height: u32,
}

/// Everything a successful build needs, computed before any output write.
struct Plan {
    /// Vertices per mode group, index 0 solid, 1 cutout, 2 translucent.
    totals: [usize; 3],
    vertices: usize,
    draws: usize,
    cells_with_geometry: u32,
    empty_cells: u32,
    min: [f32; 3],
    max: [f32; 3],
}

/// Validate the complete `CTBANK01` bank: header, sorted records, every
/// template range, every template vertex and the exact atlas payload length.
fn parse_bank(bank: &[u8]) -> Option<Bank> {
    if bank.len() < BANK_HEADER {
        return None;
    }
    if &bank[..8] != b"CTBANK01" || word(bank, 8) != 777 || word(bank, 12) != 1 {
        return None;
    }
    let states = word(bank, 16) as usize;
    let template_vertices = word(bank, 20) as usize;
    let width = word(bank, 24);
    let height = word(bank, 28);
    if states == 0
        || states > MAX_STATES
        || template_vertices > MAX_VERTICES
        || !template_vertices.is_multiple_of(6)
        || !width.is_power_of_two()
        || !height.is_power_of_two()
        || width > MAX_ATLAS_DIMENSION
        || height > MAX_ATLAS_DIMENSION
        || word(bank, 32) as usize != STATE_RECORD
        || word(bank, 36) as usize != TEMPLATE_VERTEX
        || word(bank, 40) != 0
        || word(bank, 44) != 0
        || word(bank, 52) != 0
        || word(bank, 56) != 0
        || word(bank, 60) != 0
    {
        return None;
    }
    let atlas_bytes = width as usize * height as usize * 4;
    if word(bank, 48) as usize != atlas_bytes {
        return None;
    }
    let template_offset = BANK_HEADER.checked_add(states * STATE_RECORD)?;
    let atlas_offset = template_offset.checked_add(template_vertices * TEMPLATE_VERTEX)?;
    if atlas_offset.checked_add(atlas_bytes)? != bank.len() {
        return None;
    }

    // Records: strictly ascending raw IDs, zero reserved word, and three
    // complete six-vertex ranges that lie inside the template array. Ranges and
    // whole groups may alias each other.
    let mut previous: Option<u32> = None;
    for index in 0..states {
        let record = BANK_HEADER + index * STATE_RECORD;
        let id = word(bank, record);
        if previous.is_some_and(|previous| id <= previous) || word(bank, record + 28) != 0 {
            return None;
        }
        previous = Some(id);
        for mode in 0..3 {
            let (first, count) = record_range(bank, index, mode);
            if first > template_vertices
                || !first.is_multiple_of(6)
                || !count.is_multiple_of(6)
                || count > template_vertices - first
            {
                return None;
            }
        }
    }

    // Every template vertex is finite, inside the coordinate limit and carries
    // an in-range UV; the RGBA word is opaque data.
    for index in 0..template_vertices {
        let offset = template_offset + index * TEMPLATE_VERTEX;
        let positions_ok = (0..3).all(|axis| {
            let value = float(bank, offset + axis * 4);
            value.is_finite() && value.abs() <= MAX_COORDINATE
        });
        let uv_ok = (0..2).all(|axis| {
            let value = float(bank, offset + 12 + axis * 4);
            value.is_finite() && (0.0..=1.0).contains(&value)
        });
        if !positions_ok || !uv_ok {
            return None;
        }
    }

    Some(Bank {
        states,
        template_offset,
        atlas_offset,
        atlas_bytes,
        width,
        height,
    })
}

/// Binary search the sorted records for a raw ID; ID 0 is looked up like any
/// other value and never assumed to be air.
fn find_record(bank: &[u8], states: usize, id: u32) -> Option<usize> {
    let mut low = 0usize;
    let mut high = states;
    while low < high {
        let middle = low + (high - low) / 2;
        let found = word(bank, BANK_HEADER + middle * STATE_RECORD);
        if found == id {
            return Some(middle);
        }
        if found < id {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    None
}

/// `(first, count)` of one mode range; the bank header was validated first.
fn record_range(bank: &[u8], record: usize, mode: usize) -> (usize, usize) {
    let offset = BANK_HEADER + record * STATE_RECORD + 4 + mode * 8;
    (word(bank, offset) as usize, word(bank, offset + 4) as usize)
}

/// Local cell translation for a Y/Z/X index: `x = index % 16`,
/// `z = index / 16 % 16`, `y = index / 256`.
fn cell_offset(index: usize) -> [f32; 3] {
    [
        (index % 16) as f32,
        (index / 256) as f32,
        (index / 16 % 16) as f32,
    ]
}

/// Resolve every selected cell and validate the complete output plan without
/// touching caller buffers.
fn plan_section(capture: &[u8], blocks: usize, bank: &[u8], parsed: &Bank) -> Result<Plan, i32> {
    let mut totals = [0usize; 3];
    let mut vertices = 0usize;
    let mut cells_with_geometry = 0u32;
    let mut empty_cells = 0u32;
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];

    for index in 0..SECTION_CELLS {
        let id = word(capture, blocks + index * 4);
        let record = find_record(bank, parsed.states, id).ok_or(-5)?;
        let offset = cell_offset(index);
        let mut ranges = [(0usize, 0usize); 3];
        let mut cell_vertices = 0usize;
        for (mode, range) in ranges.iter_mut().enumerate() {
            *range = record_range(bank, record, mode);
            cell_vertices += range.1;
        }
        // Bound the work before scanning positions so oversized banks fail fast.
        if vertices + cell_vertices > MAX_VERTICES {
            return Err(-3);
        }
        vertices += cell_vertices;
        if cell_vertices == 0 {
            empty_cells += 1;
            continue;
        }
        cells_with_geometry += 1;
        for (mode, &(first, count)) in ranges.iter().enumerate() {
            totals[mode] += count;
            for template in first..first + count {
                let at = parsed.template_offset + template * TEMPLATE_VERTEX;
                let position = [
                    float(bank, at) + offset[0],
                    float(bank, at + 4) + offset[1],
                    float(bank, at + 8) + offset[2],
                ];
                for (axis, value) in position.iter().enumerate() {
                    if !value.is_finite() || value.abs() > MAX_COORDINATE {
                        return Err(-3);
                    }
                    min[axis] = min[axis].min(*value);
                    max[axis] = max[axis].max(*value);
                }
            }
        }
    }

    if vertices == 0 {
        return Err(-5);
    }
    // The orbit camera refuses an entirely collapsed bounding box.
    if !(0..3).any(|axis| min[axis] < max[axis]) {
        return Err(-3);
    }
    Ok(Plan {
        totals,
        vertices,
        draws: totals.iter().filter(|count| **count != 0).count(),
        cells_with_geometry,
        empty_cells,
        min,
        max,
    })
}

/// Mesh one captured section through a compact model bank.
///
/// Query mode is exactly `vertices == NULL && draws == NULL` with both
/// capacities zero; it validates everything and writes only `info`. Otherwise
/// both outputs must be non-null with truthful capacities. Every metadata,
/// range, UV, position, bounds, size and capacity check happens before the first
/// output write, `info` is zeroed on every failure and the vertex/draw buffers
/// are left untouched on failure.
///
/// # Safety
/// `info` is aligned and writable. `capture`/`bank` are readable for their
/// declared lengths. In build mode the outputs are aligned, writable for their
/// stated capacities (in elements), disjoint from each other and from the
/// inputs, which stay unmodified for the whole call.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn craftrustnx_column_mesh(
    capture: *const u8,
    capture_len: usize,
    bank: *const u8,
    bank_len: usize,
    section: u32,
    vertices: *mut TextureVertex,
    vertex_capacity: usize,
    draws: *mut Draw,
    draw_capacity: usize,
    info: *mut CmInfo,
) -> i32 {
    if info.is_null() {
        return -1;
    }
    unsafe { info.write(CmInfo::default()) };

    let query = vertices.is_null() && draws.is_null() && vertex_capacity == 0 && draw_capacity == 0;
    let build = !vertices.is_null() && !draws.is_null();
    if capture.is_null() || bank.is_null() || !(query || build) {
        return -1;
    }
    if vertex_capacity > MAX_VERTICES || draw_capacity > MAX_DRAWS {
        return -1;
    }
    if !(CAPTURE_HEADER..=MAX_CAPTURE).contains(&capture_len) {
        return -3;
    }
    // Checked before slicing: a forged length must never widen the read.
    if !(BANK_HEADER..=MAX_BANK).contains(&bank_len) {
        return -3;
    }
    let capture = unsafe { slice::from_raw_parts(capture, capture_len) };
    let bank = unsafe { slice::from_raw_parts(bank, bank_len) };

    let Some(parsed) = parse_bank(bank) else {
        return -3;
    };
    let Ok((_, _, chunk_x, chunk_z, min_y, sections, _, _)) = snapshot::decode_header(capture)
    else {
        return -3;
    };
    if capture_len != snapshot::header_size(sections) {
        return -3;
    }
    let section = section as usize;
    if section >= sections as usize {
        return -3;
    }
    let Some(section_min_y) = min_y.checked_add(section as i32 * 16) else {
        return -3;
    };
    let blocks = CAPTURE_HEADER + section * SECTION_CELLS * 4;

    let plan = match plan_section(capture, blocks, bank, &parsed) {
        Ok(plan) => plan,
        Err(status) => return status,
    };
    if build && (vertex_capacity < plan.vertices || draw_capacity < plan.draws) {
        return -2;
    }

    if build {
        let mut cursor = 0usize;
        let mut draw_first = [0u32; 3];
        for mode in 0..3 {
            draw_first[mode] = cursor as u32;
            if plan.totals[mode] == 0 {
                continue;
            }
            for index in 0..SECTION_CELLS {
                let id = word(capture, blocks + index * 4);
                let record = match find_record(bank, parsed.states, id) {
                    Some(record) => record,
                    // The plan pass proved every cell resolves and the inputs
                    // are immutable for the whole call.
                    None => return -3,
                };
                let offset = cell_offset(index);
                let (first, count) = record_range(bank, record, mode);
                for template in first..first + count {
                    let at = parsed.template_offset + template * TEMPLATE_VERTEX;
                    let vertex = TextureVertex {
                        position: [
                            float(bank, at) + offset[0],
                            float(bank, at + 4) + offset[1],
                            float(bank, at + 8) + offset[2],
                        ],
                        uv: [float(bank, at + 12), float(bank, at + 16)],
                        rgba: word(bank, at + 20),
                    };
                    unsafe { vertices.add(cursor).write(vertex) };
                    cursor += 1;
                }
            }
        }
        debug_assert_eq!(cursor, plan.vertices);

        let mut draw_cursor = 0usize;
        for mode in 0..3 {
            if plan.totals[mode] == 0 {
                continue;
            }
            unsafe {
                draws.add(draw_cursor).write(Draw {
                    first: draw_first[mode],
                    count: plan.totals[mode] as u32,
                    mode: mode as u32,
                    reserved: 0,
                });
            }
            draw_cursor += 1;
        }
        debug_assert_eq!(draw_cursor, plan.draws);
    }

    unsafe {
        info.write(CmInfo {
            scene: SceneInfo {
                mesh: MeshInfo {
                    vertex_count: plan.vertices as u32,
                    min: plan.min,
                    max: plan.max,
                },
                draw_count: plan.draws as u32,
                width: parsed.width,
                height: parsed.height,
                atlas_offset: parsed.atlas_offset as u32,
                atlas_bytes: parsed.atlas_bytes as u32,
            },
            chunk_x,
            chunk_z,
            section_min_y,
            section_index: section as u32,
            cells_with_geometry: plan.cells_with_geometry,
            empty_cells: plan.empty_cells,
        });
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    #[derive(Clone, Copy)]
    struct Tv {
        p: [f32; 3],
        uv: [f32; 2],
        rgba: u32,
    }

    const SOLID: [Tv; 6] = [
        Tv {
            p: [0.0, 0.0, 0.0],
            uv: [0.0, 0.0],
            rgba: 0xffff_ffff,
        },
        Tv {
            p: [1.0, 0.0, 0.0],
            uv: [1.0, 0.0],
            rgba: 0xffff_ffff,
        },
        Tv {
            p: [1.0, 0.0, 1.0],
            uv: [1.0, 1.0],
            rgba: 0xffff_ffff,
        },
        Tv {
            p: [0.0, 0.0, 0.0],
            uv: [0.0, 0.0],
            rgba: 0xffff_ffff,
        },
        Tv {
            p: [1.0, 0.0, 1.0],
            uv: [1.0, 1.0],
            rgba: 0xffff_ffff,
        },
        Tv {
            p: [0.0, 0.0, 1.0],
            uv: [0.0, 1.0],
            rgba: 0xffff_ffff,
        },
    ];
    const CUTOUT: [Tv; 6] = [
        Tv {
            p: [0.0, 0.0, 0.0],
            uv: [0.25, 1.0],
            rgba: 0x1122_3344,
        },
        Tv {
            p: [0.0, 1.0, 0.0],
            uv: [0.25, 0.0],
            rgba: 0x1122_3344,
        },
        Tv {
            p: [1.0, 1.0, 0.0],
            uv: [0.75, 0.0],
            rgba: 0x1122_3344,
        },
        Tv {
            p: [0.0, 0.0, 0.0],
            uv: [0.25, 1.0],
            rgba: 0x1122_3344,
        },
        Tv {
            p: [1.0, 1.0, 0.0],
            uv: [0.75, 0.0],
            rgba: 0x1122_3344,
        },
        Tv {
            p: [1.0, 0.0, 0.0],
            uv: [0.75, 1.0],
            rgba: 0x1122_3344,
        },
    ];
    const TRANSLUCENT: [Tv; 6] = [
        Tv {
            p: [0.0, 0.0, 0.0],
            uv: [0.0, 0.5],
            rgba: 0x0102_0304,
        },
        Tv {
            p: [-1.0, 0.0, 0.0],
            uv: [0.5, 0.5],
            rgba: 0x0102_0304,
        },
        Tv {
            p: [-1.0, 2.0, 0.0],
            uv: [0.5, 1.0],
            rgba: 0x0102_0304,
        },
        Tv {
            p: [0.0, 0.0, 0.0],
            uv: [0.0, 0.5],
            rgba: 0x0102_0304,
        },
        Tv {
            p: [-1.0, 2.0, 0.0],
            uv: [0.5, 1.0],
            rgba: 0x0102_0304,
        },
        Tv {
            p: [0.0, 2.0, 0.0],
            uv: [0.0, 1.0],
            rgba: 0x0102_0304,
        },
    ];

    const VERTEX_SENTINEL: TextureVertex = TextureVertex {
        position: [42.0; 3],
        uv: [0.5; 2],
        rgba: 0xdead_beef,
    };
    const DRAW_SENTINEL: Draw = Draw {
        first: 7,
        count: 8,
        mode: 9,
        reserved: 10,
    };

    fn sentinel_info() -> CmInfo {
        CmInfo {
            scene: SceneInfo {
                mesh: MeshInfo {
                    vertex_count: 3,
                    min: [1.0; 3],
                    max: [2.0; 3],
                },
                draw_count: 4,
                width: 5,
                height: 6,
                atlas_offset: 7,
                atlas_bytes: 8,
            },
            chunk_x: 9,
            chunk_z: 10,
            section_min_y: 11,
            section_index: 12,
            cells_with_geometry: 13,
            empty_cells: 14,
        }
    }

    fn push_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }
    fn push_f32(out: &mut Vec<u8>, value: f32) {
        out.extend_from_slice(&value.to_le_bytes());
    }
    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn mutated(bytes: &[u8], offset: usize, value: u32) -> Vec<u8> {
        let mut out = bytes.to_vec();
        put_u32(&mut out, offset, value);
        out
    }

    // `(raw id, [mode 0/1/2 (first, count)])` as a hand-written bank.
    type Ranges = [(u32, u32); 3];

    fn make_bank(states: &[(u32, Ranges)], templates: &[Tv], width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"CTBANK01");
        for value in [
            777u32,
            1,
            states.len() as u32,
            templates.len() as u32,
            width,
            height,
            32,
            24,
            0,
            0,
            width * height * 4,
            0,
            0,
            0,
        ] {
            push_u32(&mut out, value);
        }
        for (id, ranges) in states {
            push_u32(&mut out, *id);
            for (first, count) in ranges {
                push_u32(&mut out, *first);
                push_u32(&mut out, *count);
            }
            push_u32(&mut out, 0);
        }
        for template in templates {
            for value in template.p {
                push_f32(&mut out, value);
            }
            for value in template.uv {
                push_f32(&mut out, value);
            }
            push_u32(&mut out, template.rgba);
        }
        out.extend(std::iter::repeat_n(0x11u8, (width * height * 4) as usize));
        out
    }

    fn make_capture(
        chunk_x: i32,
        chunk_z: i32,
        min_y: i32,
        sections: u32,
        cells: &[u32],
    ) -> Vec<u8> {
        assert_eq!(cells.len(), sections as usize * SECTION_CELLS);
        let mut out = Vec::new();
        out.extend_from_slice(b"CTCOL001");
        push_u32(&mut out, 777);
        push_u32(&mut out, 1);
        push_u32(&mut out, chunk_x as u32);
        push_u32(&mut out, chunk_z as u32);
        push_u32(&mut out, min_y as u32);
        push_u32(&mut out, sections);
        push_u32(&mut out, sections * 4096);
        push_u32(&mut out, sections * 64);
        out.extend_from_slice(&[0u8; 8]);
        for cell in cells {
            push_u32(&mut out, *cell);
        }
        out.extend(std::iter::repeat_n(0x22u8, sections as usize * 64 * 4));
        out
    }

    fn templates() -> Vec<Tv> {
        let mut out = Vec::new();
        out.extend_from_slice(&SOLID);
        out.extend_from_slice(&CUTOUT);
        out.extend_from_slice(&TRANSLUCENT);
        out
    }

    fn translated(quad: &[Tv], offset: [f32; 3]) -> Vec<TextureVertex> {
        quad.iter()
            .map(|template| TextureVertex {
                position: [
                    template.p[0] + offset[0],
                    template.p[1] + offset[1],
                    template.p[2] + offset[2],
                ],
                uv: template.uv,
                rgba: template.rgba,
            })
            .collect()
    }

    /// Mode 0/1/2 records over a shared 18-vertex template pool; ID 0 is mapped
    /// to an empty model, the highest raw ID to the same aliased ranges as ID 5.
    fn fixture() -> (Vec<u8>, Vec<u8>) {
        let mut cells = std::vec![0u32; SECTION_CELLS];
        cells[565] = 5; // y=2, z=3, x=5
        cells[4095] = u32::MAX; // y=15, z=15, x=15
        let capture = make_capture(-3, 7, -64, 1, &cells);
        let bank = make_bank(
            &[
                (0, [(0, 0), (0, 0), (0, 0)]),
                (5, [(0, 6), (6, 6), (12, 6)]),
                (u32::MAX, [(0, 6), (6, 6), (12, 6)]),
            ],
            &templates(),
            2,
            2,
        );
        (capture, bank)
    }

    fn query(capture: &[u8], bank: &[u8], section: u32, info: &mut CmInfo) -> i32 {
        unsafe {
            craftrustnx_column_mesh(
                capture.as_ptr(),
                capture.len(),
                bank.as_ptr(),
                bank.len(),
                section,
                core::ptr::null_mut(),
                0,
                core::ptr::null_mut(),
                0,
                info,
            )
        }
    }

    fn build(
        capture: &[u8],
        bank: &[u8],
        section: u32,
        vertices: &mut [TextureVertex],
        draws: &mut [Draw],
        info: &mut CmInfo,
    ) -> i32 {
        unsafe {
            craftrustnx_column_mesh(
                capture.as_ptr(),
                capture.len(),
                bank.as_ptr(),
                bank.len(),
                section,
                vertices.as_mut_ptr(),
                vertices.len(),
                draws.as_mut_ptr(),
                draws.len(),
                info,
            )
        }
    }

    /// Every failure must leave `info` zeroed and both outputs untouched.
    fn expect_failure(capture: &[u8], bank: &[u8], section: u32, status: i32) {
        let mut vertices = [VERTEX_SENTINEL; 64];
        let mut draws = [DRAW_SENTINEL; 8];
        let mut info = sentinel_info();
        assert_eq!(
            build(capture, bank, section, &mut vertices, &mut draws, &mut info),
            status
        );
        assert_eq!(info, CmInfo::default());
        assert!(vertices.iter().all(|vertex| *vertex == VERTEX_SENTINEL));
        assert!(draws.iter().all(|draw| *draw == DRAW_SENTINEL));
    }

    /// `struct cm_info` field offsets and the all-zero failure value the C
    /// fixture compares byte for byte (72 bytes, no padding).
    #[test]
    fn info_layout_matches_the_c_abi_and_zeroes() {
        assert_eq!(core::mem::size_of::<CmInfo>(), 72);
        assert_eq!(core::mem::size_of::<SceneInfo>(), 48);
        assert_eq!(core::mem::offset_of!(CmInfo, scene), 0);
        assert_eq!(core::mem::offset_of!(CmInfo, chunk_x), 48);
        assert_eq!(core::mem::offset_of!(CmInfo, chunk_z), 52);
        assert_eq!(core::mem::offset_of!(CmInfo, section_min_y), 56);
        assert_eq!(core::mem::offset_of!(CmInfo, section_index), 60);
        assert_eq!(core::mem::offset_of!(CmInfo, cells_with_geometry), 64);
        assert_eq!(core::mem::offset_of!(CmInfo, empty_cells), 68);
        let info = CmInfo::default();
        let bytes =
            unsafe { core::slice::from_raw_parts((&info as *const CmInfo).cast::<u8>(), 72) };
        assert!(bytes.iter().all(|byte| *byte == 0));
    }

    /// Query reports exact mesh metadata; the build copies whole quads in mode
    /// order, Y/Z/X cell order, template order, with unchanged UVs and RGBA.
    #[test]
    fn query_and_build_follow_the_header_contract() {
        let (capture, bank) = fixture();
        let mut info = sentinel_info();
        assert_eq!(query(&capture, &bank, 0, &mut info), 0);
        let expected = CmInfo {
            scene: SceneInfo {
                mesh: MeshInfo {
                    vertex_count: 36,
                    min: [4.0, 2.0, 3.0],
                    max: [16.0, 17.0, 16.0],
                },
                draw_count: 3,
                width: 2,
                height: 2,
                atlas_offset: 592,
                atlas_bytes: 16,
            },
            chunk_x: -3,
            chunk_z: 7,
            section_min_y: -64,
            section_index: 0,
            cells_with_geometry: 2,
            empty_cells: SECTION_CELLS as u32 - 2,
        };
        assert_eq!(info, expected);
        assert_eq!(
            info.scene.atlas_offset as usize + info.scene.atlas_bytes as usize,
            bank.len()
        );

        let mut vertices = [VERTEX_SENTINEL; 40];
        let mut draws = [DRAW_SENTINEL; 5];
        let mut built = sentinel_info();
        assert_eq!(
            build(&capture, &bank, 0, &mut vertices, &mut draws, &mut built),
            0
        );
        assert_eq!(built, expected);

        let mut wanted = translated(&SOLID, [5.0, 2.0, 3.0]);
        wanted.extend(translated(&SOLID, [15.0, 15.0, 15.0]));
        wanted.extend(translated(&CUTOUT, [5.0, 2.0, 3.0]));
        wanted.extend(translated(&CUTOUT, [15.0, 15.0, 15.0]));
        wanted.extend(translated(&TRANSLUCENT, [5.0, 2.0, 3.0]));
        wanted.extend(translated(&TRANSLUCENT, [15.0, 15.0, 15.0]));
        assert_eq!(&vertices[..36], wanted.as_slice());
        assert!(vertices[36..]
            .iter()
            .all(|vertex| *vertex == VERTEX_SENTINEL));

        assert_eq!(
            &draws[..3],
            &[
                Draw {
                    first: 0,
                    count: 12,
                    mode: 0,
                    reserved: 0,
                },
                Draw {
                    first: 12,
                    count: 12,
                    mode: 1,
                    reserved: 0,
                },
                Draw {
                    first: 24,
                    count: 12,
                    mode: 2,
                    reserved: 0,
                },
            ]
        );
        assert!(draws[3..].iter().all(|draw| *draw == DRAW_SENTINEL));
    }

    /// The selected section offset and `section_min_y` come from the capture
    /// header; other sections' cells never leak into the mesh.
    #[test]
    fn selected_section_and_min_y_come_from_the_capture_header() {
        let mut cells = std::vec![0u32; SECTION_CELLS * 2];
        cells[0] = 5;
        cells[SECTION_CELLS + 100] = u32::MAX;
        let capture = make_capture(1, -2, -32, 2, &cells);
        let bank = fixture().1;

        let mut first = sentinel_info();
        assert_eq!(query(&capture, &bank, 0, &mut first), 0);
        assert_eq!(first.section_index, 0);
        assert_eq!(first.section_min_y, -32);
        assert_eq!(first.scene.mesh.vertex_count, 18);
        assert_eq!(first.cells_with_geometry, 1);
        assert_eq!(first.cells_with_geometry + first.empty_cells, 4096);

        let mut second = sentinel_info();
        assert_eq!(query(&capture, &bank, 1, &mut second), 0);
        assert_eq!(second.section_index, 1);
        assert_eq!(second.section_min_y, -16);
        assert_eq!(second.scene.mesh.vertex_count, 18);
        assert_eq!(second.cells_with_geometry, 1);

        let mut vertices = [VERTEX_SENTINEL; 18];
        let mut draws = [DRAW_SENTINEL; 3];
        let mut built = sentinel_info();
        assert_eq!(
            build(&capture, &bank, 1, &mut vertices, &mut draws, &mut built),
            0
        );
        assert_eq!(built, second);
        // Cell 100 is x=4, y=0, z=6 in the selected section.
        assert_eq!(vertices[0].position, [4.0, 0.0, 6.0]);
        assert_eq!(vertices[13].position, [3.0, 0.0, 6.0]);
        expect_failure(&capture, &bank, 2, -3);
    }

    /// Empty mode ranges contribute no draw and no vertices; a cell is counted
    /// as empty exactly when all three ranges are empty.
    #[test]
    fn empty_mode_groups_emit_no_draws() {
        let mut cells = std::vec![0u32; SECTION_CELLS];
        cells[7] = 9;
        let capture = make_capture(0, 0, 0, 1, &cells);
        let bank = make_bank(
            &[(0, [(0, 0), (0, 0), (0, 0)]), (9, [(0, 0), (6, 6), (0, 0)])],
            &templates(),
            2,
            2,
        );

        let mut info = sentinel_info();
        assert_eq!(query(&capture, &bank, 0, &mut info), 0);
        assert_eq!(info.scene.mesh.vertex_count, 6);
        assert_eq!(info.scene.draw_count, 1);
        assert_eq!(info.cells_with_geometry, 1);
        assert_eq!(info.scene.mesh.min, [7.0, 0.0, 0.0]);
        assert_eq!(info.scene.mesh.max, [8.0, 1.0, 0.0]);

        let mut vertices = [VERTEX_SENTINEL; 6];
        let mut draws = [DRAW_SENTINEL; 1];
        let mut built = sentinel_info();
        assert_eq!(
            build(&capture, &bank, 0, &mut vertices, &mut draws, &mut built),
            0
        );
        assert_eq!(built, info);
        assert_eq!(
            draws[0],
            Draw {
                first: 0,
                count: 6,
                mode: 1,
                reserved: 0,
            }
        );
        assert_eq!(
            vertices.as_slice(),
            translated(&CUTOUT, [7.0, 0.0, 0.0]).as_slice()
        );
    }

    /// Unsupported IDs and sections that emit nothing are distinct from
    /// malformed bytes and return -5.
    #[test]
    fn missing_ids_and_empty_sections_report_minus_five() {
        let (capture, bank) = fixture();
        let mut missing = capture.clone();
        put_u32(&mut missing, 48 + 565 * 4, 12_345);
        expect_failure(&missing, &bank, 0, -5);

        let mut empty = capture.clone();
        let blocks = 48..48 + SECTION_CELLS * 4;
        empty[blocks].fill(0);
        expect_failure(&empty, &bank, 0, -5);
    }

    /// Bank and capture mutations are rejected as malformed; nothing is written.
    #[test]
    fn malformed_bank_and_capture_are_rejected_before_any_write() {
        let (capture, bank) = fixture();
        let mut info = sentinel_info();
        assert_eq!(query(&capture, &bank, 0, &mut info), 0);

        for (offset, value) in [
            (8, 776),
            (12, 2),
            (16, 0),
            (16, 65_537),
            (20, 20),
            (20, 262_146),
            (24, 3),
            (24, 2048),
            (28, 0),
            (32, 31),
            (36, 23),
            (40, 1),
            (44, 1),
            (48, 15),
            (52, 1),
            (56, 1),
            (60, 1),
        ] {
            expect_failure(&capture, &mutated(&bank, offset, value), 0, -3);
        }
        let mut magic = bank.clone();
        magic[0] = b'X';
        expect_failure(&capture, &magic, 0, -3);
        expect_failure(&capture, &bank[..bank.len() - 1], 0, -3);
        let mut longer = bank.clone();
        longer.push(0);
        expect_failure(&capture, &longer, 0, -3);

        // Record 64 = ID 0, 96 = ID 5, 128 = u32::MAX.
        for (offset, value) in [(64, 7), (64, 5), (64 + 28, 1), (96 + 4, 3), (96 + 8, 24)] {
            expect_failure(&capture, &mutated(&bank, offset, value), 0, -3);
        }
        let mut overrun = bank.clone();
        put_u32(&mut overrun, 96 + 12, 18);
        put_u32(&mut overrun, 96 + 16, 6);
        expect_failure(&capture, &overrun, 0, -3);

        // Templates start after three 32-byte records.
        let templates_at = 64 + 3 * 32;
        for (offset, bits) in [
            (templates_at, f32::NAN.to_bits()),
            (templates_at, 1025.0f32.to_bits()),
            (templates_at, (-1025.0f32).to_bits()),
            (templates_at + 12, 1.5f32.to_bits()),
            (templates_at + 16, f32::NAN.to_bits()),
        ] {
            expect_failure(&capture, &mutated(&bank, offset, bits), 0, -3);
        }

        for (offset, value) in [
            (8, 778),
            (12, 2),
            (24, 1),
            (28, 0),
            (28, 2),
            (32, 4095),
            (36, 63),
        ] {
            expect_failure(&mutated(&capture, offset, value), &bank, 0, -3);
        }
        let mut magic = capture.clone();
        magic[0] = b'X';
        expect_failure(&magic, &bank, 0, -3);
        let mut reserved = capture.clone();
        reserved[40] = 1;
        expect_failure(&reserved, &bank, 0, -3);
        expect_failure(&capture[..capture.len() - 1], &bank, 0, -3);
        let mut longer = capture.clone();
        longer.push(0);
        expect_failure(&longer, &bank, 0, -3);
    }

    /// Oversized totals, translated positions beyond the coordinate limit and
    /// an all-collapsed bounding box are refused as malformed.
    #[test]
    fn oversized_and_degenerate_meshes_are_refused() {
        // 4096 cells with eleven quads each would pass 262140 vertices.
        let mut heavy = Vec::new();
        for _ in 0..11 {
            heavy.extend_from_slice(&SOLID);
        }
        let cells = std::vec![7u32; SECTION_CELLS];
        let capture = make_capture(0, 0, 0, 1, &cells);
        let bank = make_bank(&[(7, [(0, 66), (0, 0), (0, 0)])], &heavy, 2, 2);
        expect_failure(&capture, &bank, 0, -3);

        // A template at the limit plus a non-zero cell translation exceeds 1024.
        let mut far = SOLID;
        for template in &mut far {
            template.p[0] = 1024.0;
        }
        let cells = std::vec![0u32; SECTION_CELLS];
        let capture = make_capture(0, 0, 0, 1, &cells);
        let bank = make_bank(&[(0, [(0, 6), (0, 0), (0, 0)])], &far, 2, 2);
        expect_failure(&capture, &bank, 0, -3);

        // One cell whose emitted corners all share one position: no axis spans.
        let flat = [Tv {
            p: [0.5; 3],
            uv: [0.0; 2],
            rgba: 0xffff_ffff,
        }; 6];
        let mut cells = std::vec![0u32; SECTION_CELLS];
        cells[0] = 1;
        let capture = make_capture(0, 0, 0, 1, &cells);
        let bank = make_bank(
            &[(0, [(0, 0), (0, 0), (0, 0)]), (1, [(0, 6), (0, 0), (0, 0)])],
            &flat,
            2,
            2,
        );
        expect_failure(&capture, &bank, 0, -3);
    }

    /// Insufficient capacities and bad pointer shapes fail without touching the
    /// outputs; the capacity fields themselves are bounded before any slice.
    #[test]
    fn capacity_and_argument_errors_leave_outputs_untouched() {
        let (capture, bank) = fixture();
        let mut vertices = [VERTEX_SENTINEL; 40];
        let mut draws = [DRAW_SENTINEL; 5];
        let mut info = sentinel_info();
        assert_eq!(
            build(
                &capture,
                &bank,
                0,
                &mut vertices[..35],
                &mut draws,
                &mut info
            ),
            -2
        );
        assert_eq!(info, CmInfo::default());
        assert_eq!(
            build(
                &capture,
                &bank,
                0,
                &mut vertices,
                &mut draws[..2],
                &mut info
            ),
            -2
        );
        assert_eq!(info, CmInfo::default());
        assert_eq!(
            build(
                &capture,
                &bank,
                0,
                &mut vertices[..0],
                &mut draws[..0],
                &mut info
            ),
            -2
        );
        assert_eq!(info, CmInfo::default());
        assert!(vertices.iter().all(|vertex| *vertex == VERTEX_SENTINEL));
        assert!(draws.iter().all(|draw| *draw == DRAW_SENTINEL));

        let call = |vertices: *mut TextureVertex,
                    vertex_capacity: usize,
                    draws: *mut Draw,
                    draw_capacity: usize,
                    info: *mut CmInfo| unsafe {
            craftrustnx_column_mesh(
                capture.as_ptr(),
                capture.len(),
                bank.as_ptr(),
                bank.len(),
                0,
                vertices,
                vertex_capacity,
                draws,
                draw_capacity,
                info,
            )
        };
        assert_eq!(
            call(core::ptr::null_mut(), 0, draws.as_mut_ptr(), 5, &mut info),
            -1
        );
        assert_eq!(
            call(
                vertices.as_mut_ptr(),
                40,
                core::ptr::null_mut(),
                0,
                &mut info
            ),
            -1
        );
        assert_eq!(
            call(
                core::ptr::null_mut(),
                1,
                core::ptr::null_mut(),
                0,
                &mut info
            ),
            -1
        );
        assert_eq!(
            call(
                vertices.as_mut_ptr(),
                MAX_VERTICES + 1,
                draws.as_mut_ptr(),
                5,
                &mut info
            ),
            -1
        );
        assert_eq!(
            call(
                vertices.as_mut_ptr(),
                40,
                draws.as_mut_ptr(),
                MAX_DRAWS + 1,
                &mut info
            ),
            -1
        );
        assert_eq!(
            call(
                vertices.as_mut_ptr(),
                40,
                draws.as_mut_ptr(),
                5,
                core::ptr::null_mut()
            ),
            -1
        );
        assert_eq!(info, CmInfo::default());
        assert!(vertices.iter().all(|vertex| *vertex == VERTEX_SENTINEL));
        assert!(draws.iter().all(|draw| *draw == DRAW_SENTINEL));

        let mut null_inputs = sentinel_info();
        assert_eq!(
            unsafe {
                craftrustnx_column_mesh(
                    core::ptr::null(),
                    48,
                    bank.as_ptr(),
                    bank.len(),
                    0,
                    vertices.as_mut_ptr(),
                    40,
                    draws.as_mut_ptr(),
                    5,
                    &mut null_inputs,
                )
            },
            -1
        );
        assert_eq!(null_inputs, CmInfo::default());
        assert_eq!(
            unsafe {
                craftrustnx_column_mesh(
                    capture.as_ptr(),
                    capture.len(),
                    core::ptr::null(),
                    64,
                    0,
                    vertices.as_mut_ptr(),
                    40,
                    draws.as_mut_ptr(),
                    5,
                    &mut null_inputs,
                )
            },
            -1
        );
        assert_eq!(null_inputs, CmInfo::default());

        // A forged length must be refused before the pointers are widened.
        assert_eq!(
            unsafe {
                craftrustnx_column_mesh(
                    capture.as_ptr(),
                    capture.len(),
                    bank.as_ptr(),
                    MAX_BANK + 1,
                    0,
                    vertices.as_mut_ptr(),
                    40,
                    draws.as_mut_ptr(),
                    5,
                    &mut null_inputs,
                )
            },
            -3
        );
        assert_eq!(null_inputs, CmInfo::default());
        assert_eq!(
            unsafe {
                craftrustnx_column_mesh(
                    capture.as_ptr(),
                    MAX_CAPTURE + 1,
                    bank.as_ptr(),
                    bank.len(),
                    0,
                    vertices.as_mut_ptr(),
                    40,
                    draws.as_mut_ptr(),
                    5,
                    &mut null_inputs,
                )
            },
            -3
        );
        assert_eq!(null_inputs, CmInfo::default());
        expect_failure(&capture[..47], &bank, 0, -3);
        expect_failure(&capture, &bank[..63], 0, -3);
        assert!(vertices.iter().all(|vertex| *vertex == VERTEX_SENTINEL));
        assert!(draws.iter().all(|draw| *draw == DRAW_SENTINEL));
    }
}
