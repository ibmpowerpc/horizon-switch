#ifndef CRAFTRUSTNX_COLUMN_CODEC_H
#define CRAFTRUSTNX_COLUMN_CODEC_H
#include <stdint.h>
#include "world_codec.h"
/* Normalized single column, raw IDs only. No world cache, rendering or readiness.
 * Arrays use bottom-to-top section order, then Y/Z/X blocks and biome cells. */
enum { COL_OK=0, COL_BAD_ARGUMENT=-1, COL_OUTPUT_SMALL=-2, COL_BAD_FRAME=-3,
    COL_UNSUPPORTED=-5, COL_LIMIT=-6, COL_MAX_BLOCKS=256*4096,
    COL_MAX_BIOMES=256*64 };
struct col_info {
    int32_t chunk_x;
    int32_t chunk_z;
    int32_t min_y;
    uint32_t sections;
    uint32_t blocks_written;
    uint32_t biomes_written;
};
/* Complete uncompressed play46 body, ID included. Same structural validation as
 * world decoder, including NBT/light. Caller provides initialized registry/play
 * state, valid aligned non-overlapping buffers and truthful capacities in u32
 * elements (not bytes). Body 1..WC_MAX_BODY; capacities <= COL_MAX_*.
 * Info is zeroed on failure; output arrays and input states unchanged on every
 * failure. Validate fully before writes. No allocation, network or persistence.
 * Outputs are numeric state/biome IDs without name/air/opacity semantics.
 * Only sections*4096 / sections*64 elements are written; tails untouched. */
int32_t craftrustnx_column_copy(const uint8_t *body, uint32_t length,
    const struct wc_state *world, const struct pl_state *play,
    uint32_t *blocks, uint32_t block_capacity,
    uint32_t *biomes, uint32_t biome_capacity, struct col_info *info);
#endif
