#ifndef CRAFTRUSTNX_NET_PROBE_H
#define CRAFTRUSTNX_NET_PROBE_H
#include <stdbool.h>
#include <stdint.h>
#include <stdatomic.h>

enum { NP_HOST_CAPACITY = 254, NP_REPLY_CAPACITY = 16400 };
enum np_stage {
    NP_IDLE, NP_RESOLVING, NP_CONNECTING, NP_SENDING, NP_RECEIVING,
    NP_DONE, NP_ERROR, NP_CANCELLED
};
enum np_error {
    NP_ERR_NONE, NP_ERR_ARGUMENT, NP_ERR_DNS, NP_ERR_CONNECT, NP_ERR_TIMEOUT,
    NP_ERR_SEND, NP_ERR_RECV, NP_ERR_EOF, NP_ERR_PROTOCOL, NP_ERR_CLOCK
};
struct np_job {
    char host[NP_HOST_CAPACITY];
    uint16_t port;
    int32_t protocol;
    atomic_bool cancel;
    atomic_int stage;
    enum np_error error;
    int system_error;
    int resolver_error;
    int codec_error;
    uint32_t response_length;
    uint32_t json_offset;
    uint32_t json_length;
    uint8_t response[NP_REPLY_CAPACITY];
};
// Run synchronously on a worker after socketInitializeDefault on Switch.
// Main may set cancel/read stage atomically, but must only read result fields
// after acquiring a terminal stage, and must join before reusing this job.
// No socket service initialization, UI, file access or libnx thread API here.
void np_run(struct np_job *job);
#endif
