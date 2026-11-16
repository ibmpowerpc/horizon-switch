#ifndef CRAFTRUSTNX_GPU_PROBE_H
#define CRAFTRUSTNX_GPU_PROBE_H
#include <stdint.h>
/* Standalone GPU smoke test, not a final renderer ABI. No Minecraft assets. */
enum { GP_PAUSE = 1, GP_PALETTE = 2, GP_RESET = 4, GP_EXIT = 8 };
/* 32 bytes, align8. Zero-initialize. elapsed_ns is a monotonic app-relative time. */
struct gp_state {
    uint64_t frames;
    uint64_t animation_ns;
    uint64_t previous_ns;
    uint32_t paused;
    uint32_t palette;
};
/* 128 bytes, align4. First clear, then 96 bytes of std140 vertex/color uniforms.
 * Positions are three clip-space vec4s. Colors are three RGBA vec4s. */
struct gp_frame {
    float clear[4];
    float positions[3][4];
    float colors[3][4];
    uint32_t should_exit;
    uint32_t reserved[3];
};
/* Rust: caller guarantees aligned valid non-overlapping state/frame. Returns
 * 0 on success, -1 for null; frame is zeroed on a non-null output error. No
 * allocations, SDK calls, globals or libm. Unknown button bits ignored. */
int32_t craftrustnx_gpu_update(struct gp_state *state, uint64_t elapsed_ns,
    uint32_t buttons_down, struct gp_frame *frame);
/* Native libnx/deko3d boundary. Single main thread only. Zero means success.
 * Init loads romfs:/shaders/probe_vsh.dksh + probe_fsh.dksh; caller mounts romfs.
 * Shutdown idempotent, safe after failed init. Present consumes one Rust frame;
 * no GPU reads from caller memory after it returns. No console while active. */
int32_t gp_graphics_initialize(void);
int32_t gp_graphics_present(const struct gp_frame *frame);
void gp_graphics_shutdown(void);
uint64_t gp_graphics_allocated_bytes(void);
#endif
