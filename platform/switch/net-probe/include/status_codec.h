#ifndef CRAFTRUSTNX_STATUS_CODEC_H
#define CRAFTRUSTNX_STATUS_CODEC_H
#include <stdint.h>

enum {
    STATUS_OK = 0,
    STATUS_NEED_MORE = 1,
    STATUS_BAD_ARGUMENT = -1,
    STATUS_OUTPUT_SMALL = -2,
    STATUS_BAD_FRAME = -3,
    STATUS_BAD_HOST = -4
};

// All non-null buffers/outputs must point to caller-owned, non-overlapping
// memory of the declared length. No allocation, logging or networking occurs.
int32_t craftrustnx_status_encode(const char *host, uint32_t host_len,
    uint16_t port, int32_t protocol, uint8_t *out, uint32_t capacity, uint32_t *written);
int32_t craftrustnx_status_decode(const uint8_t *data, uint32_t length,
    uint32_t *json_offset, uint32_t *json_length, uint32_t *consumed);
uint32_t craftrustnx_status_max_frame_bytes(void);
#endif
