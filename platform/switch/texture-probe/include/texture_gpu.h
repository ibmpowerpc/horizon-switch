#ifndef CRAFTRUSTNX_TEXTURE_GPU_H
#define CRAFTRUSTNX_TEXTURE_GPU_H
#include <stdbool.h>
#include <stdint.h>
#define TX_MAX_VERTICES 262140u
#define TX_MAX_DRAWS 256u
#define TX_MAX_DIMENSION 1024u
enum { TX_OPAQUE = 0, TX_CUTOUT = 1, TX_TRANSLUCENT = 2 };
struct tx_vertex {
  float position[3], uv[2];
  uint32_t rgba;
};
struct tx_draw {
  uint32_t first, count, mode, reserved;
};
/* Single graphics thread, romfs already mounted. Shader stem texture.
 * Double-buffered 1280x720 sRGB surface and depth. Negative errors on failure.
 */
int32_t tx_graphics_initialize(void);
/* Whole static atlas, power-of-two dimensions 1..1024, tightly packed RGBA8.
 * Copies pixels synchronously; retains no caller pointer. Repeated successful
 * uploads replace the texture. Failure does not replace a valid texture. */
int32_t tx_graphics_atlas(const uint8_t *rgba, uint32_t width, uint32_t height);
/* Triangle list; mode2 ranges also consist of consecutive six-vertex quads.
 * Ranges cover vertices exactly, have reserved=0 and modes0..2. Caller keeps
 * opaque/cutout before translucent and sorts translucent quads far-to-near.
 * Copies synchronously, no retained caller pointers. count0 clears the mesh. */
int32_t tx_graphics_upload(const struct tx_vertex *vertices, uint32_t count,
                           const struct tx_draw *draws, uint32_t draw_count);
/* Column-major MVP and finite RGBA clear. Opaque/cutout depth-write; straight
 * alpha translucent blending with depth-read only. Nearest/clamp sampler.
 * Cutout discards alpha<0.5 as in Horizon's terrain.wgsl fs_solid. Texture is
 * sRGB; use an sRGB render-target view for linear shader output. Hardware
 * colour correctness remains an explicit test. Wait before reusing shared
 * storage. */
int32_t tx_graphics_present(const float mvp[16], const float clear[4]);
void tx_graphics_shutdown(void);
uint64_t tx_graphics_allocated_bytes(void);
#endif
