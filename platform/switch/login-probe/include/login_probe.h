#ifndef CRAFTRUSTNX_LOGIN_PROBE_H
#define CRAFTRUSTNX_LOGIN_PROBE_H
#include "login_codec.h"
#include <stdbool.h>
#include <stdatomic.h>
#include <stdint.h>

enum lj_stage { LJ_IDLE, LJ_RESOLVING, LJ_CONNECTING, LJ_SENDING, LJ_RECEIVING,
    LJ_ACCEPTED, LJ_DISCONNECTED, LJ_AUTH_REQUIRED, LJ_UNSUPPORTED,
    LJ_ERROR, LJ_CANCELLED };
enum lj_error { LJ_ERR_NONE, LJ_ERR_ARGUMENT, LJ_ERR_DNS, LJ_ERR_CONNECT,
    LJ_ERR_TIMEOUT, LJ_ERR_SEND, LJ_ERR_RECV, LJ_ERR_EOF, LJ_ERR_PROTOCOL,
    LJ_ERR_CLOCK };
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
 * No LoginAcknowledged is sent: ACCEPTED ends at validated LoginFinished. */
void lj_run(struct lj_job *job);
#endif
