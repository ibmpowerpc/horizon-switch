#ifndef CRAFTRUSTNX_COLUMN_MESH_H
#define CRAFTRUSTNX_COLUMN_MESH_H
#include "texture_app.h"
/* CTBANK01 header64 (LE): magic8, protocol777@8, format1@12,
 * states@16 (1..65536), template_vertices@20 (0..262140, multiple6),
 * atlas width/height@24/28 (power2 1..1024), state_stride32@32,
 * vertex_stride24@36, zero@40/44, atlas_bytes@48, zero@52/56/60.
 * Then states*32 sorted unique state records: raw ID@0, first/count for
 * modes0,1,2@4/8,12/16,20/24, zero@28. Template ranges may alias; every
 * first/count multiple6 and range in template array. Then template_vertices
 * tx_vertex records24 (LE float3/float2/RGBA8), then exact RGBA8 atlas.
 * Local positions finite abs<=1024, UV finite0..1. Exact length <=16MiB.
 * Raw IDs have no intrinsic air/solid semantics: empty ranges explicitly map
 * invisible/empty models. A missing ID is unsupported, never a cube fallback.
 */
struct cm_info {
    struct tx_scene_info scene;
    int32_t chunk_x, chunk_z, section_min_y;
    uint32_t section_index, cells_with_geometry, empty_cells;
};
/* CTCOL001 capture: unchanged 48-byte console snapshot contract, <=4259888B.
 * Validate the complete capture/bank and selected section before writing.
 * Output whole six-vertex quads, mode0/1/2 groups in Y/Z/X cell order; original
 * template order inside each group, positions translated by local cell XYZ.
 * No RNG/jitter, occlusion culling, lighting or tint/color replacement.
 * scene.atlas_offset points inside BANK (not an encoded CTTEX001 scene).
 * Query: vertices/draws=NULL, both capacities0 -> success info, no arrays.
 * Build: non-null aligned outputs, truthful capacities in elements. Input and
 * outputs must be disjoint, readable/writable for stated lengths/capacities.
 * Return0 success, -1 bad pointer/argument, -2 capacity, -3 malformed input,
 * -5 missing model or empty section. Info zeroed on failure; vertex/draw
 * buffers unchanged on every failure; success leaves output tails unchanged.
 */
int32_t craftrustnx_column_mesh(const uint8_t *capture, size_t capture_len,
    const uint8_t *bank, size_t bank_len, uint32_t section,
    struct tx_vertex *vertices, size_t vertex_capacity,
    struct tx_draw *draws, size_t draw_capacity, struct cm_info *info);
#endif
