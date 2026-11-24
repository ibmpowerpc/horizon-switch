/* File lifecycle only. Rust owns the bounded column format and validation. */
#ifdef CRAFTRUSTNX_COLUMN_EXPORT
#include "column_store.h"
#include "column_snapshot.h"
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

int lj_store_column(const char *base, const struct col_info *info,
    const uint32_t *blocks, const uint32_t *biomes,
    char *path, size_t path_capacity, int *codec_error) {
    if (path && path_capacity) path[0] = 0;
    if (codec_error) *codec_error = 0;
    if (!base || !info || !blocks || !biomes || !path || !codec_error ||
        !path_capacity || strlen(base) > 256 ||
        path_capacity < strlen(base) + 48 || !info->sections || info->sections > 256 ||
        info->blocks_written != info->sections * 4096 ||
        info->biomes_written != info->sections * 64) return -1;
    const uint32_t capacity = 48 + info->sections * (4096 + 64) * 4;
    uint8_t *encoded = malloc(capacity);
    if (!encoded) return -3;
    uint32_t written = 0;
    *codec_error = craftrustnx_snapshot_encode(info->chunk_x, info->chunk_z,
        info->min_y, info->sections, blocks, info->blocks_written,
        biomes, info->biomes_written, encoded, capacity, &written);
    if (*codec_error || written != capacity) { free(encoded); return -2; }
    struct stat st;
    if ((mkdir(base, 0777) && errno != EEXIST) || stat(base, &st) || !S_ISDIR(st.st_mode)) {
        free(encoded); return -3;
    }
    char directory[384], temporary[432], final[432];
    int owned = 0;
    for (unsigned n = 0; n < 1024; ++n) {
        snprintf(directory, sizeof(directory), "%s/capture-%04u", base, n);
        if (mkdir(directory, 0777) == 0) { owned = 1; break; }
        if (errno != EEXIST) break;
    }
    if (!owned) { free(encoded); return -3; }
    snprintf(temporary, sizeof(temporary), "%s/column.tmp", directory);
    snprintf(final, sizeof(final), "%s/column.ctcol", directory);
    int result = -3;
    FILE *file = fopen(temporary, "wbx");
    if (!file) goto cleanup;
    const size_t sent = fwrite(encoded, 1, written, file);
    const int flushed = fflush(file);
    const int closed = fclose(file);
    if (sent != written || flushed || closed || rename(temporary, final)) goto cleanup;
    file = fopen(final, "rb");
    if (!file) goto cleanup;
    uint8_t buffer[8192];
    uint32_t offset = 0;
    int matches = 1;
    while (offset < written) {
        const size_t want = written - offset < sizeof(buffer) ? written - offset : sizeof(buffer);
        const size_t got = fread(buffer, 1, want, file);
        if (got != want || memcmp(buffer, encoded + offset, want)) { matches = 0; break; }
        offset += (uint32_t)want;
    }
    const int trailing = fgetc(file);
    const int read_error = ferror(file);
    const int read_closed = fclose(file);
    if (!matches || trailing != EOF || read_error || read_closed) { result = -4; goto cleanup; }
    memcpy(path, final, strlen(final) + 1);
    free(encoded); return 0;
cleanup:
    remove(temporary);
    remove(final);
    rmdir(directory);
    free(encoded);
    return result;
}
#endif
