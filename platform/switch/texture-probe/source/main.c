/* SDK lifecycle and file/pad I/O. Rust validates, controls camera and sorts. */
#include "texture_app.h"
#ifdef CRAFTRUSTNX_COLUMN_MESH
#include "column_mesh.h"
_Static_assert(sizeof(struct cm_info) == 72, "column mesh ABI");
#endif
#include <inttypes.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <switch.h>
_Static_assert(sizeof(struct tx_vertex) == 24, "texture vertex ABI");
_Static_assert(sizeof(struct tx_draw) == 16, "draw ABI");
_Static_assert(sizeof(struct tx_scene_info) == 48, "scene ABI");
#ifdef CRAFTRUSTNX_COLUMN_MESH
static const char *TITLE = "CraftRustNX: Rust Column Mesh Probe 0.12.0";
static const char *SCOPE = "Rust builds a saved section on Switch. First model options, neutral tint. "
                          "No world light, live updates or gameplay.";
static const char *PATH = "sdmc:/switch/craftrustnx/section-model-bank.bin";
static const char *CAPTURE_PATH = "sdmc:/switch/craftrustnx/column-mesh-capture.ctcol";
static const char *CHECK_LABEL = "Compare this section with 0.11; check plants, cutout holes and camera.";
static const char *COPY_LABEL = "Copy section-model-bank.bin and column-mesh-capture.ctcol beside this NRO.";
#elif defined(CRAFTRUSTNX_SECTION_PREVIEW)
static const char *TITLE = "CraftRustNX: Captured Section Probe 0.11.0";
static const char *SCOPE = "One captured Java section. First model options, neutral tint. "
                           "No world light, live updates or gameplay.";
static const char *PATH = "sdmc:/switch/craftrustnx/captured-section-scene.bin";
static const char *CHECK_LABEL = "Check captured ground, trees/plants, cutout holes and camera controls.";
static const char *COPY_LABEL = "Copy captured-section-scene.bin beside this NRO and relaunch.";
#else
static const char *TITLE = "CraftRustNX: Rust Texture Probe 0.9.1";
static const char *SCOPE = "Static models and textures. Neutral tint; no "
                           "biome/light/animation or gameplay.";
static const char *PATH = "sdmc:/switch/craftrustnx/texture-probe-scene.bin";
static const char *CHECK_LABEL = "Check nine textured shapes, flower holes, glass/depth and sprite edges.";
static const char *COPY_LABEL = "Copy texture-probe-scene.bin beside this NRO and relaunch.";
#endif
static const char *CONTROLS =
    "A: orbit | B: background | X: reset | right stick: camera | ZL/ZR: zoom";
static const char *EXIT_LABEL = "Plus: return to hbmenu";
static void wait_exit(PadState *pad) {
  while (appletMainLoop()) {
    padUpdate(pad);
    if (padGetButtonsDown(pad) & HidNpadButton_Plus)
      break;
    consoleUpdate(NULL);
  }
}
struct scene {
  uint8_t *bytes;
  struct tx_vertex *vertices;
  struct tx_draw draws[TX_MAX_DRAWS];
  struct tx_scene_info info;
#ifdef CRAFTRUSTNX_COLUMN_MESH
  struct cm_info column;
  uint64_t mesh_ns, vertex_fingerprint;
#endif
};
static void release_scene(struct scene *s) {
  free(s->bytes);
  free(s->vertices);
  s->bytes = NULL;
  s->vertices = NULL;
}
#ifndef CRAFTRUSTNX_COLUMN_MESH
static int32_t load_scene(struct scene *s) {
  FILE *f = fopen(PATH, "rb");
  if (!f)
    return -101;
  if (fseek(f, 0, SEEK_END) != 0) {
    fclose(f);
    return -102;
  }
  const long size = ftell(f);
  if (size < 64 || size > 16 * 1024 * 1024 || fseek(f, 0, SEEK_SET) != 0) {
    fclose(f);
    return -103;
  }
  size_t capacity = ((size_t)size - 64) / 24;
  if (capacity > TX_MAX_VERTICES)
    capacity = TX_MAX_VERTICES;
  if (capacity == 0) {
    fclose(f);
    return -103;
  }
  s->bytes = malloc((size_t)size);
  s->vertices = malloc(capacity * sizeof(*s->vertices));
  if (!s->bytes || !s->vertices) {
    fclose(f);
    release_scene(s);
    return -104;
  }
  const size_t n = fread(s->bytes, 1, (size_t)size, f);
  const int tail = fgetc(f);
  const bool complete = n == (size_t)size && tail == EOF && !ferror(f);
  fclose(f);
  if (!complete) {
    release_scene(s);
    return -105;
  }
  const int32_t result =
      craftrustnx_texture_decode(s->bytes, (size_t)size, s->vertices, capacity,
                                 s->draws, TX_MAX_DRAWS, &s->info);
  if (result != 0)
    release_scene(s);
  return result;
}
#else
static int32_t read_blob(const char *path, size_t maximum, uint8_t **bytes, size_t *size) {
  FILE *f = fopen(path, "rb");
  if (!f) return -101;
  if (fseek(f, 0, SEEK_END)) { fclose(f); return -102; }
  const long length = ftell(f);
  if (length < 48 || (size_t)length > maximum || fseek(f, 0, SEEK_SET)) {
    fclose(f); return -103;
  }
  *bytes = malloc((size_t)length);
  if (!*bytes) { fclose(f); return -104; }
  const size_t got = fread(*bytes, 1, (size_t)length, f);
  const int tail = fgetc(f), error = ferror(f), closed = fclose(f);
  if (got != (size_t)length || tail != EOF || error || closed) {
    free(*bytes); *bytes = NULL; return -105;
  }
  *size = (size_t)length;
  return 0;
}
static int32_t load_scene(struct scene *s) {
  uint8_t *capture = NULL;
  size_t capture_length = 0, bank_length = 0;
  int32_t result = read_blob(CAPTURE_PATH, 4259888, &capture, &capture_length);
  if (result) return result;
  result = read_blob(PATH, 16*1024*1024, &s->bytes, &bank_length);
  if (result) { free(capture); return result; }
  const uint64_t begin = armGetSystemTick();
  result = craftrustnx_column_mesh(capture, capture_length, s->bytes, bank_length,
      8, NULL, 0, NULL, 0, &s->column);
  if (result == 0) {
    const uint32_t count = s->column.scene.mesh.vertex_count;
    if (!count || count > TX_MAX_VERTICES) result = -103;
    else {
      s->vertices = malloc((size_t)count * sizeof(*s->vertices));
      if (!s->vertices) result = -104;
      else result = craftrustnx_column_mesh(capture, capture_length, s->bytes, bank_length,
          8, s->vertices, count, s->draws, TX_MAX_DRAWS, &s->column);
    }
  }
  s->mesh_ns = armTicksToNs(armGetSystemTick() - begin);
  free(capture);
  if (result) { release_scene(s); return result; }
  s->info = s->column.scene;
  // Diagnostic fingerprint over the generated vertex bytes, not a security hash.
  const uint8_t *v = (const uint8_t *)s->vertices;
  s->vertex_fingerprint = UINT64_C(14695981039346656037);
  for (size_t i = 0; i < (size_t)s->info.mesh.vertex_count * sizeof(*s->vertices); ++i) {
    s->vertex_fingerprint ^= v[i];
    s->vertex_fingerprint *= UINT64_C(1099511628211);
  }
  return 0;
}
#endif
static uint32_t input(uint64_t down, uint64_t held) {
  uint32_t b = 0;
  if (down & HidNpadButton_A)
    b |= TP_ORBIT;
  if (down & HidNpadButton_B)
    b |= TP_BACKGROUND;
  if (down & HidNpadButton_X)
    b |= TP_RESET;
  if (down & HidNpadButton_Plus)
    b |= TP_EXIT;
  if (held & HidNpadButton_ZL)
    b |= TP_ZOOM_IN;
  if (held & HidNpadButton_ZR)
    b |= TP_ZOOM_OUT;
  return b;
}
int main(int argc, char **argv) {
  (void)argc;
  (void)argv;
  consoleInit(NULL);
  padConfigureInput(1, HidNpadStyleSet_NpadStandard);
  PadState pad;
  padInitializeDefault(&pad);
  puts(TITLE);
  puts(SCOPE);
  puts(CONTROLS);
  puts(PATH);
#ifdef CRAFTRUSTNX_COLUMN_MESH
  puts(CAPTURE_PATH);
#endif
  puts(CHECK_LABEL);
  struct scene scene = {0};
  int32_t status = load_scene(&scene);
  const Result romfs = romfsInit();
  if (status != 0 || R_FAILED(romfs)) {
    printf("Scene status: %" PRId32 " | RomFS: 0x%08x\n", status,
           (unsigned)romfs);
    puts(COPY_LABEL);
    puts(EXIT_LABEL);
    wait_exit(&pad);
    release_scene(&scene);
    if (R_SUCCEEDED(romfs))
      romfsExit();
    consoleExit(NULL);
    return 1;
  }
  printf("Rust validated %" PRIu32 " vertices, %" PRIu32
         " draws, atlas %" PRIu32 " x %" PRIu32 ".\n",
         scene.info.mesh.vertex_count, scene.info.draw_count, scene.info.width,
         scene.info.height);
#ifdef CRAFTRUSTNX_COLUMN_MESH
  printf("Rust meshed section %" PRIu32 " (Y=%" PRId32 "), modeled=%" PRIu32 " empty=%" PRIu32 ".\n",
      scene.column.section_index, scene.column.section_min_y,
      scene.column.cells_with_geometry, scene.column.empty_cells);
  printf("Query + build + allocation: %.3f ms (file I/O excluded).\n", scene.mesh_ns / 1e6);
  printf("Vertex fingerprint: %016" PRIx64 "\n", scene.vertex_fingerprint);
#endif
  puts("A: start 3D | Plus: exit");
  bool start = false;
  while (appletMainLoop()) {
    padUpdate(&pad);
    uint64_t down = padGetButtonsDown(&pad);
    if (down & HidNpadButton_Plus)
      break;
    if (down & HidNpadButton_A) {
      start = true;
      break;
    }
    consoleUpdate(NULL);
  }
  if (!start) {
    release_scene(&scene);
    romfsExit();
    consoleExit(NULL);
    return 0;
  }
  consoleExit(NULL);
  status = tx_graphics_initialize();
  if (status == 0)
    status = tx_graphics_atlas(scene.bytes + scene.info.atlas_offset,
                               scene.info.width, scene.info.height);
  free(scene.bytes);
  scene.bytes = NULL;
  struct tp_app_state state = {0};
  struct tp_app_frame frame = {0};
  float previous_mvp[16] = {0};
  bool have_mesh = false;
  bool needs_depth_sort = false;
  for (uint32_t i = 0; i < scene.info.draw_count; ++i)
    if (scene.draws[i].mode == TX_TRANSLUCENT) needs_depth_sort = true;
  const uint64_t tick = armGetSystemTick();
  uint64_t frames = 0;
  bool closing = false;
  while (status == 0) {
    if (!appletMainLoop()) {
      closing = true;
      break;
    }
    padUpdate(&pad);
    HidAnalogStickState stick = padGetStickPos(&pad, 1);
    status = craftrustnx_terrain_update(
        &state, &scene.info.mesh, armTicksToNs(armGetSystemTick() - tick),
        input(padGetButtonsDown(&pad), padGetButtons(&pad)), stick.x / 32767.0f,
        stick.y / 32767.0f, &frame);
    if (status != 0 || frame.should_exit)
      break;
    // Opaque/cutout geometry is camera-independent and stays on the GPU.
    // Only blended quads need re-sorting and re-upload on a camera change.
    if (!have_mesh || (needs_depth_sort &&
        memcmp(previous_mvp, frame.mvp, sizeof(previous_mvp)) != 0)) {
      status = craftrustnx_texture_sort(
          scene.vertices, scene.info.mesh.vertex_count, scene.draws,
          scene.info.draw_count, frame.mvp);
      if (status == 0)
        status =
            tx_graphics_upload(scene.vertices, scene.info.mesh.vertex_count,
                               scene.draws, scene.info.draw_count);
      if (status != 0)
        break;
      memcpy(previous_mvp, frame.mvp, sizeof(previous_mvp));
      have_mesh = true;
    }
    status = tx_graphics_present(frame.mvp, frame.clear);
    if (status == 0)
      ++frames;
  }
  const uint64_t gpu_bytes = tx_graphics_allocated_bytes();
  tx_graphics_shutdown();
  romfsExit();
  release_scene(&scene);
  if (!closing) {
    consoleInit(NULL);
    puts(TITLE);
    printf("Status: %" PRId32 " | submitted frames: %" PRIu64
           " | GPU blocks: %" PRIu64 " bytes\n",
           status, frames, gpu_bytes);
    puts(SCOPE);
    puts(EXIT_LABEL);
    wait_exit(&pad);
    consoleExit(NULL);
  }
  return status == 0 ? 0 : 1;
}
