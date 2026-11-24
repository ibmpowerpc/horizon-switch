/* Independent C caller checks for the frozen CTCOL001 column snapshot ABI.
 *
 * Expected wire bytes are written by hand here in little-endian order, never by
 * the codec under test, so the fixtures are independent of Rust internals.
 * Covers: negative coordinates/min_y, raw u32 IDs (0, 255, 256, 0x1FF,
 * 0xDEADBEEF, UINT32_MAX), exact header/body bytes and ordering, header-only
 * decode, truncation/trailing/short output, null and over-limit arguments,
 * header corruptions, transactional failure (no writes, zero info/written),
 * untouched tails/inputs, and the 256-section boundary.
 *
 * Build/run through tests/run-snapshot-host.sh, which links the existing host
 * staticlib. ASan/UBSan instrument this C file only; the Rust staticlib is not
 * rebuilt with sanitizers. */
#include "column_snapshot.h"
#include <limits.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

_Static_assert(sizeof(struct snap_info) == 24, "snap_info ABI must be 24 bytes");
_Static_assert(SNAP_HEADER_SIZE == 48, "header must be 48 bytes");
_Static_assert(SNAP_MAX_FILE_SIZE == 4259888, "max file size must be 4259888");
_Static_assert(SNAP_MAX_BLOCKS == 1048576, "max blocks must be 1048576");
_Static_assert(SNAP_MAX_BIOMES == 16384, "max biomes must be 16384");

static int failures;

static void fail(const char *what, int line) {
    fprintf(stderr, "FAIL %s:%d %s\n", __FILE__, line, what);
    ++failures;
}
#define CHECK(cond, what) ((cond) ? (void)0 : fail((what), __LINE__))

static void put_le32(uint8_t *p, uint32_t value) {
    p[0] = (uint8_t)value;
    p[1] = (uint8_t)(value >> 8);
    p[2] = (uint8_t)(value >> 16);
    p[3] = (uint8_t)(value >> 24);
}

static uint32_t get_le32(const uint8_t *p) {
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) |
           ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

static uint8_t *alloc_u8(size_t count) {
    uint8_t *p = malloc(count ? count : 1);
    if (!p) {
        fputs("out of memory\n", stderr);
        exit(2);
    }
    return p;
}

static uint32_t *alloc_u32(size_t count) {
    uint32_t *p = malloc((count ? count : 1) * sizeof *p);
    if (!p) {
        fputs("out of memory\n", stderr);
        exit(2);
    }
    return p;
}

static int bytes_are(const uint8_t *p, uint32_t count, uint8_t value) {
    for (uint32_t i = 0; i < count; ++i)
        if (p[i] != value) return 0;
    return 1;
}

static int arrays_are(const uint32_t *p, uint32_t count, uint32_t value) {
    for (uint32_t i = 0; i < count; ++i)
        if (p[i] != value) return 0;
    return 1;
}

static int same_u32(const uint32_t *a, const uint32_t *b, uint32_t count) {
    for (uint32_t i = 0; i < count; ++i)
        if (a[i] != b[i]) return 0;
    return 1;
}

static int info_is_zero(const struct snap_info *info) {
    return info->chunk_x == 0 && info->chunk_z == 0 && info->min_y == 0 &&
           info->sections == 0 && info->blocks_written == 0 &&
           info->biomes_written == 0;
}

static uint32_t file_length(uint32_t sections) {
    return (uint32_t)SNAP_HEADER_SIZE + sections * (4096u + 64u) * 4u;
}

/* Pattern carries raw IDs that no air/name mapping may rewrite. */
static void fill_pattern(uint32_t sections, uint32_t *blocks, uint32_t *biomes) {
    const uint32_t block_count = sections * 4096u;
    const uint32_t biome_count = sections * 64u;
    for (uint32_t i = 0; i < block_count; ++i) blocks[i] = 0x10000000u + i;
    for (uint32_t i = 0; i < biome_count; ++i) biomes[i] = 0x20000000u + i;
    blocks[0] = 0u;
    blocks[1] = 255u;
    blocks[2] = 256u;
    blocks[3] = 0xFFFFFFFFu;
    blocks[100] = 0x1FFu;
    blocks[block_count - 1] = 0xDEADBEEFu;
    biomes[0] = 0u;
    biomes[1] = 300u;
    biomes[biome_count - 1] = 0xFFFFFFFFu;
}

static void write_header(uint8_t *h, int32_t chunk_x, int32_t chunk_z,
                         int32_t min_y, uint32_t sections, uint32_t blocks,
                         uint32_t biomes) {
    memcpy(h, "CTCOL001", 8);
    put_le32(h + 8, 777u);
    put_le32(h + 12, 1u);
    put_le32(h + 16, (uint32_t)chunk_x);
    put_le32(h + 20, (uint32_t)chunk_z);
    put_le32(h + 24, (uint32_t)min_y);
    put_le32(h + 28, sections);
    put_le32(h + 32, blocks);
    put_le32(h + 36, biomes);
    put_le32(h + 40, 0u);
    put_le32(h + 44, 0u);
}

/* Writes header, all blocks LE, then all biomes LE. Returns bytes written. */
static uint32_t build_bytes(uint8_t *out, int32_t chunk_x, int32_t chunk_z,
                            int32_t min_y, uint32_t sections,
                            const uint32_t *blocks, const uint32_t *biomes) {
    const uint32_t block_count = sections * 4096u;
    const uint32_t biome_count = sections * 64u;
    uint32_t offset = (uint32_t)SNAP_HEADER_SIZE;
    write_header(out, chunk_x, chunk_z, min_y, sections, block_count, biome_count);
    for (uint32_t i = 0; i < block_count; ++i, offset += 4) put_le32(out + offset, blocks[i]);
    for (uint32_t i = 0; i < biome_count; ++i, offset += 4) put_le32(out + offset, biomes[i]);
    return offset;
}

/* Explicit little-endian header for (-2, -3, -64, sections=1). */
static const uint8_t expected_header_1[48] = {
    0x43, 0x54, 0x43, 0x4F, 0x4C, 0x30, 0x30, 0x31, /* "CTCOL001" */
    0x09, 0x03, 0x00, 0x00,                         /* protocol 777 */
    0x01, 0x00, 0x00, 0x00,                         /* format 1 */
    0xFE, 0xFF, 0xFF, 0xFF,                         /* chunk_x -2 */
    0xFD, 0xFF, 0xFF, 0xFF,                         /* chunk_z -3 */
    0xC0, 0xFF, 0xFF, 0xFF,                         /* min_y -64 */
    0x01, 0x00, 0x00, 0x00,                         /* sections 1 */
    0x00, 0x10, 0x00, 0x00,                         /* blocks 4096 */
    0x40, 0x00, 0x00, 0x00,                         /* biomes 64 */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00  /* reserved */
};

static void test_limits_abi(void) {
    CHECK(craftrustnx_snapshot_max_file_size() == (uint32_t)SNAP_MAX_FILE_SIZE,
          "max_file_size matches header");
    CHECK((uint32_t)SNAP_MAX_FILE_SIZE ==
              (uint32_t)SNAP_HEADER_SIZE + (uint32_t)SNAP_MAX_SECTIONS * (4096u + 64u) * 4u,
          "max formula 48+256*4160*4");
    CHECK(sizeof(struct snap_info) == 24, "snap_info size");
    CHECK(offsetof(struct snap_info, chunk_x) == 0, "chunk_x offset");
    CHECK(offsetof(struct snap_info, chunk_z) == 4, "chunk_z offset");
    CHECK(offsetof(struct snap_info, min_y) == 8, "min_y offset");
    CHECK(offsetof(struct snap_info, sections) == 12, "sections offset");
    CHECK(offsetof(struct snap_info, blocks_written) == 16, "blocks_written offset");
    CHECK(offsetof(struct snap_info, biomes_written) == 20, "biomes_written offset");
    struct snap_info info = {INT32_MIN, INT32_MAX, -16, 256u, 1048576u, 16384u};
    CHECK(info.chunk_x == INT32_MIN && info.chunk_z == INT32_MAX && info.min_y == -16,
          "signed 32-bit coordinate fields");
    CHECK(info.sections == 256u && info.blocks_written == 1048576u &&
              info.biomes_written == 16384u,
          "unsigned 32-bit count fields");
}

static void test_encode_exact(void) {
    const uint32_t sections = 1, blocks_count = 4096, biomes_count = 64;
    const uint32_t length = file_length(sections);
    uint32_t *blocks = alloc_u32(blocks_count);
    uint32_t *biomes = alloc_u32(biomes_count);
    uint32_t *blocks_copy = alloc_u32(blocks_count);
    uint32_t *biomes_copy = alloc_u32(biomes_count);
    fill_pattern(sections, blocks, biomes);
    memcpy(blocks_copy, blocks, blocks_count * sizeof *blocks);
    memcpy(biomes_copy, biomes, biomes_count * sizeof *biomes);

    uint8_t *expected = alloc_u8(length);
    CHECK(build_bytes(expected, -2, -3, -64, sections, blocks, biomes) == length,
          "fixture length");
    CHECK(memcmp(expected, expected_header_1, sizeof expected_header_1) == 0,
          "hand-written header matches fixture header");

    uint8_t *out = alloc_u8(length + 16);
    memset(out, 0xAA, length + 16);
    uint32_t written = 0xDEADBEEFu;
    int32_t status = craftrustnx_snapshot_encode(-2, -3, -64, sections,
        blocks, blocks_count, biomes, biomes_count, out, length, &written);
    CHECK(status == SNAP_OK, "encode returns SNAP_OK");
    CHECK(written == length, "encode written exact length");
    CHECK(memcmp(out, expected, length) == 0, "encode exact LE header/body bytes");
    CHECK(memcmp(out, expected_header_1, sizeof expected_header_1) == 0,
          "encode header matches explicit bytes");
    CHECK(bytes_are(out + length, 16, 0xAA), "encode output tail untouched");
    CHECK(same_u32(blocks, blocks_copy, blocks_count) &&
              same_u32(biomes, biomes_copy, biomes_count),
          "encode leaves input arrays untouched");

    uint32_t offset = (uint32_t)SNAP_HEADER_SIZE;
    for (uint32_t i = 0; i < blocks_count; ++i, offset += 4)
        if (get_le32(out + offset) != blocks[i]) {
            CHECK(0, "encode block order/LE");
            break;
        }
    for (uint32_t i = 0; i < biomes_count; ++i, offset += 4)
        if (get_le32(out + offset) != biomes[i]) {
            CHECK(0, "encode biome order/LE after blocks");
            break;
        }
    CHECK(get_le32(out + 48) == 0u && get_le32(out + 48 + 4) == 255u &&
              get_le32(out + 48 + 8) == 256u &&
              get_le32(out + 48 + 12) == 0xFFFFFFFFu,
          "raw IDs 0/255/256/UINT32_MAX preserved");
    CHECK(get_le32(out + 48 + (blocks_count - 1) * 4) == 0xDEADBEEFu,
          "last block sentinel before biomes");
    CHECK(get_le32(out + 48 + blocks_count * 4) == 0u,
          "first biome follows last block");
    CHECK(get_le32(out + 48 + blocks_count * 4 + (biomes_count - 1) * 4) == 0xFFFFFFFFu,
          "last biome sentinel");

    /* Larger capacity is allowed; the codec must write only the file. */
    memset(out, 0xBB, length + 16);
    written = 0;
    CHECK(craftrustnx_snapshot_encode(-2, -3, -64, sections, blocks, blocks_count,
              biomes, biomes_count, out, length + 16, &written) == SNAP_OK,
          "encode accepts capacity above file length");
    CHECK(written == length, "larger capacity writes file length");
    CHECK(memcmp(out, expected, length) == 0 && bytes_are(out + length, 16, 0xBB),
          "larger capacity leaves tail and bytes exact");

    free(out);
    free(expected);
    free(biomes_copy);
    free(blocks_copy);
    free(biomes);
    free(blocks);
}

static void test_decode_exact(void) {
    const uint32_t sections = 1, blocks_count = 4096, biomes_count = 64;
    const uint32_t length = file_length(sections);
    uint32_t *blocks = alloc_u32(blocks_count);
    uint32_t *biomes = alloc_u32(biomes_count);
    fill_pattern(sections, blocks, biomes);
    uint8_t *bytes = alloc_u8(length);
    CHECK(build_bytes(bytes, -2, -3, -64, sections, blocks, biomes) == length,
          "decode fixture length");

    uint32_t *out_blocks = alloc_u32(blocks_count + 8);
    uint32_t *out_biomes = alloc_u32(biomes_count + 8);
    memset(out_blocks, 0xCC, (blocks_count + 8) * sizeof *out_blocks);
    memset(out_biomes, 0xDD, (biomes_count + 8) * sizeof *out_biomes);
    struct snap_info info;
    memset(&info, 0x5A, sizeof info);
    CHECK(craftrustnx_snapshot_decode(bytes, length, out_blocks, blocks_count + 8,
              out_biomes, biomes_count + 8, &info) == SNAP_OK,
          "decode returns SNAP_OK");
    CHECK(info.chunk_x == -2 && info.chunk_z == -3 && info.min_y == -64,
          "decode signed coordinates/min_y");
    CHECK(info.sections == sections && info.blocks_written == blocks_count &&
              info.biomes_written == biomes_count,
          "decode exact header counts");
    CHECK(same_u32(out_blocks, blocks, blocks_count), "decode block order/IDs");
    CHECK(same_u32(out_biomes, biomes, biomes_count), "decode biome order/IDs");
    CHECK(arrays_are(out_blocks + blocks_count, 8, 0xCCCCCCCCu),
          "decode block tail untouched");
    CHECK(arrays_are(out_biomes + biomes_count, 8, 0xDDDDDDDDu),
          "decode biome tail untouched");

    /* Exact capacities must also succeed and write the full extent. */
    memset(out_blocks, 0xCC, (blocks_count + 8) * sizeof *out_blocks);
    memset(out_biomes, 0xDD, (biomes_count + 8) * sizeof *out_biomes);
    memset(&info, 0x5A, sizeof info);
    CHECK(craftrustnx_snapshot_decode(bytes, length, out_blocks, blocks_count,
              out_biomes, biomes_count, &info) == SNAP_OK,
          "decode exact capacities accepted");
    CHECK(same_u32(out_blocks, blocks, blocks_count) &&
              same_u32(out_biomes, biomes, biomes_count),
          "decode exact capacities fill arrays");
    CHECK(arrays_are(out_blocks + blocks_count, 8, 0xCCCCCCCCu) &&
              arrays_are(out_biomes + biomes_count, 8, 0xDDDDDDDDu),
          "exact capacity still leaves caller tail");

    /* Signed extremes are raw header values, not clamped. */
    uint8_t *edge = alloc_u8(length);
    CHECK(build_bytes(edge, INT32_MIN, INT32_MAX, INT32_MIN, sections, blocks, biomes) == length,
          "edge fixture length");
    memset(&info, 0x5A, sizeof info);
    CHECK(craftrustnx_snapshot_decode(edge, length, out_blocks, blocks_count + 8,
              out_biomes, biomes_count + 8, &info) == SNAP_OK,
          "decode int32 extreme chunk/min_y");
    CHECK(info.chunk_x == INT32_MIN && info.chunk_z == INT32_MAX &&
              info.min_y == INT32_MIN,
          "decode preserves int32 extremes");

    free(edge);
    free(out_biomes);
    free(out_blocks);
    free(bytes);
    free(biomes);
    free(blocks);
}

static void expect_header_error(const uint8_t *data, uint32_t length,
                                const char *name, int want_bad_frame) {
    struct snap_info info;
    memset(&info, 0x5A, sizeof info);
    int32_t status = craftrustnx_snapshot_decode_header(data, length, &info);
    if (want_bad_frame) CHECK(status == SNAP_BAD_FRAME, name);
    else CHECK(status != SNAP_OK, name);
    CHECK(info_is_zero(&info), "header failure zeroes info");
}

static void test_header_only(void) {
    const uint32_t sections = 1, blocks_count = 4096, biomes_count = 64;
    const uint32_t length = file_length(sections);
    uint32_t *blocks = alloc_u32(blocks_count);
    uint32_t *biomes = alloc_u32(biomes_count);
    fill_pattern(sections, blocks, biomes);
    uint8_t *bytes = alloc_u8(length);
    build_bytes(bytes, -2, -3, -64, sections, blocks, biomes);

    struct snap_info info;
    memset(&info, 0x5A, sizeof info);
    CHECK(craftrustnx_snapshot_decode_header(bytes, SNAP_HEADER_SIZE, &info) == SNAP_OK,
          "header-only accepts 48-byte header without body");
    CHECK(info.chunk_x == -2 && info.chunk_z == -3 && info.min_y == -64 &&
              info.sections == 1 && info.blocks_written == 4096 &&
              info.biomes_written == 64,
          "header-only fills all snap_info fields");

    /* The full decoder must not accept a header with no body. */
    uint32_t *out_blocks = alloc_u32(blocks_count);
    uint32_t *out_biomes = alloc_u32(biomes_count);
    memset(out_blocks, 0xCC, blocks_count * sizeof *out_blocks);
    memset(out_biomes, 0xDD, biomes_count * sizeof *out_biomes);
    memset(&info, 0x5A, sizeof info);
    int32_t status = craftrustnx_snapshot_decode(bytes, SNAP_HEADER_SIZE,
        out_blocks, blocks_count, out_biomes, biomes_count, &info);
    CHECK(status != SNAP_OK, "full decode rejects header-only input");
    CHECK(info_is_zero(&info), "header-only full-decode zeroes info");
    CHECK(arrays_are(out_blocks, blocks_count, 0xCCCCCCCCu) &&
              arrays_are(out_biomes, biomes_count, 0xDDDDDDDDu),
          "header-only full-decode writes nothing");

    expect_header_error(bytes, 47, "header rejects length 47", 0);
    expect_header_error(bytes, 0, "header rejects length 0", 0);
    expect_header_error(NULL, 0, "header rejects null data", 0);

    memset(&info, 0x5A, sizeof info);
    CHECK(craftrustnx_snapshot_decode_header(bytes, 0, NULL) == SNAP_BAD_ARGUMENT,
          "header null info is BAD_ARGUMENT");
    memset(&info, 0x5A, sizeof info);
    CHECK(craftrustnx_snapshot_decode_header(NULL, SNAP_HEADER_SIZE, &info) ==
              SNAP_BAD_ARGUMENT,
          "header null data is BAD_ARGUMENT");
    CHECK(info_is_zero(&info), "null-data header zeroes info");
    CHECK(craftrustnx_snapshot_decode_header(bytes, (uint32_t)SNAP_MAX_FILE_SIZE + 1u,
              &info) == SNAP_BAD_ARGUMENT,
          "header over-limit length rejected before slicing");

    free(out_biomes);
    free(out_blocks);
    free(bytes);
    free(biomes);
    free(blocks);
}

static void expect_encode_error(const char *name, int32_t chunk_x, int32_t chunk_z,
                                int32_t min_y, uint32_t sections,
                                const uint32_t *blocks, uint32_t blocks_written,
                                const uint32_t *biomes, uint32_t biomes_written,
                                uint8_t *out, uint32_t capacity, uint32_t guard,
                                int32_t expected) {
    memset(out, 0xA5, guard);
    uint32_t written = 0xDEADBEEFu;
    int32_t status = craftrustnx_snapshot_encode(chunk_x, chunk_z, min_y, sections,
        blocks, blocks_written, biomes, biomes_written, out, capacity, &written);
    CHECK(status == expected, name);
    CHECK(written == 0u, "encode failure zeroes written");
    CHECK(bytes_are(out, guard, 0xA5), "encode failure leaves output untouched");
}

static void test_encode_rejects(void) {
    const uint32_t sections = 1, blocks_count = 4096, biomes_count = 64;
    const uint32_t length = file_length(sections);
    uint32_t *blocks = alloc_u32(blocks_count);
    uint32_t *biomes = alloc_u32(biomes_count);
    fill_pattern(sections, blocks, biomes);
    uint8_t *out = alloc_u8(length + 16);
    const uint32_t guard = length + 16;

    expect_encode_error("encode rejects sections 0", -2, -3, -64, 0,
        blocks, 0, biomes, 0, out, length, guard, SNAP_BAD_ARGUMENT);
    expect_encode_error("encode rejects sections 257", -2, -3, -64, 257,
        blocks, 257u * 4096u, biomes, 257u * 64u, out, length, guard,
        SNAP_BAD_ARGUMENT);
    expect_encode_error("encode rejects min_y -63", -2, -3, -63, sections,
        blocks, blocks_count, biomes, biomes_count, out, length, guard,
        SNAP_BAD_ARGUMENT);
    expect_encode_error("encode rejects min_y 15", -2, -3, 15, sections,
        blocks, blocks_count, biomes, biomes_count, out, length, guard,
        SNAP_BAD_ARGUMENT);
    expect_encode_error("encode rejects min_y + sections*16 i32 overflow",
        -2, -3, 0x7FFFFFF0, sections, blocks, blocks_count, biomes, biomes_count,
        out, length, guard, SNAP_BAD_ARGUMENT);
    expect_encode_error("encode rejects blocks_written short", -2, -3, -64,
        sections, blocks, blocks_count - 1, biomes, biomes_count, out, length,
        guard, SNAP_BAD_ARGUMENT);
    expect_encode_error("encode rejects blocks_written long", -2, -3, -64,
        sections, blocks, blocks_count + 1, biomes, biomes_count, out, length,
        guard, SNAP_BAD_ARGUMENT);
    expect_encode_error("encode rejects blocks_written 0", -2, -3, -64,
        sections, blocks, 0, biomes, biomes_count, out, length, guard,
        SNAP_BAD_ARGUMENT);
    expect_encode_error("encode rejects biomes_written short", -2, -3, -64,
        sections, blocks, blocks_count, biomes, biomes_count - 1, out, length,
        guard, SNAP_BAD_ARGUMENT);
    expect_encode_error("encode rejects biomes_written long", -2, -3, -64,
        sections, blocks, blocks_count, biomes, biomes_count + 1, out, length,
        guard, SNAP_BAD_ARGUMENT);
    expect_encode_error("encode rejects biomes_written 0", -2, -3, -64,
        sections, blocks, blocks_count, biomes, 0, out, length, guard,
        SNAP_BAD_ARGUMENT);
    expect_encode_error("encode rejects short output", -2, -3, -64, sections,
        blocks, blocks_count, biomes, biomes_count, out, length - 1, guard,
        SNAP_OUTPUT_SMALL);
    expect_encode_error("encode rejects capacity 0", -2, -3, -64, sections,
        blocks, blocks_count, biomes, biomes_count, out, 0, guard,
        SNAP_BAD_ARGUMENT);
    expect_encode_error("encode rejects capacity above max", -2, -3, -64,
        sections, blocks, blocks_count, biomes, biomes_count, out,
        (uint32_t)SNAP_MAX_FILE_SIZE + 1u, guard, SNAP_BAD_ARGUMENT);
    expect_encode_error("encode rejects null blocks", -2, -3, -64, sections,
        NULL, blocks_count, biomes, biomes_count, out, length, guard,
        SNAP_BAD_ARGUMENT);
    expect_encode_error("encode rejects null biomes", -2, -3, -64, sections,
        blocks, blocks_count, NULL, biomes_count, out, length, guard,
        SNAP_BAD_ARGUMENT);

    uint32_t written = 0xDEADBEEFu;
    CHECK(craftrustnx_snapshot_encode(-2, -3, -64, sections, blocks, blocks_count,
              biomes, biomes_count, NULL, length, &written) == SNAP_BAD_ARGUMENT,
          "encode rejects null out");
    CHECK(written == 0u, "null out zeroes written");
    memset(out, 0xA5, guard);
    CHECK(craftrustnx_snapshot_encode(-2, -3, -64, sections, blocks, blocks_count,
              biomes, biomes_count, out, length, NULL) == SNAP_BAD_ARGUMENT,
          "encode rejects null written");
    CHECK(bytes_are(out, guard, 0xA5), "null written leaves output");

    /* Valid small negative min_y is accepted. */
    memset(out, 0, guard);
    written = 0;
    CHECK(craftrustnx_snapshot_encode(-2, -3, -16, sections, blocks, blocks_count,
              biomes, biomes_count, out, length, &written) == SNAP_OK,
          "encode accepts min_y -16");
    CHECK(written == length, "min_y -16 writes full file");

    free(out);
    free(biomes);
    free(blocks);
}

static void expect_decode_error(const uint8_t *data, uint32_t length,
                                uint32_t *blocks, uint32_t block_capacity,
                                uint32_t *biomes, uint32_t biome_capacity,
                                const char *name, int want_bad_frame) {
    struct snap_info info;
    memset(&info, 0x5A, sizeof info);
    memset(blocks, 0xCC, (size_t)block_capacity * sizeof *blocks);
    memset(biomes, 0xDD, (size_t)biome_capacity * sizeof *biomes);
    int32_t status = craftrustnx_snapshot_decode(data, length, blocks,
        block_capacity, biomes, biome_capacity, &info);
    if (want_bad_frame) CHECK(status == SNAP_BAD_FRAME, name);
    else CHECK(status != SNAP_OK, name);
    CHECK(info_is_zero(&info), "decode failure zeroes info");
    CHECK(arrays_are(blocks, block_capacity, 0xCCCCCCCCu),
          "decode failure leaves blocks");
    CHECK(arrays_are(biomes, biome_capacity, 0xDDDDDDDDu),
          "decode failure leaves biomes");
}

static void test_decode_rejects(void) {
    const uint32_t sections = 1, blocks_count = 4096, biomes_count = 64;
    const uint32_t length = file_length(sections);
    uint32_t *input_blocks = alloc_u32(blocks_count);
    uint32_t *input_biomes = alloc_u32(biomes_count);
    fill_pattern(sections, input_blocks, input_biomes);
    uint8_t *bytes = alloc_u8(length);
    build_bytes(bytes, -2, -3, -64, sections, input_blocks, input_biomes);

    uint32_t *blocks = alloc_u32(blocks_count + 8);
    uint32_t *biomes = alloc_u32(biomes_count + 8);
    struct snap_info info;

    memset(blocks, 0xCC, (blocks_count + 8) * sizeof *blocks);
    memset(biomes, 0xDD, (biomes_count + 8) * sizeof *biomes);
    memset(&info, 0x5A, sizeof info);
    CHECK(craftrustnx_snapshot_decode(bytes, length, blocks, blocks_count - 1,
              biomes, biomes_count, &info) == SNAP_OUTPUT_SMALL,
          "decode rejects short block capacity");
    CHECK(info_is_zero(&info) && arrays_are(blocks, blocks_count - 1, 0xCCCCCCCCu) &&
              arrays_are(biomes, biomes_count, 0xDDDDDDDDu),
          "short block capacity writes nothing");

    memset(blocks, 0xCC, (blocks_count + 8) * sizeof *blocks);
    memset(biomes, 0xDD, (biomes_count + 8) * sizeof *biomes);
    memset(&info, 0x5A, sizeof info);
    CHECK(craftrustnx_snapshot_decode(bytes, length, blocks, blocks_count,
              biomes, biomes_count - 1, &info) == SNAP_OUTPUT_SMALL,
          "decode rejects short biome capacity");
    CHECK(info_is_zero(&info) && arrays_are(blocks, blocks_count, 0xCCCCCCCCu) &&
              arrays_are(biomes, biomes_count - 1, 0xDDDDDDDDu),
          "short biome capacity writes no blocks either");

    expect_decode_error(bytes, 0, blocks, blocks_count, biomes, biomes_count,
                        "decode rejects length 0", 0);
    expect_decode_error(bytes, 1, blocks, blocks_count, biomes, biomes_count,
                        "decode rejects length 1", 0);
    expect_decode_error(bytes, 47, blocks, blocks_count, biomes, biomes_count,
                        "decode rejects length 47", 0);
    expect_decode_error(bytes, 48, blocks, blocks_count, biomes, biomes_count,
                        "decode rejects header without body", 0);
    expect_decode_error(bytes, 49, blocks, blocks_count, biomes, biomes_count,
                        "decode rejects one body byte", 0);
    expect_decode_error(bytes, length - 1, blocks, blocks_count, biomes,
                        biomes_count, "decode rejects truncated body", 0);

    uint8_t *trailing = alloc_u8(length + 1);
    memcpy(trailing, bytes, length);
    trailing[length] = 0x00;
    expect_decode_error(trailing, length + 1, blocks, blocks_count, biomes,
                        biomes_count, "decode rejects trailing byte", 0);

    memset(blocks, 0xCC, (blocks_count + 8) * sizeof *blocks);
    memset(biomes, 0xDD, (biomes_count + 8) * sizeof *biomes);
    memset(&info, 0x5A, sizeof info);
    CHECK(craftrustnx_snapshot_decode(bytes, (uint32_t)SNAP_MAX_FILE_SIZE + 1u,
              blocks, blocks_count, biomes, biomes_count, &info) == SNAP_BAD_ARGUMENT,
          "decode rejects over-limit length before slicing");
    CHECK(info_is_zero(&info) && arrays_are(blocks, blocks_count, 0xCCCCCCCCu) &&
              arrays_are(biomes, biomes_count, 0xDDDDDDDDu),
          "over-limit length writes nothing");

    memset(&info, 0x5A, sizeof info);
    CHECK(craftrustnx_snapshot_decode(bytes, UINT32_MAX, blocks, blocks_count,
              biomes, biomes_count, &info) == SNAP_BAD_ARGUMENT,
          "decode rejects UINT32_MAX length");
    CHECK(info_is_zero(&info), "UINT32_MAX length zeroes info");

    memset(blocks, 0xCC, (blocks_count + 8) * sizeof *blocks);
    memset(biomes, 0xDD, (biomes_count + 8) * sizeof *biomes);
    memset(&info, 0x5A, sizeof info);
    CHECK(craftrustnx_snapshot_decode(NULL, 0, blocks, blocks_count, biomes,
              biomes_count, &info) == SNAP_BAD_ARGUMENT,
          "decode rejects null data");
    CHECK(info_is_zero(&info) && arrays_are(blocks, blocks_count, 0xCCCCCCCCu) &&
              arrays_are(biomes, biomes_count, 0xDDDDDDDDu),
          "null data writes nothing");
    memset(&info, 0x5A, sizeof info);
    CHECK(craftrustnx_snapshot_decode(bytes, length, NULL, blocks_count, biomes,
              biomes_count, &info) == SNAP_BAD_ARGUMENT,
          "decode rejects null blocks");
    CHECK(info_is_zero(&info) && arrays_are(biomes, biomes_count, 0xDDDDDDDDu),
          "null blocks leaves biomes");
    memset(&info, 0x5A, sizeof info);
    CHECK(craftrustnx_snapshot_decode(bytes, length, blocks, blocks_count, NULL,
              biomes_count, &info) == SNAP_BAD_ARGUMENT,
          "decode rejects null biomes");
    CHECK(info_is_zero(&info) && arrays_are(blocks, blocks_count, 0xCCCCCCCCu),
          "null biomes leaves blocks");
    memset(blocks, 0xCC, (blocks_count + 8) * sizeof *blocks);
    memset(biomes, 0xDD, (biomes_count + 8) * sizeof *biomes);
    CHECK(craftrustnx_snapshot_decode(bytes, length, blocks, blocks_count, biomes,
              biomes_count, NULL) == SNAP_BAD_ARGUMENT,
          "decode rejects null info");
    CHECK(arrays_are(blocks, blocks_count, 0xCCCCCCCCu) &&
              arrays_are(biomes, biomes_count, 0xDDDDDDDDu),
          "null info writes nothing");

    free(trailing);
    free(biomes);
    free(blocks);
    free(bytes);
    free(input_biomes);
    free(input_blocks);
}

static void corrupt_word(const uint8_t *valid, uint32_t length, uint32_t offset,
                         uint32_t value, const char *name, int want_bad_frame,
                         uint32_t *blocks, uint32_t *biomes) {
    uint8_t *bad = alloc_u8(length);
    memcpy(bad, valid, length);
    put_le32(bad + offset, value);
    expect_header_error(bad, SNAP_HEADER_SIZE, name, want_bad_frame);
    expect_decode_error(bad, length, blocks, 4096, biomes, 64, name, want_bad_frame);
    free(bad);
}

static void test_corrupt_header(void) {
    const uint32_t sections = 1, blocks_count = 4096, biomes_count = 64;
    const uint32_t length = file_length(sections);
    uint32_t *input_blocks = alloc_u32(blocks_count);
    uint32_t *input_biomes = alloc_u32(biomes_count);
    fill_pattern(sections, input_blocks, input_biomes);
    uint8_t *bytes = alloc_u8(length);
    build_bytes(bytes, -2, -3, -64, sections, input_blocks, input_biomes);
    uint32_t *blocks = alloc_u32(blocks_count);
    uint32_t *biomes = alloc_u32(biomes_count);

    uint8_t *bad = alloc_u8(length);
    memcpy(bad, bytes, length);
    bad[0] = 0x00;
    expect_header_error(bad, SNAP_HEADER_SIZE, "corrupt magic first byte", 1);
    expect_decode_error(bad, length, blocks, blocks_count, biomes, biomes_count,
                        "decode corrupt magic first byte", 1);
    memcpy(bad, bytes, length);
    bad[7] = 0x00;
    expect_header_error(bad, SNAP_HEADER_SIZE, "corrupt magic last byte", 1);
    expect_decode_error(bad, length, blocks, blocks_count, biomes, biomes_count,
                        "decode corrupt magic last byte", 1);
    free(bad);

    corrupt_word(bytes, length, 8, 778u, "corrupt protocol 778", 1, blocks, biomes);
    corrupt_word(bytes, length, 8, 0u, "corrupt protocol 0", 1, blocks, biomes);
    corrupt_word(bytes, length, 12, 2u, "corrupt format 2", 1, blocks, biomes);
    corrupt_word(bytes, length, 40, 1u, "corrupt reserved byte 40", 1, blocks, biomes);
    corrupt_word(bytes, length, 44, 0x01000000u, "corrupt reserved high byte 44", 1,
                 blocks, biomes);
    corrupt_word(bytes, length, 32, 4095u, "corrupt blocks_written 4095", 1, blocks,
                 biomes);
    corrupt_word(bytes, length, 32, 4097u, "corrupt blocks_written 4097", 1, blocks,
                 biomes);
    corrupt_word(bytes, length, 36, 63u, "corrupt biomes_written 63", 1, blocks,
                 biomes);
    corrupt_word(bytes, length, 36, 65u, "corrupt biomes_written 65", 1, blocks,
                 biomes);

    /* Bounds/alignment/overflow codes are not pinned by the frozen header, so
     * require a negative status, zero info and no writes rather than a code. */
    corrupt_word(bytes, length, 28, 0u, "corrupt sections 0", 0, blocks, biomes);
    corrupt_word(bytes, length, 28, 257u, "corrupt sections 257", 0, blocks, biomes);
    corrupt_word(bytes, length, 24, (uint32_t)-63, "corrupt min_y -63", 0, blocks,
                 biomes);
    corrupt_word(bytes, length, 24, 0x7FFFFFF0u, "corrupt min_y i32 overflow", 0,
                 blocks, biomes);

    free(biomes);
    free(blocks);
    free(bytes);
    free(input_biomes);
    free(input_blocks);
}

static void test_max_sections(void) {
    const uint32_t sections = (uint32_t)SNAP_MAX_SECTIONS;
    const uint32_t blocks_count = (uint32_t)SNAP_MAX_BLOCKS;
    const uint32_t biomes_count = (uint32_t)SNAP_MAX_BIOMES;
    const uint32_t length = file_length(sections);
    CHECK(length == (uint32_t)SNAP_MAX_FILE_SIZE, "256 sections is max file size");

    uint32_t *blocks = alloc_u32(blocks_count);
    uint32_t *biomes = alloc_u32(biomes_count);
    fill_pattern(sections, blocks, biomes);
    uint8_t *expected = alloc_u8(length);
    CHECK(build_bytes(expected, -1, 1, -2032, sections, blocks, biomes) == length,
          "max fixture length");

    uint8_t *out = alloc_u8(length + 16);
    memset(out, 0x3C, length + 16);
    uint32_t written = 0;
    CHECK(craftrustnx_snapshot_encode(-1, 1, -2032, sections, blocks, blocks_count,
              biomes, biomes_count, out, length, &written) == SNAP_OK,
          "max encode SNAP_OK");
    CHECK(written == length && memcmp(out, expected, length) == 0,
          "max encode exact bytes");
    CHECK(bytes_are(out + length, 16, 0x3C), "max encode tail untouched");

    /* At 256 sections the documented capacity is exactly SNAP_MAX_*; there is
     * no in-capacity tail to preserve beyond the column. */
    uint32_t *out_blocks = alloc_u32(blocks_count);
    uint32_t *out_biomes = alloc_u32(biomes_count);
    memset(out_blocks, 0xCC, blocks_count * sizeof *out_blocks);
    memset(out_biomes, 0xDD, biomes_count * sizeof *out_biomes);
    struct snap_info info;
    memset(&info, 0x5A, sizeof info);
    CHECK(craftrustnx_snapshot_decode(expected, length, out_blocks, blocks_count,
              out_biomes, biomes_count, &info) == SNAP_OK,
          "max decode SNAP_OK");
    CHECK(info.sections == sections && info.blocks_written == blocks_count &&
              info.biomes_written == biomes_count,
          "max decode exact counts");
    CHECK(same_u32(out_blocks, blocks, blocks_count) &&
              same_u32(out_biomes, biomes, biomes_count),
          "max decode all cells");

    memset(&info, 0x5A, sizeof info);
    CHECK(craftrustnx_snapshot_decode_header(expected, SNAP_HEADER_SIZE, &info) ==
              SNAP_OK,
          "max header-only decode accepts sections 256");
    CHECK(info.sections == 256u && info.blocks_written == 1048576u &&
              info.biomes_written == 16384u,
          "max header-only counts");

    free(out_biomes);
    free(out_blocks);
    free(out);
    free(expected);
    free(biomes);
    free(blocks);
}

int main(void) {
    test_limits_abi();
    test_encode_exact();
    test_decode_exact();
    test_header_only();
    test_encode_rejects();
    test_decode_rejects();
    test_corrupt_header();
    test_max_sections();
    if (failures) {
        fprintf(stderr, "%d snapshot host check(s) failed\n", failures);
        return 1;
    }
    puts("PASS snapshot host: ABI, exact LE bytes, ordering, ID sentinels, "
         "header-only, corruption, truncation/trailing, short output, null/over-limit, "
         "transactional no-write, 256-section max");
    return 0;
}
