#ifndef CRAFTRUSTNX_PLAY_CODEC_H
#define CRAFTRUSTNX_PLAY_CODEC_H
#include <stdint.h>
enum { PL_OK = 0, PL_BAD_ARGUMENT = -1, PL_OUTPUT_SMALL = -2,
    PL_BAD_FRAME = -3, PL_UNSUPPORTED = -5, PL_REPLY_BODY = 1024,
    PL_MAX_BODY = 8388608, PL_DIMENSION_CAP = 257 };
enum pl_kind { PL_METADATA = 1, PL_LOGIN, PL_POSITION, PL_KEEP_ALIVE, PL_PING,
    PL_CHUNK, PL_BATCH_START, PL_BATCH_END, PL_COOKIE, PL_RESOURCE_PACK,
    PL_DISCONNECT, PL_RECONFIGURE, PL_TRANSFER, PL_RESPAWN };
/* Fixed ABI: 320 bytes, alignment 8 on AArch64 and the host. IEEE binary
 * position/rotation bits are stored in native-endian integers; never NaN/Inf.
 * Zero-initialize for a new connection. Position is valid only after an absolute
 * position is received. Partial-relative first teleports are UNSUPPORTED. */
struct pl_state {
    uint64_t position_bits[3];
    uint32_t yaw_bits;
    uint32_t pitch_bits;
    int32_t player_id;
    int32_t dimension_type;
    int32_t game_mode;
    int32_t teleport_id;
    uint32_t login_seen;
    uint32_t position_seen;
    char dimension[PL_DIMENSION_CAP];
};
/* Exactly 32 bytes. payload_* refer to the supplied uncompressed body after ID.
 * CHUNK value/aux are column x/z (only that header is parsed). BATCH_END value
 * is declared batch size; POSITION value is teleport ID. Pack flags bit0 required. */
struct pl_event {
    uint32_t kind;
    int32_t packet_id;
    uint32_t reply_length;
    uint32_t payload_offset;
    uint32_t payload_length;
    int32_t value;
    int32_t aux;
    uint32_t flags;
};
/* Parse ONE already-unframed/inflated play body. The existing configuration
 * framing decoder (phase PLAY) owns stream caps/compression and consumed bytes.
 * No allocation, persistence, network or world simulation. State changes only
 * on success; event zeroed on errors, reply unchanged on errors. Valid aligned
 * non-overlapping caller-owned body/state/reply/event regions are required.
 * Known data packets stay opaque. No resource installation or PlayerLoaded.
 * Replies are UNFRAMED bodies: caller frames/encrypts/sends exactly once. */
int32_t craftrustnx_play_decode_body(const uint8_t *body, uint32_t length,
    struct pl_state *state, uint8_t *reply, uint32_t reply_capacity,
    struct pl_event *event);
#endif
