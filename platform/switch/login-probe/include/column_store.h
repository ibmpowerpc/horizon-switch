#ifndef CRAFTRUSTNX_COLUMN_STORE_H
#define CRAFTRUSTNX_COLUMN_STORE_H
#include "column_codec.h"
#include <stddef.h>
/* Save a terminal, worker-joined column, never an active network buffer.
 * base is a trusted directory. Each successful call creates an exclusive
 * capture-N directory, closes a temporary file, renames and verifies readback.
 * Existing captures are never replaced. Raw world IDs are private user data.
 * Return 0, -1 argument, -2 Rust codec, -3 file/heap error, -4 readback mismatch.
 * path is cleared on failure; codec_error receives the Rust status or zero. */
int lj_store_column(const char *base, const struct col_info *info,
    const uint32_t *blocks, const uint32_t *biomes,
    char *path, size_t path_capacity, int *codec_error);
#endif
