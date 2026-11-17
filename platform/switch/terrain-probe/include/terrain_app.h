#ifndef CRAFTRUSTNX_TERRAIN_APP_H
#define CRAFTRUSTNX_TERRAIN_APP_H
#include "terrain_gpu.h"
#include <stddef.h>

enum { TP_ORBIT=1, TP_BACKGROUND=2, TP_RESET=4, TP_EXIT=8,
       TP_ZOOM_IN=16, TP_ZOOM_OUT=32 };
struct tp_mesh_info { uint32_t vertex_count; float min[3], max[3]; };
struct tp_app_state {
    uint64_t previous_ns;
    float yaw, pitch, zoom;
    uint32_t auto_orbit, frames, palette;
};
struct tp_app_frame { float clear[4], mvp[16]; uint32_t should_exit, reserved[3]; };

/* Mesh file: CTMESH01, u32 protocol777, u32 format1, u32 vertex_count,
 * u32 reserved0, six LE f32 bounds(minXYZ,maxXYZ), then count 16-byte vertices
 * (three LE f32 + u32 RGBA). Exact size48+16*count; max TP_MAX_VERTICES.
 * Zero count rejected. No textures or state semantics. On failure info is zero,
 * vertex output untouched. Caller provides valid non-overlapping aligned
 * buffers; the input bytes themselves need not be aligned. */
int32_t craftrustnx_mesh_decode(const uint8_t *data, size_t len,
    struct tp_vertex *vertices, size_t capacity, struct tp_mesh_info *info);
/* State initially zero. info is a successfully decoded mesh's bounds.
 * Edge-trigger ORBIT/BACKGROUND/RESET/EXIT; held ZOOM_IN/OUT. Stick rotates view.
 * On invalid/null inputs frame is zeroed when non-null, state unchanged.
 * Pure Rust: no SDK, allocation, I/O or game/world state. */
int32_t craftrustnx_terrain_update(struct tp_app_state *state,
    const struct tp_mesh_info *info, uint64_t elapsed_ns, uint32_t buttons,
    float stick_x, float stick_y, struct tp_app_frame *frame);
#endif
