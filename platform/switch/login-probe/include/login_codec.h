#ifndef CRAFTRUSTNX_LOGIN_CODEC_H
#define CRAFTRUSTNX_LOGIN_CODEC_H
#include <stdint.h>

enum { LP_OK = 0, LP_NEED_MORE = 1, LP_BAD_ARGUMENT = -1,
    LP_OUTPUT_SMALL = -2, LP_BAD_FRAME = -3, LP_BAD_PROFILE = -4,
    LP_UNSUPPORTED = -5, LP_MAX_BODY = 65536, LP_MAX_WIRE = 65541 };
enum lp_event_kind { LP_LOGIN_SUCCESS = 1, LP_DISCONNECT, LP_ENCRYPTION_REQUEST,
    LP_SET_COMPRESSION, LP_CUSTOM_QUERY, LP_COOKIE_REQUEST };
struct lp_event {
    uint32_t kind;
    uint32_t consumed;
    uint32_t data_offset; /* Payload after packet ID, in scratch. */
    uint32_t data_length;
    int32_t threshold; /* Only meaningful for SET_COMPRESSION. */
    uint8_t uuid[16];
    char name[17]; /* Server profile, NUL terminated; only for LOGIN_SUCCESS. */
    uint8_t session[16]; /* Java 26.3 LoginFinished session UUID. */
};
/* Caller-owned, valid, aligned, non-overlapping buffers/outputs; no allocation.
 * Fixed protocol 777. Username: 1..16 ASCII letters, digits, underscore.
 * Encodes handshake next-state login + LoginStart with offline UUID v3. */
int32_t craftrustnx_login_start(const char *host, uint32_t host_len, uint16_t port,
    const char *name, uint32_t name_len, uint8_t *out, uint32_t capacity,
    uint32_t *written);
/* compression_threshold=-1 means framing before SetCompression. Otherwise
 * validate the compression envelope and zlib body against threshold and caps.
 * scratch contains the decompressed packet body (ID included). Returns only
 * structurally validated events. Outputs reset on errors, incomplete outer
 * frames return NEED_MORE. Never logs in or acknowledges configuration itself. */
int32_t craftrustnx_login_decode(const uint8_t *data, uint32_t length,
    int32_t compression_threshold, uint8_t *scratch, uint32_t scratch_capacity,
    struct lp_event *event);
#endif
