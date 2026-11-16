#ifndef CRAFTRUSTNX_LOGIN_PROBE_H
#define CRAFTRUSTNX_LOGIN_PROBE_H
#include "login_codec.h"
#include "play_codec.h"
#include "world_codec.h"
#include <stdbool.h>
#include <stdatomic.h>
#include <stdint.h>

enum lj_stage { LJ_IDLE, LJ_RESOLVING, LJ_CONNECTING, LJ_SENDING, LJ_RECEIVING,
    LJ_ACCEPTED, LJ_DISCONNECTED, LJ_AUTH_REQUIRED, LJ_UNSUPPORTED,
    LJ_ERROR, LJ_CANCELLED, LJ_KEY_EXCHANGE, LJ_CONFIGURING, LJ_PLAY_WAIT,
    LJ_PLAY_PACKET_SEEN, LJ_ACTION_REQUIRED, LJ_PLAY_RECEIVING, LJ_PLAY_SESSION_OBSERVED, LJ_WORLD_DATA_OBSERVED };
enum lj_error { LJ_ERR_NONE, LJ_ERR_ARGUMENT, LJ_ERR_DNS, LJ_ERR_CONNECT,
    LJ_ERR_TIMEOUT, LJ_ERR_SEND, LJ_ERR_RECV, LJ_ERR_EOF, LJ_ERR_PROTOCOL,
    LJ_ERR_CLOCK, LJ_ERR_CRYPTO, LJ_ERR_MEMORY, LJ_ERR_LIMIT };
struct lj_job {
    char host[254];
    char name[17];
    uint16_t port;
    atomic_bool cancel;
    atomic_int stage;
    enum lj_error error;
    int system_error;
    int resolver_error;
    int codec_error;
    int crypto_error;
    int crypto_detail;
    bool saw_encryption_request;
    bool server_requested_auth;
    bool encryption_active;
    bool login_accepted;
    bool configuration_acknowledged;
    bool resource_pack_declined;
    bool required_pack_declined;
    uint32_t configuration_frames;
    uint32_t configuration_bytes;
    uint32_t registry_frames;
    int32_t last_configuration_id;
    int32_t first_play_id;
    uint32_t first_play_body_bytes;
    struct pl_state play;
    uint32_t play_frames;
    uint32_t play_bytes;
    uint32_t teleports_acked;
    uint32_t keepalives_replied;
    uint32_t pongs_replied;
    uint32_t chunk_frames;
    uint32_t chunk_batches;
    int32_t last_play_id;
    int32_t last_chunk_x;
    int32_t last_chunk_z;
    struct wc_state world;
    struct wc_event world_sample;
    struct wc_event world_last;
    int32_t world_error;
    uint32_t validated_chunks;
    uint32_t validated_light_updates;
    uint32_t validated_sections;
    uint32_t validated_heightmaps;
    uint32_t validated_block_entities;
    uint32_t validated_sky_layers;
    uint32_t validated_block_layers;
    int32_t compression;
    uint32_t received_bytes;
    uint32_t frame_count;
    struct lp_event event;
    uint8_t wire[LP_MAX_WIRE];
    uint8_t scratch[LP_MAX_BODY];
};
/* Synchronous worker, sockets initialized by caller. Caller owns static job;
 * only cancel/stage may be accessed while working. Result fields only after
 * acquiring a terminal stage. Join before reusing job/closing socket service.
 * Login variants stop at LoginFinished. Configuration build acknowledges it
 * and stops after the first framed play packet. Play-session build observes
 * login, pose ACK, opaque chunk batch and keepalive. No world is loaded. */
void lj_run(struct lj_job *job);
#endif
