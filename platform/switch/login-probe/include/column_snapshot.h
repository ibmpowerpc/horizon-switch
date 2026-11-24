#ifndef CRAFTRUSTNX_COLUMN_SNAPSHOT_H
#define CRAFTRUSTNX_COLUMN_SNAPSHOT_H
#include <stdint.h>
/* CTCOL001 binary snapshot: 48-byte header + block_ids + biome_ids.
 * All integers little-endian. Header: magic "CTCOL001" (8), protocol 777 (4),
 * format 1 (4), chunk_x (4), chunk_z (4), min_y (4), sections (4),
 * blocks_written (4), biomes_written (4), reserved (8).
 * No light retention, name mapping or readiness assumptions. Pure raw u32 IDs. */
enum { SNAP_OK=0, SNAP_BAD_ARGUMENT=-1, SNAP_OUTPUT_SMALL=-2, SNAP_BAD_FRAME=-3 };
struct snap_info {
    int32_t chunk_x;
    int32_t chunk_z;
    int32_t min_y;
    uint32_t sections;
    uint32_t blocks_written;
    uint32_t biomes_written;
};
/* Header size: 48 bytes. */
enum { SNAP_HEADER_SIZE=48 };
/* Maximum sections: 256. */
enum { SNAP_MAX_SECTIONS=256 };
/* Maximum blocks per column: 256*4096. */
enum { SNAP_MAX_BLOCKS=256*4096 };
/* Maximum biomes per column: 256*64. */
enum { SNAP_MAX_BIOMES=256*64 };
/* Maximum file size in bytes: 48 + 256*(4096+64)*4. */
enum { SNAP_MAX_FILE_SIZE=48+256*(4096+64)*4 };
/* Every buffer must be valid, aligned where needed, non-overlapping and have
 * its stated readable/writable extent. Null pointers and zero or over-limit
 * file lengths are BAD_ARGUMENT. Capacity bounds are specified below.
 * On failure written/info is zeroed and
 * data arrays/output bytes are unchanged; successful writes leave tails alone.
 * Header-only decode accepts 48..MAX_FILE_SIZE bytes without requiring a body.
 * A nonempty truncated/malformed header is BAD_FRAME. */
int32_t craftrustnx_snapshot_decode_header(const uint8_t *data, uint32_t length, struct snap_info *info);
/* Encode snapshot into caller-owned output. Blocks/biomes raw u32 IDs.
 * Counts must equal sections*4096 and sections*64; sections 1..256, min_y a
 * multiple of 16 and min_y+sections*16 fits i32, else BAD_ARGUMENT.
 * Out capacity 1..SNAP_MAX_FILE_SIZE; insufficient nonzero capacity is
 * OUTPUT_SMALL. Written is the complete file byte count on success. */
int32_t craftrustnx_snapshot_encode(int32_t chunk_x, int32_t chunk_z, int32_t min_y,
    uint32_t sections, const uint32_t *blocks, uint32_t blocks_written,
    const uint32_t *biomes, uint32_t biomes_written, uint8_t *out, uint32_t capacity, uint32_t *written);
/* Decode an exact-length file. Truncation/trailing bytes or invalid header
 * fields are BAD_FRAME (empty/over-limit length is BAD_ARGUMENT).
 * Capacities are in u32 elements, 0..SNAP_MAX_*; insufficient capacities are
 * OUTPUT_SMALL. Only sections*4096 / sections*64 elements are written. */
int32_t craftrustnx_snapshot_decode(const uint8_t *data, uint32_t length,
    uint32_t *blocks, uint32_t block_capacity,
    uint32_t *biomes, uint32_t biome_capacity, struct snap_info *info);
/* Maximum file size in bytes. */
uint32_t craftrustnx_snapshot_max_file_size(void);
#endif
