#ifndef CRAFTRUSTNX_TERRAIN_GPU_H
#define CRAFTRUSTNX_TERRAIN_GPU_H
#include <stdint.h>

/* Experimental untextured mesh bridge. Rust owns geometry, camera and input.
 * No Minecraft state, model, fluid or light semantics are implemented here. */
#define TP_MAX_VERTICES 262140u
struct tp_vertex {
    float position[3];
    /* Little endian memory bytes R,G,B,A: R | G<<8 | B<<16 | A<<24. */
    uint32_t rgba;
};

/* Single graphics thread. Caller mounts romfs. Negative results are errors.
 * Shader paths: romfs:/shaders/terrain_vsh.dksh and terrain_fsh.dksh.
 * Initialize one double-buffered 1280x720 RGBA8 surface with a depth target. */
int32_t tp_graphics_initialize(void);
/* Non-indexed triangle list. count <= TP_MAX_VERTICES, divisible by 3.
 * count=0 empties the mesh (vertices may be null). Otherwise caller supplies
 * count valid vertices; the bridge copies them before returning and retains
 * no caller pointer. Invalid arguments leave the previous mesh usable. */
int32_t tp_graphics_upload(const struct tp_vertex *vertices, uint32_t count);
/* Column-major MVP, four finite clear-color components. Geometry color is
 * RGBA8 normalized in the shader. Device uses depth zero-to-one and upper-left
 * framebuffer origin (SDK defaults). Do not alter the caller's MVP in shaders.
 * GPU work is complete before shared command/uniform/mesh storage is reused. */
int32_t tp_graphics_present(const float mvp[16], const float clear[4]);
/* Safe after failed init, idempotent; wait GPU before freeing resources. */
void tp_graphics_shutdown(void);
uint64_t tp_graphics_allocated_bytes(void);

#endif
