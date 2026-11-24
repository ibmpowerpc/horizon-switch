#define _POSIX_C_SOURCE 200809L
#define _DARWIN_C_SOURCE
#include "column_store.h"
#include "column_snapshot.h"
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

/* Real files and Rust decoder: persistence and no-overwrite behavior. */
static void read_and_check(const char *path, const struct col_info *column,
    const uint32_t *blocks, const uint32_t *biomes) {
    uint8_t bytes[48 + (4096 + 64) * 4];
    uint32_t decoded_blocks[4096], decoded_biomes[64];
    struct snap_info decoded;
    FILE *file = fopen(path, "rb");
    assert(file);
    assert(fread(bytes, 1, sizeof(bytes), file) == sizeof(bytes));
    assert(fgetc(file) == EOF && !ferror(file));
    assert(fclose(file) == 0);
    assert(craftrustnx_snapshot_decode(bytes, sizeof(bytes), decoded_blocks, 4096,
        decoded_biomes, 64, &decoded) == SNAP_OK);
    assert(decoded.chunk_x == column->chunk_x && decoded.chunk_z == column->chunk_z);
    assert(decoded.min_y == column->min_y && decoded.sections == 1);
    assert(decoded.blocks_written == 4096 && decoded.biomes_written == 64);
    assert(memcmp(decoded_blocks, blocks, sizeof(decoded_blocks)) == 0);
    assert(memcmp(decoded_biomes, biomes, sizeof(decoded_biomes)) == 0);
}

int main(void) {
    char scratch[] = "/tmp/craftrustnx-column-store-XXXXXX";
    assert(mkdtemp(scratch));
    char base[256], first[512], second[512], failed[512], capture[512];
    snprintf(base, sizeof(base), "%s/captures", scratch);
    uint32_t blocks[4096], biomes[64];
    for (size_t i = 0; i < 4096; ++i) blocks[i] = (uint32_t)i * 0x01020304u;
    for (size_t i = 0; i < 64; ++i) biomes[i] = (uint32_t)i + 256;
    const struct col_info info = {-7, 19, -64, 1, 4096, 64};
    int codec_error = 999;
    assert(lj_store_column(base, &info, blocks, biomes, first, sizeof(first), &codec_error) == 0);
    assert(codec_error == 0);
    assert(strstr(first, "/capture-0000/column.ctcol"));
    read_and_check(first, &info, blocks, biomes);
    assert(lj_store_column(base, &info, blocks, biomes, second, sizeof(second), &codec_error) == 0);
    assert(strstr(second, "/capture-0001/column.ctcol") && strcmp(first, second));
    read_and_check(first, &info, blocks, biomes);
    read_and_check(second, &info, blocks, biomes);

    struct col_info bad = info;
    --bad.blocks_written;
    strcpy(failed, "must be cleared");
    assert(lj_store_column(base, &bad, blocks, biomes, failed, sizeof(failed), &codec_error) == -1);
    assert(!failed[0] && codec_error == 0);
    bad = info;
    bad.min_y = -63;
    assert(lj_store_column(base, &bad, blocks, biomes, failed, sizeof(failed), &codec_error) == -2);
    assert(!failed[0] && codec_error != 0);
    snprintf(capture, sizeof(capture), "%s/capture-0002", base);
    struct stat st;
    assert(stat(capture, &st) == -1);
    /* A regular file cannot become the capture directory; preserve its bytes. */
    assert(lj_store_column(first, &info, blocks, biomes, failed, sizeof(failed), &codec_error) == -3);
    assert(!failed[0]);
    read_and_check(first, &info, blocks, biomes);
    for (unsigned i = 0; i < 2; ++i) {
        assert(remove(i == 0 ? first : second) == 0);
        snprintf(capture, sizeof(capture), "%s/capture-%04u", base, i);
        assert(rmdir(capture) == 0);
    }
    assert(rmdir(base) == 0 && rmdir(scratch) == 0);
    puts("column store: save/readback, unique captures, validation and file-error PASS");
    return 0;
}
