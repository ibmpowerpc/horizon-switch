/* CFB8 stream cipher mode - private header for crypto implementations */
#ifndef CRAFTRUSTNX_CFB8_H
#define CRAFTRUSTNX_CFB8_H
#include <stdint.h>

/* Block cipher callback: encrypt 16-byte src into dst using context.
 * Returns 0 on success, non-zero on error. Context is caller-owned. */
typedef int (*cfb8_block_fn)(void *ctx, const uint8_t src[16], uint8_t dst[16]);

/* CFB8 encryption/decryption with arbitrary chunk sizes including zero.
 * shift_reg: 16-byte shift register, updated after processing.
 * block_fn: AES block encryption callback.
 * block_ctx: opaque context passed to block_fn.
 * data: buffer processed in-place.
 * length: number of bytes to process.
 * Returns 0 on success, propagates block_fn error code on failure. */
int cfb8_process(uint8_t shift_reg[16], cfb8_block_fn block_fn, void *block_ctx,
    uint8_t *data, uint32_t length, int is_decrypt);

#endif
