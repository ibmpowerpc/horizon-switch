#ifndef CRAFTRUSTNX_TEXTURE_APP_H
#define CRAFTRUSTNX_TEXTURE_APP_H
#include "terrain_app.h"
#include "texture_gpu.h"
struct tx_scene_info {
  struct tp_mesh_info mesh;
  uint32_t draw_count, width, height, atlas_offset, atlas_bytes;
};
/* File CTTEX001: protocol777, format1, vertex_count, draw_count, width,
 * height, flags0, reserved0 (all LE u32), bounds6 LE f32 at40; header64.
 * Then vertex_count*24 bytes, draw_count*16 bytes, width*height*4 RGBA8.
 * Exact length, max16MiB. Validates everything before any vertex/draw write;
 * info zero on failure. Non-null aligned/disjoint outputs, readable input.
 * Return0/-1 bad argument/-2 capacity/-3 invalid file. */
int32_t craftrustnx_texture_decode(const uint8_t *data, size_t len,
                                   struct tx_vertex *vertices,
                                   size_t vertex_capacity,
                                   struct tx_draw *draws, size_t draw_capacity,
                                   struct tx_scene_info *info);
/* Sort six-vertex translucent quads by descending clip w using current MVP.
 * Allocation-free; validates all arguments before mutations. Opaque/cutout
 * vertices unchanged. Return0/-1 argument/-3 invalid geometry/ranges/MVP. */
int32_t craftrustnx_texture_sort(struct tx_vertex *vertices, uint32_t count,
                                 const struct tx_draw *draws,
                                 uint32_t draw_count, const float mvp[16]);
#endif
