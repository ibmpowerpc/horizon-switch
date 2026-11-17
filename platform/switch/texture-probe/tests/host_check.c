/* Host C ABI checks for the textured scene decoder and quad sort.
 * ASan/UBSan instrument this translation unit only. The Rust staticlib is
 * not instrumented. This is not a hardware texture pass.
 * Expected bytes are written by this file. The decoder is not used as an oracle.
 */
#include "texture_app.h"

#include <math.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

_Static_assert(sizeof(struct tx_vertex) == 24, "vertex");
_Static_assert(offsetof(struct tx_vertex, uv) == 12, "uv");
_Static_assert(offsetof(struct tx_vertex, rgba) == 20, "rgba");
_Static_assert(sizeof(struct tx_draw) == 16, "draw");
_Static_assert(sizeof(struct tp_mesh_info) == 28, "mesh");
_Static_assert(sizeof(struct tx_scene_info) == 48, "info");
_Static_assert(offsetof(struct tx_scene_info, draw_count) == 28, "draw_count");
_Static_assert(offsetof(struct tx_scene_info, atlas_offset) == 40, "atlas_offset");
_Static_assert(offsetof(struct tx_scene_info, atlas_bytes) == 44, "atlas_bytes");
_Static_assert(TX_MAX_VERTICES == 262140u, "vert limit");
_Static_assert(TX_MAX_DRAWS == 256u, "draw limit");
_Static_assert(TX_MAX_DIMENSION == 1024u, "dim limit");
_Static_assert(TX_OPAQUE == 0 && TX_CUTOUT == 1 && TX_TRANSLUCENT == 2, "modes");

enum { HEADER = 64, VSTRIDE = 24, DSTRIDE = 16, MAX_FILE = 16 * 1024 * 1024 };

static unsigned checks, fails;
static char scene_note[160] = "scene not requested";

static void bad(const char *fmt, ...) {
    va_list ap;
    fails++;
    fprintf(stderr, "FAIL ");
    va_start(ap, fmt);
    vfprintf(stderr, fmt, ap);
    va_end(ap);
    fputc('\n', stderr);
}

#define CHECK(cond, ...)                                                       \
    do {                                                                       \
        checks++;                                                              \
        if (!(cond))                                                           \
            bad(__VA_ARGS__);                                                  \
    } while (0)

static void *xmalloc(size_t n) {
    void *p;
    if (n == 0)
        n = 1;
    p = malloc(n);
    if (!p) {
        fprintf(stderr, "out of memory (%zu)\n", n);
        exit(2);
    }
    return p;
}

static int host_le(void) {
    const uint32_t x = 1;
    return *(const unsigned char *)&x == 1;
}

static void wr_u32(unsigned char *p, uint32_t v) { memcpy(p, &v, 4); }

static void wr_f32(unsigned char *p, float v) { memcpy(p, &v, 4); }

static uint32_t rd_u32(const unsigned char *p) {
    uint32_t v;
    memcpy(&v, p, 4);
    return v;
}

static float rd_f32(const unsigned char *p) {
    float v;
    memcpy(&v, p, 4);
    return v;
}

static float bits_f(uint32_t u) {
    float f;
    memcpy(&f, &u, 4);
    return f;
}

static int fbits_eq(float a, float b) {
    uint32_t xa, xb;
    memcpy(&xa, &a, 4);
    memcpy(&xb, &b, 4);
    return xa == xb;
}

static int all_zero(const void *p, size_t n) {
    const unsigned char *b = p;
    size_t i;
    for (i = 0; i < n; i++)
        if (b[i] != 0)
            return 0;
    return 1;
}

static void vert_set(struct tx_vertex *v, float x, float y, float z, float u,
                     float vv, uint32_t rgba) {
    v->position[0] = x;
    v->position[1] = y;
    v->position[2] = z;
    v->uv[0] = u;
    v->uv[1] = vv;
    v->rgba = rgba;
}

static void wr_vert(unsigned char *p, const struct tx_vertex *v) {
    wr_f32(p + 0, v->position[0]);
    wr_f32(p + 4, v->position[1]);
    wr_f32(p + 8, v->position[2]);
    wr_f32(p + 12, v->uv[0]);
    wr_f32(p + 16, v->uv[1]);
    wr_u32(p + 20, v->rgba);
}

static void wr_draw(unsigned char *p, const struct tx_draw *d) {
    wr_u32(p + 0, d->first);
    wr_u32(p + 4, d->count);
    wr_u32(p + 8, d->mode);
    wr_u32(p + 12, d->reserved);
}

struct scene {
    unsigned char *bytes;
    size_t len;
    uint32_t vc, dc, width, height;
    float bounds[6];
};

static struct scene *scene_new(uint32_t vc, const struct tx_vertex *verts,
                               uint32_t dc, const struct tx_draw *draws,
                               uint32_t width, uint32_t height,
                               const float bounds[6], uint32_t protocol,
                               uint32_t format, uint32_t flags,
                               uint32_t reserved, int atlas_fill) {
    size_t vbytes, dbytes, pixels, abytes, total;
    struct scene *s;
    unsigned char *b;
    uint32_t i;
    if (vc != 0 && VSTRIDE > SIZE_MAX / vc)
        return NULL;
    vbytes = (size_t)vc * VSTRIDE;
    if (dc != 0 && DSTRIDE > SIZE_MAX / dc)
        return NULL;
    dbytes = (size_t)dc * DSTRIDE;
    if (width != 0 && (size_t)height > SIZE_MAX / width)
        return NULL;
    pixels = (size_t)width * (size_t)height;
    if (pixels != 0 && 4 > SIZE_MAX / pixels)
        return NULL;
    abytes = pixels * 4;
    total = HEADER;
    if (vbytes > SIZE_MAX - total || dbytes > SIZE_MAX - (total + vbytes) ||
        abytes > SIZE_MAX - (total + vbytes + dbytes))
        return NULL;
    total += vbytes + dbytes + abytes;
    if (total > 32u * 1024u * 1024u)
        return NULL;
    s = xmalloc(sizeof *s);
    s->bytes = xmalloc(total);
    s->len = total;
    s->vc = vc;
    s->dc = dc;
    s->width = width;
    s->height = height;
    memcpy(s->bounds, bounds, sizeof s->bounds);
    b = s->bytes;
    memset(b, 0, total);
    memcpy(b, "CTTEX001", 8);
    wr_u32(b + 8, protocol);
    wr_u32(b + 12, format);
    wr_u32(b + 16, vc);
    wr_u32(b + 20, dc);
    wr_u32(b + 24, width);
    wr_u32(b + 28, height);
    wr_u32(b + 32, flags);
    wr_u32(b + 36, reserved);
    for (i = 0; i < 6; i++)
        wr_f32(b + 40 + i * 4, bounds[i]);
    if (verts) {
        for (i = 0; i < vc; i++)
            wr_vert(b + HEADER + (size_t)i * VSTRIDE, &verts[i]);
    }
    if (draws) {
        for (i = 0; i < dc; i++)
            wr_draw(b + HEADER + vbytes + (size_t)i * DSTRIDE, &draws[i]);
    }
    if (abytes)
        memset(b + HEADER + vbytes + dbytes, atlas_fill, abytes);
    return s;
}

static void scene_free(struct scene *s) {
    if (!s)
        return;
    free(s->bytes);
    free(s);
}

static void fill_inside(struct tx_vertex *v, uint32_t n) {
    uint32_t i;
    for (i = 0; i < n; i++)
        vert_set(&v[i], (i & 1) ? 1.f : 0.f, (i % 3) ? 1.f : 0.f, 0.f, 0.f,
                 (i & 2) ? 1.f : 0.f, 0x100u + i);
}

static void expect_ok(const char *name, const struct scene *s) {
    size_t phys_v = (size_t)s->vc + 1;
    size_t phys_d = (size_t)s->dc + 1;
    size_t off, abytes;
    struct tx_vertex *verts = xmalloc(phys_v * sizeof *verts);
    struct tx_draw *draws = xmalloc(phys_d * sizeof *draws);
    struct tx_vertex guard_v;
    struct tx_draw guard_d;
    struct tx_scene_info info;
    int32_t rc;
    unsigned i;
    memset(verts, 0xA5, phys_v * sizeof *verts);
    memset(draws, 0xA5, phys_d * sizeof *draws);
    guard_v = verts[s->vc];
    guard_d = draws[s->dc];
    memset(&info, 0x3C, sizeof info);
    rc = craftrustnx_texture_decode(s->bytes, s->len, verts, s->vc, draws, s->dc,
                                   &info);
    CHECK(rc == 0, "%s rc %ld", name, (long)rc);
    if (rc == 0) {
        off = HEADER + (size_t)s->vc * VSTRIDE + (size_t)s->dc * DSTRIDE;
        abytes = (size_t)s->width * (size_t)s->height * 4u;
        CHECK(memcmp(verts, s->bytes + HEADER, (size_t)s->vc * VSTRIDE) == 0,
              "%s vertex bytes", name);
        CHECK(memcmp(draws, s->bytes + HEADER + (size_t)s->vc * VSTRIDE,
                     (size_t)s->dc * DSTRIDE) == 0,
              "%s draw bytes", name);
        CHECK(info.mesh.vertex_count == s->vc, "%s vertex_count", name);
        CHECK(info.draw_count == s->dc, "%s draw_count", name);
        CHECK(info.width == s->width && info.height == s->height, "%s dimensions",
              name);
        CHECK(info.atlas_offset == off, "%s atlas_offset %u want %zu", name,
              info.atlas_offset, off);
        CHECK(info.atlas_bytes == abytes, "%s atlas_bytes %u want %zu", name,
              info.atlas_bytes, abytes);
        CHECK(s->len == off + abytes, "%s fixture length", name);
        for (i = 0; i < 3; i++) {
            CHECK(fbits_eq(info.mesh.min[i], s->bounds[i]), "%s min %u", name, i);
            CHECK(fbits_eq(info.mesh.max[i], s->bounds[3 + i]), "%s max %u", name,
                  i);
        }
        CHECK(memcmp(&verts[s->vc], &guard_v, sizeof guard_v) == 0,
              "%s vertex overrun", name);
        CHECK(memcmp(&draws[s->dc], &guard_d, sizeof guard_d) == 0, "%s draw overrun",
              name);
    }
    free(verts);
    free(draws);
}

static void expect_fail(const char *name, const unsigned char *data, size_t len,
                        size_t cap_v, size_t cap_d, int32_t want) {
    uint32_t claim_v = 8, claim_d = 4;
    size_t phys_v, phys_d, vb, db;
    struct tx_vertex *verts, *vsnap;
    struct tx_draw *draws, *dsnap;
    struct tx_scene_info info;
    int32_t rc;
    if (data && len >= 36) {
        claim_v = rd_u32(data + 16);
        claim_d = rd_u32(data + 20);
    }
    phys_v = cap_v + 1;
    phys_d = cap_d + 1;
    if (claim_v < 1000000u && (size_t)claim_v + 1 > phys_v)
        phys_v = (size_t)claim_v + 1;
    if (claim_d < 10000u && (size_t)claim_d + 1 > phys_d)
        phys_d = (size_t)claim_d + 1;
    verts = xmalloc(phys_v * sizeof *verts);
    draws = xmalloc(phys_d * sizeof *draws);
    memset(verts, 0xA5, phys_v * sizeof *verts);
    memset(draws, 0xA5, phys_d * sizeof *draws);
    vb = phys_v * sizeof *verts;
    db = phys_d * sizeof *draws;
    vsnap = xmalloc(vb);
    dsnap = xmalloc(db);
    memcpy(vsnap, verts, vb);
    memcpy(dsnap, draws, db);
    memset(&info, 0x3C, sizeof info);
    rc = craftrustnx_texture_decode(data, len, verts, cap_v, draws, cap_d, &info);
    CHECK(rc == want, "%s rc %ld want %ld", name, (long)rc, (long)want);
    CHECK(memcmp(verts, vsnap, vb) == 0, "%s vertices mutated", name);
    CHECK(memcmp(draws, dsnap, db) == 0, "%s draws mutated", name);
    CHECK(all_zero(&info, sizeof info), "%s info not zero", name);
    free(vsnap);
    free(dsnap);
    free(verts);
    free(draws);
}

static struct scene *canonical(void) {
    struct tx_vertex v[12];
    struct tx_draw d[3];
    const float bounds[6] = {0, 0, 0, 1, 1, 1};
    vert_set(&v[0], 0, 0, 0, 0, 0, 0x000000FFu);
    vert_set(&v[1], 1, 0, 0, 1, 0, 0x0000FF00u);
    vert_set(&v[2], 0, 1, 0, 0, 1, 0x00FF0000u);
    vert_set(&v[3], 0, 0, 1, 0, 0, 0x010000FFu);
    vert_set(&v[4], 1, 0, 1, 1, 0, 0x0100FF00u);
    vert_set(&v[5], 1, 1, 1, 1, 1, 0x01FF0000u);
    vert_set(&v[6], 0, 0, 0.5f, 0, 0, 0x80FFFFFFu);
    vert_set(&v[7], 1, 0, 0.5f, 1, 0, 0x80FFFFFEu);
    vert_set(&v[8], 1, 1, 0.5f, 1, 1, 0x80FFFFFDu);
    vert_set(&v[9], 1, 1, 0.5f, 1, 1, 0x80FFFFFCu);
    vert_set(&v[10], 0, 1, 0.5f, 0, 1, 0x80FFFFFBu);
    vert_set(&v[11], 0, 0, 0.5f, 0, 0, 0x80FFFFFAu);
    d[0] = (struct tx_draw){0, 3, TX_OPAQUE, 0};
    d[1] = (struct tx_draw){3, 3, TX_CUTOUT, 0};
    d[2] = (struct tx_draw){6, 6, TX_TRANSLUCENT, 0};
    return scene_new(12, v, 3, d, 2, 2, bounds, 777, 1, 0, 0, 0x7E);
}

static void patch_fail(const char *name, const struct scene *s, size_t off,
                       const void *val, size_t n) {
    unsigned char *b = xmalloc(s->len);
    memcpy(b, s->bytes, s->len);
    memcpy(b + off, val, n);
    expect_fail(name, b, s->len, s->vc, s->dc, -3);
    free(b);
}

static void run_group_cases(void) {
    struct tx_vertex verts[16];
    const float bounds[6] = {0, 0, 0, 1, 1, 1};
    struct {
        const char *name;
        uint32_t vc, dc;
        struct tx_draw d[3];
        int32_t want;
    } cases[] = {
        {"skip cutout", 9, 2, {{0, 3, TX_OPAQUE, 0}, {3, 6, TX_TRANSLUCENT, 0}}, 0},
        {"only translucent", 6, 1, {{0, 6, TX_TRANSLUCENT, 0}}, 0},
        {"cutout triangles", 9, 1, {{0, 9, TX_CUTOUT, 0}}, 0},
        {"opaque triangles", 9, 1, {{0, 9, TX_OPAQUE, 0}}, 0},
        {"gap", 9, 2, {{0, 3, TX_OPAQUE, 0}, {6, 3, TX_CUTOUT, 0}}, -3},
        {"overlap", 6, 2, {{0, 3, TX_OPAQUE, 0}, {0, 3, TX_CUTOUT, 0}}, -3},
        {"same mode", 6, 2, {{0, 3, TX_OPAQUE, 0}, {3, 3, TX_OPAQUE, 0}}, -3},
        {"decreasing mode", 6, 2, {{0, 3, TX_CUTOUT, 0}, {3, 3, TX_OPAQUE, 0}}, -3},
        {"two translucent", 12, 2, {{0, 6, TX_TRANSLUCENT, 0}, {6, 6, TX_TRANSLUCENT, 0}}, -3},
        {"mode 3", 3, 1, {{0, 3, 3, 0}}, -3},
        {"reserved", 3, 1, {{0, 3, TX_OPAQUE, 1}}, -3},
        {"translucent tris", 3, 1, {{0, 3, TX_TRANSLUCENT, 0}}, -3},
        {"translucent 9", 9, 1, {{0, 9, TX_TRANSLUCENT, 0}}, -3},
        {"draw not multiple of 3", 6, 2, {{0, 4, TX_OPAQUE, 0}, {4, 2, TX_CUTOUT, 0}}, -3},
        {"empty draw", 3, 1, {{0, 0, TX_OPAQUE, 0}}, -3},
        {"draw not at zero", 6, 1, {{3, 3, TX_OPAQUE, 0}}, -3},
        {"tail uncovered", 6, 1, {{0, 3, TX_OPAQUE, 0}}, -3},
        {"range extends past", 3, 1, {{0, 6, TX_OPAQUE, 0}}, -3},
    };
    size_t i;
    fill_inside(verts, 16);
    for (i = 0; i < sizeof cases / sizeof cases[0]; i++) {
        struct scene *s = scene_new(cases[i].vc, verts, cases[i].dc, cases[i].d, 1,
                                    1, bounds, 777, 1, 0, 0, 0x11);
        if (!s) {
            CHECK(0, "%s builder", cases[i].name);
            continue;
        }
        if (cases[i].want == 0)
            expect_ok(cases[i].name, s);
        else
            expect_fail(cases[i].name, s->bytes, s->len, cases[i].vc, cases[i].dc,
                        cases[i].want);
        scene_free(s);
    }
}

static void run_limit_cases(const struct scene *good) {
    const float bounds[6] = {0, 0, 0, 1, 1, 1};
    const float edge[6] = {-1024.f, 0.f, 0.f, 1024.f, 1.f, 0.f};
    struct tx_vertex tri[3];
    struct tx_draw one = {0, 3, TX_OPAQUE, 0};
    struct scene *s;
    unsigned char *big;
    uint32_t n;
    vert_set(&tri[0], 0, 0, 0, 0, 0, 1);
    vert_set(&tri[1], 1, 0, 0, 1, 0, 2);
    vert_set(&tri[2], 0, 1, 0, 0, 1, 3);
    s = scene_new(3, tri, 1, &one, 1024, 1024, bounds, 777, 1, 0, 0, 0x22);
    CHECK(s != NULL, "1024 builder");
    if (s) {
        expect_ok("dimensions 1024", s);
        scene_free(s);
    }
    s = scene_new(3, tri, 1, &one, 2048, 1, bounds, 777, 1, 0, 0, 0x33);
    CHECK(s != NULL, "2048 builder");
    if (s) {
        expect_fail("width 2048", s->bytes, s->len, 3, 1, -3);
        scene_free(s);
    }
    s = scene_new(3, tri, 1, &one, 1, 2048, bounds, 777, 1, 0, 0, 0x33);
    if (s) {
        expect_fail("height 2048", s->bytes, s->len, 3, 1, -3);
        scene_free(s);
    }
    s = scene_new(3, tri, 1, &one, 3, 1, bounds, 777, 1, 0, 0, 0x33);
    if (s) {
        expect_fail("width not power of two", s->bytes, s->len, 3, 1, -3);
        scene_free(s);
    }
    s = scene_new(3, tri, 1, &one, 0, 1, bounds, 777, 1, 0, 0, 0);
    if (s) {
        expect_fail("width zero", s->bytes, s->len, 3, 1, -3);
        scene_free(s);
    }
    vert_set(&tri[0], -1024.f, 0, 0, 0, 0, 4);
    vert_set(&tri[1], 1024.f, 0, 0, 1, 1, 5);
    vert_set(&tri[2], 0, 1, 0, 0.5f, 0.5f, 6);
    s = scene_new(3, tri, 1, &one, 1, 1, edge, 777, 1, 0, 0, 0x44);
    if (s) {
        expect_ok("bounds at 1024", s);
        scene_free(s);
    }
    n = 262140u;
    one.count = n;
    s = scene_new(n, NULL, 1, &one, 1, 1, bounds, 777, 1, 0, 0, 0x55);
    CHECK(s != NULL, "max vertex builder");
    if (s) {
        uint32_t i;
        for (i = 0; i < n; i++) {
            unsigned char *p = s->bytes + HEADER + (size_t)i * VSTRIDE;
            wr_f32(p + 0, i + 1 == n ? 1.f : 0.f);
            wr_f32(p + 12, i == 123456u ? 0.25f : 0.f);
            wr_u32(p + 20, i);
        }
        expect_ok("max vertices 262140", s);
        scene_free(s);
    }
    one.count = 262143u;
    s = scene_new(262143u, NULL, 1, &one, 1, 1, bounds, 777, 1, 0, 0, 0x66);
    CHECK(s != NULL, "over vertex builder");
    if (s) {
        expect_fail("vertex count 262143", s->bytes, s->len, 262143u, 1, -3);
        scene_free(s);
    }
    one.count = 3;
    big = xmalloc((size_t)MAX_FILE + 1);
    memset(big, 0, (size_t)MAX_FILE + 1);
    memcpy(big, good->bytes, good->len);
    expect_fail("length above 16MiB", big, (size_t)MAX_FILE + 1, good->vc, good->dc,
                -3);
    free(big);
}

static void run_decode(void) {
    struct scene *s = canonical();
    unsigned char *raw, *trail;
    struct tx_vertex verts[13], vsnap[13];
    struct tx_draw draws[4], dsnap[4];
    struct tx_scene_info info;
    uint32_t neg0 = 0x80000000u;
    uint32_t nan = 0x7fc00000u;
    uint32_t inf = 0x7f800000u;
    uint32_t u;
    float f;
    int32_t rc;
    CHECK(s != NULL, "canonical builder");
    if (!s)
        return;
    expect_ok("canonical groups", s);
    raw = xmalloc(s->len + 1);
    memcpy(raw + 1, s->bytes, s->len);
    CHECK(((uintptr_t)(raw + 1) & 3u) != 0, "unaligned fixture");
    memset(verts, 0xA5, sizeof verts);
    memset(draws, 0xA5, sizeof draws);
    memset(&info, 0x3C, sizeof info);
    rc = craftrustnx_texture_decode(raw + 1, s->len, verts, s->vc, draws, s->dc,
                                   &info);
    CHECK(rc == 0, "unaligned rc %ld", (long)rc);
    if (rc == 0) {
        CHECK(memcmp(verts, s->bytes + HEADER, (size_t)s->vc * VSTRIDE) == 0,
              "unaligned vertices");
        CHECK(memcmp(draws, s->bytes + HEADER + (size_t)s->vc * VSTRIDE,
                     (size_t)s->dc * DSTRIDE) == 0,
              "unaligned draws");
    }
    free(raw);

    memcpy(verts, s->bytes + HEADER, 12 * VSTRIDE);
    memcpy(draws, s->bytes + HEADER + 12 * VSTRIDE, 3 * DSTRIDE);
    memset(&info, 0x3C, sizeof info);
    rc = craftrustnx_texture_decode(s->bytes, s->len, verts, 13, draws, 4, &info);
    CHECK(rc == 0, "preload rc %ld", (long)rc);
    memcpy(vsnap, verts, sizeof vsnap);
    memcpy(dsnap, draws, sizeof dsnap);
    raw = xmalloc(s->len);
    memcpy(raw, s->bytes, s->len);
    raw[0] = 'X';
    memset(&info, 0x11, sizeof info);
    rc = craftrustnx_texture_decode(raw, s->len, verts, 13, draws, 4, &info);
    CHECK(rc == -3, "failure after success rc %ld", (long)rc);
    CHECK(memcmp(verts, vsnap, sizeof verts) == 0, "failure after success vertices");
    CHECK(memcmp(draws, dsnap, sizeof draws) == 0, "failure after success draws");
    CHECK(all_zero(&info, sizeof info), "failure after success info");
    free(raw);

    u = 776;
    patch_fail("protocol", s, 8, &u, 4);
    u = 2;
    patch_fail("format", s, 12, &u, 4);
    u = 0;
    patch_fail("zero vertices", s, 16, &u, 4);
    patch_fail("zero draws", s, 20, &u, 4);
    u = 1;
    patch_fail("flags", s, 32, &u, 4);
    patch_fail("header reserved", s, 36, &u, 4);
    f = 2.f;
    patch_fail("vertex outside bounds", s, 64, &f, 4);
    f = 1.5f;
    patch_fail("uv above 1", s, 64 + 12, &f, 4);
    f = -0.25f;
    patch_fail("uv below 0", s, 64 + 16, &f, 4);
    patch_fail("uv nan", s, 64 + 12, &nan, 4);
    patch_fail("position nan", s, 64 + VSTRIDE, &nan, 4);
    patch_fail("position inf", s, 64 + 4, &inf, 4);
    f = 2.f;
    patch_fail("last vertex uv", s, 64 + 11 * VSTRIDE + 12, &f, 4);
    f = 0.f;
    patch_fail("max x excludes vertex", s, 52, &f, 4);
    memcpy(raw = xmalloc(s->len), s->bytes, s->len);
    wr_f32(raw + 40, 0);
    wr_f32(raw + 44, 0);
    wr_f32(raw + 48, 0);
    wr_f32(raw + 52, 0);
    wr_f32(raw + 56, 0);
    wr_f32(raw + 60, 0);
    expect_fail("point bounds", raw, s->len, s->vc, s->dc, -3);
    free(raw);
    f = 2.f;
    patch_fail("inverted bounds", s, 40, &f, 4);
    patch_fail("bounds nan", s, 40, &nan, 4);
    patch_fail("bounds inf", s, 52, &inf, 4);
    f = 1025.f;
    patch_fail("bounds above 1024", s, 52, &f, 4);
    f = -1025.f;
    patch_fail("bounds below -1024", s, 40, &f, 4);
    u = 1;
    patch_fail("draw reserved", s, 64 + 12 * VSTRIDE + 12, &u, 4);
    s->bytes[0] = 'X';
    expect_fail("magic", s->bytes, s->len, s->vc, s->dc, -3);
    s->bytes[0] = 'C';
    expect_fail("truncated", s->bytes, s->len - 1, s->vc, s->dc, -3);
    expect_fail("short header", s->bytes, 63, s->vc, s->dc, -3);
    expect_fail("empty", s->bytes, 0, s->vc, s->dc, -3);
    trail = xmalloc(s->len + 1);
    memcpy(trail, s->bytes, s->len);
    trail[s->len] = 0xFF;
    expect_fail("trailing byte", trail, s->len + 1, s->vc, s->dc, -3);
    free(trail);
    expect_fail("vertex capacity", s->bytes, s->len, s->vc - 1, s->dc, -2);
    expect_fail("draw capacity", s->bytes, s->len, s->vc, s->dc - 1, -2);
    expect_fail("zero capacity", s->bytes, s->len, 0, s->dc, -2);

    memcpy(raw = xmalloc(s->len), s->bytes, s->len);
    memcpy(raw + 64 + 12, &neg0, 4);
    {
        struct scene neg = *s;
        neg.bytes = raw;
        expect_ok("negative zero uv", &neg);
    }
    free(raw);

    memset(verts, 0xA5, sizeof verts);
    memset(draws, 0xA5, sizeof draws);
    memcpy(vsnap, verts, sizeof vsnap);
    memcpy(dsnap, draws, sizeof dsnap);
    memset(&info, 0x3C, sizeof info);
    rc = craftrustnx_texture_decode(NULL, s->len, verts, 12, draws, 3, &info);
    CHECK(rc == -1, "null data %ld", (long)rc);
    CHECK(memcmp(verts, vsnap, sizeof verts) == 0, "null data vertices");
    CHECK(memcmp(draws, dsnap, sizeof draws) == 0, "null data draws");
    CHECK(all_zero(&info, sizeof info), "null data info");
    memset(&info, 0x3C, sizeof info);
    rc = craftrustnx_texture_decode(s->bytes, s->len, NULL, 12, draws, 3, &info);
    CHECK(rc == -1, "null vertices %ld", (long)rc);
    CHECK(memcmp(draws, dsnap, sizeof draws) == 0, "null vertices draws");
    CHECK(all_zero(&info, sizeof info), "null vertices info");
    memset(&info, 0x3C, sizeof info);
    rc = craftrustnx_texture_decode(s->bytes, s->len, verts, 12, NULL, 3, &info);
    CHECK(rc == -1, "null draws %ld", (long)rc);
    CHECK(memcmp(verts, vsnap, sizeof verts) == 0, "null draws vertices");
    CHECK(all_zero(&info, sizeof info), "null draws info");
    rc = craftrustnx_texture_decode(s->bytes, s->len, verts, 12, draws, 3, NULL);
    CHECK(rc == -1, "null info %ld", (long)rc);
    CHECK(memcmp(verts, vsnap, sizeof verts) == 0, "null info vertices");
    CHECK(memcmp(draws, dsnap, sizeof draws) == 0, "null info draws");
    run_group_cases();
    run_limit_cases(s);
    scene_free(s);
}

/* Average clip-w of corners 0,1,2,5. Column-major MVP, w row is 3,7,11,15. */
static float quad_key(const float m[16], const struct tx_vertex *q) {
    static const int corner[4] = {0, 1, 2, 5};
    float sum = 0;
    int i;
    for (i = 0; i < 4; i++) {
        const float *p = q[corner[i]].position;
        sum += (m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15]) * 0.25f;
    }
    return sum;
}

enum { SORT_N = 33 };

static void put_quad(struct tx_vertex *q, float x, float y, float z, float dx,
                     float dy, float dz, uint32_t id) {
    static const float uv[6][2] = {{0, 0}, {1, 0}, {1, 1}, {1, 1}, {0, 1}, {0, 0}};
    int i;
    for (i = 0; i < 6; i++) {
        int decoy = i == 3 || i == 4;
        vert_set(&q[i], decoy ? dx : x, decoy ? dy : y, decoy ? dz : z, uv[i][0],
                 uv[i][1], (id << 8) | (uint32_t)i);
    }
}

static void load_sort(struct tx_vertex *v, struct tx_draw d[3]) {
    int i;
    memset(v, 0, SORT_N * sizeof *v);
    vert_set(&v[0], -1024.f, 0, 0, 0, 0, 0xA0);
    vert_set(&v[1], 1024.f, 0, 0, 1, 1, 0xA1);
    vert_set(&v[2], 0, 0, 0, 0.5f, 0.5f, 0xA2);
    for (i = 0; i < 12; i++)
        vert_set(&v[3 + i], 0, i < 6 ? 0.f : 20.f, 0, 0, 0, 0xC000u + (uint32_t)i);
    put_quad(v + 15, -3, 9, 8, 100, 0, 0, 1);
    put_quad(v + 21, 12, 3, 12, 0, 0, 0, 2);
    put_quad(v + 27, 9, 12, 9, 0, 0, 0, 3);
    d[0] = (struct tx_draw){0, 3, TX_OPAQUE, 0};
    d[1] = (struct tx_draw){3, 12, TX_CUTOUT, 0};
    d[2] = (struct tx_draw){15, 18, TX_TRANSLUCENT, 0};
}

static void expect_sort_keep(const char *name, struct tx_vertex *v, uint32_t count,
                             struct tx_draw *d, uint32_t dc, const float *m,
                             int32_t want) {
    struct tx_vertex *vs = xmalloc((size_t)count * sizeof *vs);
    struct tx_draw *ds = xmalloc((size_t)dc * sizeof *ds);
    int32_t rc;
    memcpy(vs, v, (size_t)count * sizeof *vs);
    memcpy(ds, d, (size_t)dc * sizeof *ds);
    rc = craftrustnx_texture_sort(v, count, d, dc, m);
    CHECK(rc == want, "%s rc %ld want %ld", name, (long)rc, (long)want);
    CHECK(memcmp(v, vs, (size_t)count * sizeof *v) == 0, "%s vertices mutated", name);
    CHECK(memcmp(d, ds, (size_t)dc * sizeof *d) == 0, "%s draws mutated", name);
    free(vs);
    free(ds);
}

static void run_sort(void) {
    struct tx_vertex *v = xmalloc(SORT_N * sizeof *v);
    struct tx_vertex *orig = xmalloc(SORT_N * sizeof *v);
    struct tx_draw d[3], draws_snap[3];
    float m[16], badm[16];
    float kp, kq, kr;
    int32_t rc;
    const float sort_bounds[6] = {-1024.f, 0.f, 0.f, 1024.f, 20.f, 12.f};
    load_sort(v, d);
    memcpy(orig, v, SORT_N * sizeof *v);
    memset(m, 0, sizeof m);
    m[3] = 1.f;
    m[7] = 3.f;
    m[11] = 1.f;
    kp = quad_key(m, v + 15);
    kq = quad_key(m, v + 21);
    kr = quad_key(m, v + 27);
    CHECK(kr > kq && kq > kp, "fixture keys %g %g %g", kr, kq, kp);
    CHECK(fbits_eq(kp, 32.f) && fbits_eq(kq, 33.f) && fbits_eq(kr, 54.f),
          "fixture key bits %g %g %g", kp, kq, kr);
    rc = craftrustnx_texture_sort(v, SORT_N, d, 3, m);
    CHECK(rc == 0, "sort rc %ld", (long)rc);
    CHECK(memcmp(d, (struct tx_draw[3]){{0, 3, 0, 0}, {3, 12, 1, 0}, {15, 18, 2, 0}},
                 sizeof d) == 0,
          "sort rewrote draws");
    if (rc == 0) {
        CHECK(memcmp(v, orig, 15 * sizeof *v) == 0, "opaque/cutout changed");
        CHECK(memcmp(v + 15, orig + 27, 6 * sizeof *v) == 0, "far quad");
        CHECK(memcmp(v + 21, orig + 21, 6 * sizeof *v) == 0, "mid quad");
        CHECK(memcmp(v + 27, orig + 15, 6 * sizeof *v) == 0, "near quad");
        CHECK(!(quad_key(m, v + 15) < quad_key(m, v + 21)) &&
                  !(quad_key(m, v + 21) < quad_key(m, v + 27)),
              "sorted keys");
        memcpy(orig, v, SORT_N * sizeof *v);
        rc = craftrustnx_texture_sort(v, SORT_N, d, 3, m);
        CHECK(rc == 0 && memcmp(v, orig, SORT_N * sizeof *v) == 0, "sort idempotent");
    }

    {
        struct tx_vertex neg[15], norig[15];
        struct tx_draw nd[2];
        float zm[16];
        memset(zm, 0, sizeof zm);
        zm[11] = 1.f;
        memset(neg, 0, sizeof neg);
        vert_set(&neg[0], 0, 0, 0, 0, 0, 1);
        vert_set(&neg[1], 0, 0, 0, 0, 0, 2);
        vert_set(&neg[2], 0, 0, 0, 0, 0, 3);
        put_quad(neg + 3, 0, 0, -8.f, 0, 0, 50.f, 8);
        put_quad(neg + 9, 0, 0, -1.f, 0, 0, -1.f, 1);
        nd[0] = (struct tx_draw){0, 3, TX_OPAQUE, 0};
        nd[1] = (struct tx_draw){3, 12, TX_TRANSLUCENT, 0};
        memcpy(norig, neg, sizeof neg);
        CHECK(quad_key(zm, neg + 9) > quad_key(zm, neg + 3), "negative fixture");
        rc = craftrustnx_texture_sort(neg, 15, nd, 2, zm);
        CHECK(rc == 0, "negative sort %ld", (long)rc);
        if (rc == 0) {
            CHECK(memcmp(neg, norig, 3 * sizeof *neg) == 0, "negative opaque");
            CHECK(memcmp(neg + 3, norig + 9, 6 * sizeof *neg) == 0, "less negative quad");
            CHECK(memcmp(neg + 9, norig + 3, 6 * sizeof *neg) == 0, "more negative quad");
        }
    }

    load_sort(v, d);
    memcpy(badm, m, sizeof m);
    badm[5] = bits_f(0x7fc00000u);
    expect_sort_keep("nan mvp", v, SORT_N, d, 3, badm, -3);
    memcpy(badm, m, sizeof m);
    badm[0] = bits_f(0x7f800000u);
    expect_sort_keep("inf mvp", v, SORT_N, d, 3, badm, -3);
    memcpy(badm, m, sizeof m);
    badm[15] = bits_f(0xff800000u);
    expect_sort_keep("neg inf mvp", v, SORT_N, d, 3, badm, -3);
    load_sort(v, d);
    v[0].position[0] = bits_f(0x7fc00000u);
    expect_sort_keep("nan position", v, SORT_N, d, 3, m, -3);
    load_sort(v, d);
    v[4].position[1] = 1025.f;
    expect_sort_keep("position above 1024", v, SORT_N, d, 3, m, -3);
    load_sort(v, d);
    v[16].position[0] = -1025.f;
    expect_sort_keep("position below -1024", v, SORT_N, d, 3, m, -3);
    load_sort(v, d);
    memset(badm, 0, sizeof badm);
    badm[11] = 1.0e38f;
    CHECK(isfinite(badm[11]), "huge mvp finite");
    expect_sort_keep("nonfinite depth", v, SORT_N, d, 3, badm, -3);
    load_sort(v, d);
    d[1].reserved = 1;
    expect_sort_keep("sort reserved", v, SORT_N, d, 3, m, -3);
    load_sort(v, d);
    d[0].mode = TX_TRANSLUCENT;
    d[1].mode = TX_CUTOUT;
    d[2].mode = TX_OPAQUE;
    expect_sort_keep("sort mode order", v, SORT_N, d, 3, m, -3);
    load_sort(v, d);
    {
        struct tx_draw bad = {0, 9, TX_TRANSLUCENT, 0};
        expect_sort_keep("sort not quads", v, 9, &bad, 1, m, -3);
    }
    load_sort(v, d);
    memcpy(orig, v, SORT_N * sizeof *v);
    rc = craftrustnx_texture_sort(v, 0, d, 3, m);
    CHECK(rc == -3 && memcmp(v, orig, SORT_N * sizeof *v) == 0, "sort count zero");
    memcpy(draws_snap, d, sizeof d);
    rc = craftrustnx_texture_sort(NULL, SORT_N, d, 3, m);
    CHECK(rc == -1, "null sort vertices %ld", (long)rc);
    CHECK(memcmp(d, draws_snap, sizeof d) == 0, "null sort vertices draws");
    load_sort(v, d);
    memcpy(orig, v, SORT_N * sizeof *v);
    rc = craftrustnx_texture_sort(v, SORT_N, NULL, 3, m);
    CHECK(rc == -1 && memcmp(v, orig, SORT_N * sizeof *v) == 0, "null sort draws");
    rc = craftrustnx_texture_sort(v, SORT_N, d, 3, NULL);
    CHECK(rc == -1 && memcmp(v, orig, SORT_N * sizeof *v) == 0, "null sort mvp");
    rc = craftrustnx_texture_sort(v, 262141u, d, 3, m);
    CHECK(rc == -3 && memcmp(v, orig, SORT_N * sizeof *v) == 0, "sort over max vertices");
    rc = craftrustnx_texture_sort(v, SORT_N, d, 257u, m);
    CHECK(rc == -3 && memcmp(v, orig, SORT_N * sizeof *v) == 0, "sort over max draws");
    rc = craftrustnx_texture_sort(v, SORT_N, d, 0, m);
    CHECK(rc == -3 && memcmp(v, orig, SORT_N * sizeof *v) == 0, "sort zero draws");

    {
        struct scene *s = scene_new(SORT_N, v, 3, d, 1, 1, sort_bounds, 777, 1, 0, 0, 9);
        struct tx_vertex *out;
        struct tx_draw od[4];
        struct tx_scene_info info;
        CHECK(s != NULL, "sort file builder");
        if (s) {
            out = xmalloc((SORT_N + 1) * sizeof *out);
            memset(out, 0xA5, (SORT_N + 1) * sizeof *out);
            memset(od, 0xA5, sizeof od);
            rc = craftrustnx_texture_decode(s->bytes, s->len, out, SORT_N, od, 3, &info);
            CHECK(rc == 0, "decode sort scene %ld", (long)rc);
            if (rc == 0) {
                CHECK(memcmp(out, s->bytes + HEADER, SORT_N * VSTRIDE) == 0,
                      "decoded sort vertices");
                rc = craftrustnx_texture_sort(out, SORT_N, od, 3, m);
                CHECK(rc == 0, "sort decoded %ld", (long)rc);
                CHECK(memcmp(out, s->bytes + HEADER, 15 * VSTRIDE) == 0,
                      "decoded prefix");
                CHECK(memcmp(out + 15, s->bytes + HEADER + 27 * VSTRIDE, 6 * VSTRIDE) == 0,
                      "decoded far");
                CHECK(memcmp(out + 21, s->bytes + HEADER + 21 * VSTRIDE, 6 * VSTRIDE) == 0,
                      "decoded mid");
                CHECK(memcmp(out + 27, s->bytes + HEADER + 15 * VSTRIDE, 6 * VSTRIDE) == 0,
                      "decoded near");
            }
            free(out);
            scene_free(s);
        }
    }
    free(orig);
    free(v);
}

static void run_legacy(void) {
    unsigned char mesh[96], *corrupt;
    struct tp_vertex *verts = xmalloc(4 * sizeof *verts);
    struct tp_vertex snap;
    struct tp_mesh_info info;
    struct tp_app_state state;
    struct tp_app_frame frame;
    int32_t rc;
    unsigned i;
    memset(mesh, 0, sizeof mesh);
    memcpy(mesh, "CTMESH01", 8);
    wr_u32(mesh + 8, 777);
    wr_u32(mesh + 12, 1);
    wr_u32(mesh + 16, 3);
    wr_u32(mesh + 20, 0);
    wr_f32(mesh + 24, -1);
    wr_f32(mesh + 28, 0);
    wr_f32(mesh + 32, 0);
    wr_f32(mesh + 36, 1);
    wr_f32(mesh + 40, 1);
    wr_f32(mesh + 44, 0);
    wr_f32(mesh + 48, -1);
    wr_f32(mesh + 52, 0);
    wr_f32(mesh + 56, 0);
    wr_u32(mesh + 60, 0xff0000ffu);
    wr_f32(mesh + 64, 1);
    wr_f32(mesh + 68, 0);
    wr_f32(mesh + 72, 0);
    wr_u32(mesh + 76, 0xff00ff00u);
    wr_f32(mesh + 80, 0);
    wr_f32(mesh + 84, 1);
    wr_f32(mesh + 88, 0);
    wr_u32(mesh + 92, 0xffff0000u);
    memset(verts, 0xA5, 4 * sizeof *verts);
    snap = verts[3];
    memset(&info, 0x3C, sizeof info);
    rc = craftrustnx_mesh_decode(mesh, sizeof mesh, verts, 3, &info);
    CHECK(rc == 0, "legacy mesh %ld", (long)rc);
    CHECK(info.vertex_count == 3, "legacy count");
    CHECK(memcmp(verts, mesh + 48, 48) == 0, "legacy vertices");
    CHECK(memcmp(&verts[3], &snap, sizeof snap) == 0, "legacy overrun");
    memset(&state, 0, sizeof state);
    memset(&frame, 0x3C, sizeof frame);
    rc = craftrustnx_terrain_update(&state, &info, 0, 0, 0, 0, &frame);
    CHECK(rc == 0, "legacy camera %ld", (long)rc);
    CHECK(state.frames == 1 && state.auto_orbit == 1, "legacy camera state");
    CHECK(fbits_eq(state.zoom, 1.f), "legacy zoom");
    CHECK(frame.should_exit == 0 && fbits_eq(frame.clear[3], 1.f), "legacy frame");
    for (i = 0; i < 16; i++)
        CHECK(isfinite(frame.mvp[i]), "legacy mvp %u", i);
    corrupt = xmalloc(sizeof mesh);
    memcpy(corrupt, mesh, sizeof mesh);
    corrupt[0] = 'X';
    memset(verts, 0xA5, 4 * sizeof *verts);
    snap = verts[0];
    memset(&info, 0x3C, sizeof info);
    rc = craftrustnx_mesh_decode(corrupt, sizeof mesh, verts, 3, &info);
    CHECK(rc == -3, "legacy corrupt %ld", (long)rc);
    CHECK(memcmp(&verts[0], &snap, sizeof snap) == 0, "legacy corrupt vertices");
    CHECK(all_zero(&info, sizeof info), "legacy corrupt info");
    memset(&state, 0, sizeof state);
    rc = craftrustnx_terrain_update(&state, &info, 0, 0, 0, 0, NULL);
    CHECK(rc == -1 && state.frames == 0, "legacy null frame");
    free(corrupt);
    free(verts);
}

static void run_scene(const char *path) {
    FILE *f = fopen(path, "rb");
    long sz;
    unsigned char *data;
    uint32_t protocol, format, vc, dc, width, height, flags, reserved;
    size_t expect, i, nq = 0, tfirst = 0, tcount = 0;
    struct tx_vertex *verts, *before;
    struct tx_draw *draws, *dsnap;
    struct tx_scene_info info;
    float m[16];
    int32_t rc;
    unsigned char *used = NULL;
    CHECK(f != NULL, "open scene %s", path);
    if (!f) {
        snprintf(scene_note, sizeof scene_note, "scene missing");
        return;
    }
    CHECK(fseek(f, 0, SEEK_END) == 0, "scene seek");
    sz = ftell(f);
    CHECK(sz >= HEADER && sz <= MAX_FILE, "scene size %ld", sz);
    if (sz < HEADER || sz > MAX_FILE) {
        fclose(f);
        snprintf(scene_note, sizeof scene_note, "scene size rejected");
        return;
    }
    rewind(f);
    data = xmalloc((size_t)sz);
    const size_t received = fread(data, 1, (size_t)sz, f);
    CHECK(received == (size_t)sz && !ferror(f), "scene read");
    fclose(f);
    if (received != (size_t)sz) {
        free(data);
        return;
    }
    CHECK(memcmp(data, "CTTEX001", 8) == 0, "scene magic");
    protocol = rd_u32(data + 8);
    format = rd_u32(data + 12);
    vc = rd_u32(data + 16);
    dc = rd_u32(data + 20);
    width = rd_u32(data + 24);
    height = rd_u32(data + 28);
    flags = rd_u32(data + 32);
    reserved = rd_u32(data + 36);
    /* Bound untrusted header fields before allocation or length arithmetic. */
    const int bounded = vc > 0 && vc <= TX_MAX_VERTICES && vc % 3u == 0 &&
        dc > 0 && dc <= TX_MAX_DRAWS && width > 0 && width <= TX_MAX_DIMENSION &&
        height > 0 && height <= TX_MAX_DIMENSION && !(width & (width - 1)) &&
        !(height & (height - 1));
    CHECK(bounded, "scene bounded header");
    if (!bounded) {
        free(data);
        return;
    }
    expect = HEADER + (size_t)vc * VSTRIDE + (size_t)dc * DSTRIDE +
             (size_t)width * (size_t)height * 4u;
    CHECK(protocol == 777 && format == 1 && flags == 0 && reserved == 0,
          "scene header fields");
    CHECK((size_t)sz == expect, "scene exact length %ld want %zu", sz, expect);
    if ((size_t)sz != expect) {
        free(data);
        return;
    }
    verts = xmalloc(((size_t)vc + 1) * sizeof *verts);
    draws = xmalloc(((size_t)dc + 1) * sizeof *draws);
    memset(verts, 0xA5, ((size_t)vc + 1) * sizeof *verts);
    memset(draws, 0xA5, ((size_t)dc + 1) * sizeof *draws);
    memset(&info, 0x3C, sizeof info);
    rc = craftrustnx_texture_decode(data, (size_t)sz, verts, vc, draws, dc, &info);
    CHECK(rc == 0, "scene decode %ld", (long)rc);
    if (rc != 0) {
        snprintf(scene_note, sizeof scene_note, "scene decode %ld", (long)rc);
        free(draws);
        free(verts);
        free(data);
        return;
    }
    CHECK(info.mesh.vertex_count == vc && info.draw_count == dc, "scene counts");
    CHECK(info.width == width && info.height == height, "scene dimensions");
    CHECK(info.atlas_offset == HEADER + vc * VSTRIDE + dc * DSTRIDE, "scene atlas");
    CHECK(info.atlas_bytes == width * height * 4u, "scene atlas bytes");
    for (i = 0; i < 3; i++) {
        CHECK(fbits_eq(info.mesh.min[i], rd_f32(data + 40 + i * 4)), "scene min");
        CHECK(fbits_eq(info.mesh.max[i], rd_f32(data + 52 + i * 4)), "scene max");
    }
    CHECK(memcmp(verts, data + HEADER, (size_t)vc * VSTRIDE) == 0, "scene vertices");
    CHECK(memcmp(draws, data + HEADER + (size_t)vc * VSTRIDE, (size_t)dc * DSTRIDE) == 0,
          "scene draws");
    {
        struct tx_vertex guard;
        struct tx_draw dguard;
        memset(&guard, 0xA5, sizeof guard);
        memset(&dguard, 0xA5, sizeof dguard);
        CHECK(memcmp(&verts[vc], &guard, sizeof guard) == 0, "scene vertex overrun");
        CHECK(memcmp(&draws[dc], &dguard, sizeof dguard) == 0, "scene draw overrun");
    }
    before = xmalloc((size_t)vc * sizeof *before);
    dsnap = xmalloc((size_t)dc * sizeof *dsnap);
    memcpy(before, verts, (size_t)vc * sizeof *before);
    memcpy(dsnap, draws, (size_t)dc * sizeof *dsnap);
    memset(m, 0, sizeof m);
    m[3] = 1.f;
    m[7] = 3.f;
    m[11] = 1.f;
    rc = craftrustnx_texture_sort(verts, vc, draws, dc, m);
    CHECK(rc == 0, "scene sort %ld", (long)rc);
    CHECK(memcmp(draws, dsnap, (size_t)dc * sizeof *draws) == 0, "scene sort draws");
    if (rc == 0) {
        for (i = 0; i < dc; i++) {
            if (draws[i].mode == TX_TRANSLUCENT) {
                tfirst = draws[i].first;
                tcount = draws[i].count;
            }
        }
        CHECK(tcount % 6u == 0 && tfirst + tcount <= vc, "scene translucent range");
        if (tfirst + tcount <= vc) {
            CHECK(memcmp(verts, before, tfirst * sizeof *verts) == 0, "scene prefix");
            CHECK(memcmp(verts + tfirst + tcount, before + tfirst + tcount,
                         ((size_t)vc - (tfirst + tcount)) * sizeof *verts) == 0,
                  "scene suffix");
        }
        nq = tcount / 6u;
        used = xmalloc(nq ? nq : 1);
        memset(used, 0, nq ? nq : 1);
        for (i = 0; i < nq; i++) {
            size_t j;
            int found = 0;
            float ka, kb;
            for (j = 0; j < nq; j++) {
                if (used[j])
                    continue;
                if (memcmp(verts + tfirst + i * 6, before + tfirst + j * 6,
                           6 * sizeof *verts) == 0) {
                    used[j] = 1;
                    found = 1;
                    break;
                }
            }
            CHECK(found, "scene quad %zu split or lost", i);
            if (i + 1 < nq) {
                ka = quad_key(m, verts + tfirst + i * 6);
                kb = quad_key(m, verts + tfirst + (i + 1) * 6);
                CHECK(isfinite(ka) && isfinite(kb) && !(ka < kb),
                      "scene order %zu keys %g %g", i, ka, kb);
            }
        }
        snprintf(scene_note, sizeof scene_note, "scene %u verts %u draws %ux%u quads %zu",
                 vc, dc, width, height, nq);
    }
    free(used);
    free(dsnap);
    free(before);
    free(draws);
    free(verts);
    free(data);
}

int main(int argc, char **argv) {
    if (!host_le()) {
        fprintf(stderr, "host is not little-endian\n");
        return 2;
    }
    if (argc > 2) {
        fprintf(stderr, "usage: host_check [scene.bin]\n");
        return 2;
    }
    run_decode();
    run_sort();
    run_legacy();
    if (argc == 2)
        run_scene(argv[1]);
    printf("%s host texture ABI checks=%u failures=%u\n", fails ? "FAIL" : "PASS",
           checks, fails);
    printf("ASan/UBSan instrumented the C harness only; Rust staticlib was not "
           "instrumented.\n");
    printf("Not a hardware texture pass. %s\n", scene_note);
    return fails ? 1 : 0;
}
