#ifndef CRAFTRUSTNX_WORLD_CODEC_H
#define CRAFTRUSTNX_WORLD_CODEC_H
#include <stdint.h>
#include "play_codec.h"
enum { WC_OK = 0, WC_BAD_ARGUMENT = -1, WC_BAD_FRAME = -3,
    WC_UNSUPPORTED = -5, WC_LIMIT = -6, WC_MAX_BODY = 8388608,
    WC_MAX_DIMENSIONS = 32, WC_MAX_SECTIONS = 256 };
enum { WC_CONFIGURATION = 0, WC_PLAY = 1 };
enum wc_kind { WC_METADATA = 1, WC_DIMENSIONS, WC_BIOMES, WC_CHUNK, WC_LIGHT };
struct wc_dimension {
    int32_t min_y;
    int32_t height;
    uint32_t has_skylight;
    uint32_t valid;
};
/* 528 bytes / align4. Zero-initialize for a new connection. Only selected
 * configuration registries are retained; block-state names are not installed. */
struct wc_state {
    uint32_t dimension_count;
    uint32_t biome_count;
    uint32_t dimensions_seen;
    uint32_t biomes_seen;
    struct wc_dimension dimensions[WC_MAX_DIMENSIONS];
};
/* 72 bytes / align4. Section/count/light structure validated, not a world load.
 * Sample is the block below the last acknowledged pose, only for its column
 * and when within this dimension; raw state/biome IDs have no name mapping. */
struct wc_event {
    uint32_t kind;
    int32_t packet_id;
    uint32_t entries;
    int32_t chunk_x;
    int32_t chunk_z;
    uint32_t sections;
    uint32_t heightmaps;
    uint32_t block_entities;
    uint32_t sky_layers;
    uint32_t block_layers;
    uint32_t nonempty_blocks;
    uint32_t fluid_blocks;
    uint32_t sample_valid;
    int32_t sample_x;
    int32_t sample_y;
    int32_t sample_z;
    int32_t state_id;
    int32_t biome_id;
};
/* ONE complete uncompressed body, ID included, 1..8MiB. No allocation/network/
 * persistence or readiness/movement packets. Config id7 retains dimension_type
 * and worldgen/biome registries; play46/49 validates chunk/light wire structure.
 * Other packets are opaque metadata. Caller owns valid aligned non-overlapping
 * body/state/play/event buffers. State unchanged and event zeroed on failure.
 * No semantic claims about block names, rendering, full lighting or PlayerLoaded. */
int32_t craftrustnx_world_decode_body(const uint8_t *body, uint32_t length,
    uint32_t phase, struct wc_state *state, const struct pl_state *play,
    struct wc_event *event);
#endif
