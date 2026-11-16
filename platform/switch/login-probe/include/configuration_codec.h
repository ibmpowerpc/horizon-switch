#ifndef CRAFTRUSTNX_CONFIGURATION_CODEC_H
#define CRAFTRUSTNX_CONFIGURATION_CODEC_H
#include <stdint.h>
enum { CF_OK = 0, CF_NEED_MORE = 1, CF_BAD_ARGUMENT = -1,
    CF_OUTPUT_SMALL = -2, CF_BAD_FRAME = -3, CF_UNSUPPORTED = -5,
    CF_MAX_BODY = 8388608, CF_MAX_FRAME = 2097151, CF_MAX_WIRE = 2097156,
    CF_REPLY_BODY = 1024 };
enum { CF_PHASE_CONFIGURATION = 0, CF_PHASE_PLAY = 1 };
enum cf_kind { CF_METADATA = 1, CF_FINISH, CF_KEEP_ALIVE, CF_PING,
    CF_KNOWN_PACKS, CF_COOKIE_REQUEST, CF_RESOURCE_PACK, CF_DISCONNECT,
    CF_USER_ACTION, CF_TRANSFER, CF_PLAY_PACKET };
struct cf_event {
    uint32_t kind;
    uint32_t consumed;
    uint32_t body_length;
    int32_t packet_id;
    uint32_t payload_offset; /* Region in scratch after packet ID. */
    uint32_t payload_length;
    uint32_t reply_length; /* Unframed response body in caller reply buffer. */
    uint32_t flags; /* Resource-pack required bit is bit 0. */
};
/* Valid caller-owned aligned non-overlapping input/scratch/reply/event regions.
 * Decode ONE complete (decrypted) frame, return consumed for buffered tails.
 * Known registry/NBT/metadata payloads are deliberately opaque, not loaded or
 * semantically validated. No claim of having installed packs or world data.
 * Control replies: keepalive, pong, empty known packs, no cookie, pack declined,
 * finish acknowledgement. Code of conduct returns USER_ACTION without consent.
 * No networking, allocation, persistence, downloading or following transfers.
 * Event zeroed on errors; NEED_MORE only for incomplete outer frame. */
int32_t craftrustnx_configuration_decode(const uint8_t *data, uint32_t length,
    int32_t compression, uint32_t phase, uint8_t *scratch, uint32_t scratch_capacity,
    uint8_t *reply, uint32_t reply_capacity, struct cf_event *event);
/* Frame a client body of 1..1024 bytes. Threshold -1: raw; >=0: compression
 * envelope, with a valid stored-block zlib stream when threshold requires it.
 * Failure leaves out unchanged and written zero. Ciphering is caller's job. */
int32_t craftrustnx_configuration_frame(const uint8_t *body, uint32_t length,
    int32_t compression, uint8_t *out, uint32_t capacity, uint32_t *written);
#endif
