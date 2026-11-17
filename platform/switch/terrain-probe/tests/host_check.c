/* Real C->Rust ABI check against a private exported mesh; no SDK/GPU mocks. */
#include "terrain_app.h"
#include <assert.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

_Static_assert(sizeof(struct tp_vertex) == 16, "vertex ABI");
_Static_assert(sizeof(struct tp_mesh_info) == 28, "mesh ABI");
_Static_assert(sizeof(struct tp_app_state) == 32, "state ABI");
_Static_assert(sizeof(struct tp_app_frame) == 96, "frame ABI");
_Static_assert(offsetof(struct tp_app_state, auto_orbit) == 20, "orbit ABI");

int main(int argc, char **argv) {
    assert(argc == 2);
    FILE *f = fopen(argv[1], "rb"); assert(f);
    assert(fseek(f, 0, SEEK_END) == 0);
    const long size = ftell(f); assert(size > 48 && size < 8*1024*1024);
    rewind(f);
    unsigned char *bytes = malloc((size_t)size); assert(bytes);
    assert(fread(bytes, 1, (size_t)size, f) == (size_t)size);
    assert(fclose(f) == 0);
    const size_t capacity = ((size_t)size - 48) / sizeof(struct tp_vertex);
    struct tp_vertex *vertices = malloc(capacity*sizeof(*vertices)); assert(vertices);
    struct tp_mesh_info info;
    assert(craftrustnx_mesh_decode(bytes, (size_t)size, vertices, capacity, &info) == 0);
    assert(info.vertex_count == capacity && capacity % 3 == 0);
    /* Both test host and target are little-endian; every decoded field agrees. */
    const unsigned int endian = 1;
    assert(*(const unsigned char *)&endian == 1);
    assert(memcmp(vertices, bytes+48, capacity*sizeof(*vertices)) == 0);

    struct tp_app_state state = {0};
    struct tp_app_frame frame;
    assert(craftrustnx_terrain_update(&state, &info, 0, 0, 0, 0, &frame) == 0);
    assert(state.frames == 1 && state.auto_orbit == 1);
    for (size_t i=0; i<capacity; ++i) {
        const float *v = vertices[i].position;
        float clip[4];
        for (int row=0; row<4; ++row)
            clip[row] = frame.mvp[row]*v[0] + frame.mvp[4+row]*v[1]
                + frame.mvp[8+row]*v[2] + frame.mvp[12+row];
        assert(isfinite(clip[3]) && clip[3] > 0);
        assert(fabsf(clip[0]) <= clip[3]+0.0001f);
        assert(fabsf(clip[1]) <= clip[3]+0.0001f);
        assert(clip[2] >= 0 && clip[2] <= clip[3]+0.0001f);
    }
    assert(craftrustnx_terrain_update(&state, &info, 1000000, TP_ORBIT, 0, 0, &frame) == 0);
    assert(state.auto_orbit == 0);
    assert(craftrustnx_terrain_update(&state, &info, 2000000, TP_EXIT, 0, 0, &frame) == 0);
    assert(frame.should_exit == 1);

    const unsigned int nan_bits=0x7fc00000u;
    memcpy(bytes + 48 + (capacity-1)*16, &nan_bits, 4);
    memset(vertices, 0xcd, capacity*sizeof(*vertices));
    assert(craftrustnx_mesh_decode(bytes, (size_t)size, vertices, capacity, &info) == -3);
    const unsigned char *raw = (const unsigned char *)vertices;
    for (size_t i=0; i<capacity*sizeof(*vertices); ++i) assert(raw[i] == 0xcd);
    const struct tp_mesh_info zero={0}; assert(memcmp(&info,&zero,sizeof(info))==0);
    printf("PASS: %zu vertices, exact C/Rust fields, all in initial camera frustum, controls, transactional corrupt-mesh refusal\n",capacity);
    free(vertices); free(bytes); return 0;
}
