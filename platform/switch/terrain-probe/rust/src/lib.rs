//! Allocation-free mesh validation and camera for the standalone 3D probe.
//! No gameplay, network, SDK or file I/O. ABI: ../include/terrain_app.h.
#![cfg_attr(not(test), no_std)]

use core::{mem, slice};

#[cfg(feature = "textured")]
pub mod textured;

pub const MAX_VERTICES: usize = 262_140;
pub const MAX_FILE_BYTES: usize = 48 + 16 * MAX_VERTICES;
pub const ORBIT: u32 = 1;
pub const BACKGROUND: u32 = 2;
pub const RESET: u32 = 4;
pub const EXIT: u32 = 8;
pub const ZOOM_IN: u32 = 16;
pub const ZOOM_OUT: u32 = 32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub rgba: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeshInfo {
    pub vertex_count: u32,
    pub min: [f32; 3],
    pub max: [f32; 3],
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AppState {
    pub previous_ns: u64,
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub auto_orbit: u32,
    pub frames: u32,
    pub palette: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AppFrame {
    pub clear: [f32; 4],
    pub mvp: [f32; 16],
    pub should_exit: u32,
    pub reserved: [u32; 3],
}
const _: () = {
    assert!(mem::size_of::<Vertex>() == 16);
    assert!(mem::offset_of!(Vertex, rgba) == 12);
    assert!(mem::size_of::<MeshInfo>() == 28);
    assert!(mem::size_of::<AppState>() == 32);
    assert!(mem::align_of::<AppState>() == 8);
    assert!(mem::size_of::<AppFrame>() == 96);
};

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn float(bytes: &[u8], offset: usize) -> f32 {
    f32::from_bits(word(bytes, offset))
}
fn valid_bounds(info: &MeshInfo) -> bool {
    info.vertex_count != 0
        && info.vertex_count as usize <= MAX_VERTICES
        && info.vertex_count.is_multiple_of(3)
        && (0..3).all(|i| {
            info.min[i].is_finite()
                && info.max[i].is_finite()
                && info.min[i].abs() <= 1024.0
                && info.max[i].abs() <= 1024.0
                && info.min[i] <= info.max[i]
        })
        && (0..3).any(|i| info.min[i] < info.max[i])
}

/// Validate a complete little-endian mesh before writing any vertices.
///
/// # Safety
/// Non-null info/vertices are aligned, writable and disjoint; data points to len
/// readable bytes. Output capacity is in vertices. Buffers do not overlap.
#[no_mangle]
pub unsafe extern "C" fn craftrustnx_mesh_decode(
    data: *const u8,
    len: usize,
    vertices: *mut Vertex,
    capacity: usize,
    info: *mut MeshInfo,
) -> i32 {
    if info.is_null() {
        return -1;
    }
    unsafe {
        info.write(MeshInfo::default());
    }
    if data.is_null() || vertices.is_null() {
        return -1;
    }
    if !(48..=MAX_FILE_BYTES).contains(&len) {
        return -3;
    }
    let bytes = unsafe { slice::from_raw_parts(data, len) };
    if &bytes[..8] != b"CTMESH01"
        || word(bytes, 8) != 777
        || word(bytes, 12) != 1
        || word(bytes, 20) != 0
    {
        return -3;
    }
    let count = word(bytes, 16) as usize;
    if count == 0 || count > MAX_VERTICES || !count.is_multiple_of(3) || len != 48 + count * 16 {
        return -3;
    }
    if capacity < count {
        return -2;
    }
    let decoded = MeshInfo {
        vertex_count: count as u32,
        min: core::array::from_fn(|i| float(bytes, 24 + i * 4)),
        max: core::array::from_fn(|i| float(bytes, 36 + i * 4)),
    };
    if !valid_bounds(&decoded) {
        return -3;
    }
    for i in 0..count {
        for axis in 0..3 {
            let v = float(bytes, 48 + i * 16 + axis * 4);
            if !v.is_finite() || v < decoded.min[axis] || v > decoded.max[axis] {
                return -3;
            }
        }
    }
    for i in 0..count {
        let vertex = Vertex {
            position: core::array::from_fn(|axis| float(bytes, 48 + i * 16 + axis * 4)),
            rgba: word(bytes, 48 + i * 16 + 12),
        };
        unsafe {
            vertices.add(i).write(vertex);
        }
    }
    unsafe {
        info.write(decoded);
    }
    0
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = libm::sqrtf(dot(v, v));
    v.map(|x| x / length)
}
fn perspective(near: f32, far: f32) -> [f32; 16] {
    let mut m = [0.0; 16];
    let cot_half_fov = 1.732_050_8; // 60 degree vertical field of view.
    m[0] = cot_half_fov / (1280.0 / 720.0);
    m[5] = cot_half_fov;
    m[10] = far / (near - far);
    m[11] = -1.0;
    m[14] = (far * near) / (near - far);
    m
}
fn multiply(a: [f32; 16], b: [f32; 16]) -> [f32; 16] {
    core::array::from_fn(|i| {
        let row = i % 4;
        let col = i / 4;
        (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum()
    })
}
fn matrix(info: &MeshInfo, state: &AppState) -> [f32; 16] {
    let center = core::array::from_fn::<_, 3, _>(|i| (info.min[i] + info.max[i]) * 0.5);
    let extent = core::array::from_fn::<_, 3, _>(|i| (info.max[i] - info.min[i]) * 0.5);
    let radius = libm::sqrtf(dot(extent, extent)).max(0.1);
    let distance = radius * 3.0 * state.zoom;
    let eye = [
        center[0] + distance * libm::cosf(state.pitch) * libm::cosf(state.yaw),
        center[1] + distance * libm::sinf(state.pitch),
        center[2] + distance * libm::cosf(state.pitch) * libm::sinf(state.yaw),
    ];
    let forward = normalize(core::array::from_fn(|i| center[i] - eye[i]));
    let right = normalize(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    let view = [
        right[0],
        up[0],
        -forward[0],
        0.0,
        right[1],
        up[1],
        -forward[1],
        0.0,
        right[2],
        up[2],
        -forward[2],
        0.0,
        -dot(right, eye),
        -dot(up, eye),
        dot(forward, eye),
        1.0,
    ];
    multiply(
        perspective((radius * 0.01).max(0.001), distance + radius * 20.0),
        view,
    )
}

/// Update bounded orbit camera controls; no simulation/network effects.
///
/// # Safety
/// Non-null pointers are aligned and valid for their types, disjoint, and
/// writable for state/frame. info remains readable during this call.
#[no_mangle]
pub unsafe extern "C" fn craftrustnx_terrain_update(
    state: *mut AppState,
    info: *const MeshInfo,
    elapsed_ns: u64,
    buttons: u32,
    stick_x: f32,
    stick_y: f32,
    frame: *mut AppFrame,
) -> i32 {
    if frame.is_null() {
        return -1;
    }
    unsafe {
        frame.write(AppFrame::default());
    }
    if state.is_null() || info.is_null() {
        return -1;
    }
    let original = unsafe { *state };
    let info = unsafe { *info };
    if !valid_bounds(&info) || !stick_x.is_finite() || !stick_y.is_finite() {
        return -3;
    }
    if original.frames != 0
        && (!original.yaw.is_finite()
            || !original.pitch.is_finite()
            || !original.zoom.is_finite()
            || original.zoom < 0.35
            || original.zoom > 3.0
            || original.pitch.abs() > 1.3
            || original.auto_orbit > 1
            || original.palette > 2)
    {
        return -3;
    }
    let mut next = original;
    let reset = original.frames == 0 || buttons & RESET != 0;
    let delta = if reset {
        0
    } else {
        elapsed_ns
            .saturating_sub(original.previous_ns)
            .min(250_000_000)
    };
    next.previous_ns = elapsed_ns.max(original.previous_ns);
    let seconds = delta as f32 * 1e-9;
    if reset {
        next.yaw = 0.7;
        next.pitch = 0.5;
        next.zoom = 1.0;
        next.auto_orbit = 1;
        next.palette = 0;
    } else {
        if buttons & ORBIT != 0 {
            next.auto_orbit ^= 1;
        }
        if buttons & BACKGROUND != 0 {
            next.palette = (next.palette + 1) % 3;
        }
        let stick = |v: f32| {
            if v.abs() < 0.15 {
                0.0
            } else {
                v.clamp(-1.0, 1.0)
            }
        };
        next.yaw += (0.3 * next.auto_orbit as f32 + stick(stick_x) * 2.0) * seconds;
        next.pitch = (next.pitch + stick(stick_y) * 1.5 * seconds).clamp(-1.3, 1.3);
        next.zoom = (next.zoom
            + ((buttons & ZOOM_OUT != 0) as i32 - (buttons & ZOOM_IN != 0) as i32) as f32
                * seconds)
            .clamp(0.35, 3.0);
    }
    next.yaw = libm::fmodf(next.yaw, core::f32::consts::TAU);
    if next.yaw < 0.0 {
        next.yaw += core::f32::consts::TAU;
    }
    next.frames = next.frames.saturating_add(1);
    let backgrounds = [
        [0.03, 0.04, 0.07, 1.0],
        [0.22, 0.28, 0.36, 1.0],
        [0.65, 0.72, 0.8, 1.0],
    ];
    let output = AppFrame {
        clear: backgrounds[next.palette as usize],
        mvp: matrix(&info, &next),
        should_exit: u32::from(buttons & EXIT != 0),
        reserved: [0; 3],
    };
    if !output.mvp.iter().all(|x| x.is_finite()) {
        return -3;
    }
    unsafe {
        state.write(next);
        frame.write(output);
    }
    0
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    extern "C" {
        fn abort() -> !;
    }
    unsafe { abort() }
}

// Host C-link checks use precompiled core's personality references; the
// abort-built Switch core does not need this symbol.
#[cfg(all(not(test), not(target_os = "horizon")))]
#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> std::vec::Vec<u8> {
        let mut out = std::vec::Vec::from(*b"CTMESH01");
        for n in [777u32, 1, 3, 0] {
            out.extend(n.to_le_bytes());
        }
        for f in [-1.0f32, 0.0, 0.0, 1.0, 1.0, 0.0] {
            out.extend(f.to_le_bytes());
        }
        for (position, color) in [
            ([-1.0f32, 0.0, 0.0], 0xff0000ffu32),
            ([1.0, 0.0, 0.0], 0xff00ff00),
            ([0.0, 1.0, 0.0], 0xffff0000),
        ] {
            for f in position {
                out.extend(f.to_le_bytes());
            }
            out.extend(color.to_le_bytes());
        }
        out
    }
    fn info() -> MeshInfo {
        MeshInfo {
            vertex_count: 3,
            min: [-1.0, 0.0, 0.0],
            max: [1.0, 1.0, 0.0],
        }
    }
    fn update(s: &mut AppState, t: u64, b: u32) -> AppFrame {
        let mut f = AppFrame::default();
        assert_eq!(
            unsafe { craftrustnx_terrain_update(s, &info(), t, b, 0.0, 0.0, &mut f) },
            0
        );
        f
    }
    #[test]
    fn mesh_decodes_unaligned_little_endian_input() {
        let mut bytes = std::vec![99];
        bytes.extend(fixture());
        let mut vertices = [Vertex::default(); 3];
        let mut meta = MeshInfo::default();
        assert_eq!(
            unsafe {
                craftrustnx_mesh_decode(
                    bytes[1..].as_ptr(),
                    bytes.len() - 1,
                    vertices.as_mut_ptr(),
                    3,
                    &mut meta,
                )
            },
            0
        );
        assert_eq!(meta, info());
        assert_eq!(vertices[0].position, [-1.0, 0.0, 0.0]);
        assert_eq!(vertices[2].rgba, 0xffff0000);
    }
    #[test]
    fn malformed_mesh_never_partially_writes_vertices() {
        let good = fixture();
        let sentinel = Vertex {
            position: [42.0; 3],
            rgba: 123,
        };
        let mut cases = std::vec::Vec::new();
        for n in 0..good.len() {
            cases.push(good[..n].to_vec());
        }
        for (offset, word) in [
            (8, 776u32),
            (12, 2),
            (16, 4),
            (20, 1),
            (48, f32::NAN.to_bits()),
            (80, 2.0f32.to_bits()),
        ] {
            let mut b = good.clone();
            b[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
            cases.push(b);
        }
        for bytes in cases {
            let mut vertices = [sentinel; 3];
            let mut meta = info();
            assert_ne!(
                unsafe {
                    craftrustnx_mesh_decode(
                        bytes.as_ptr(),
                        bytes.len(),
                        vertices.as_mut_ptr(),
                        3,
                        &mut meta,
                    )
                },
                0
            );
            assert_eq!(vertices, [sentinel; 3]);
            assert_eq!(meta, MeshInfo::default());
        }
    }
    #[test]
    fn insufficient_capacity_and_nulls_preserve_output() {
        let b = fixture();
        let mut v = [Vertex::default(); 3];
        let mut m = info();
        assert_eq!(
            unsafe { craftrustnx_mesh_decode(b.as_ptr(), b.len(), v.as_mut_ptr(), 2, &mut m) },
            -2
        );
        assert_eq!(v, [Vertex::default(); 3]);
        assert_eq!(m, MeshInfo::default());
        assert_eq!(
            unsafe {
                craftrustnx_mesh_decode(core::ptr::null(), b.len(), v.as_mut_ptr(), 3, &mut m)
            },
            -1
        );
    }
    #[test]
    fn projection_maps_right_handed_near_and_far_to_zero_and_one() {
        let m = perspective(0.1, 100.0);
        for (z, want) in [(-0.1, 0.0), (-100.0, 1.0)] {
            let depth = (m[10] * z + m[14]) / (-z);
            assert!((depth - want).abs() < 1e-6);
        }
    }
    #[test]
    fn orbit_controls_reset_and_timestamps_are_bounded() {
        let mut s = AppState::default();
        let initial = update(&mut s, 100, 0);
        assert_eq!(s.zoom, 1.0);
        assert!(initial.mvp.iter().all(|x| x.is_finite()));
        update(&mut s, 1_000_000_100, 0);
        assert!((s.yaw - 0.775).abs() < 1e-5);
        let yaw = s.yaw;
        update(&mut s, 1_000_000_200, ORBIT);
        assert_eq!(s.auto_orbit, 0);
        update(&mut s, 2_000_000_200, 0);
        assert_eq!(s.yaw, yaw);
        update(&mut s, 100, 0);
        assert_eq!(s.previous_ns, 2_000_000_200);
        let f = update(&mut s, 3_000_000_200, RESET | EXIT | BACKGROUND);
        assert_eq!(s.yaw, 0.7);
        assert_eq!(s.palette, 0);
        assert_eq!(f.should_exit, 1);
    }
    #[test]
    fn invalid_camera_input_is_transactional() {
        let mut s = AppState::default();
        update(&mut s, 0, 0);
        let before = s;
        let mut f = AppFrame {
            clear: [1.0; 4],
            ..AppFrame::default()
        };
        assert_eq!(
            unsafe { craftrustnx_terrain_update(&mut s, &info(), 1, 0, f32::NAN, 0.0, &mut f) },
            -3
        );
        assert_eq!(s, before);
        assert_eq!(f, AppFrame::default());
    }
}
