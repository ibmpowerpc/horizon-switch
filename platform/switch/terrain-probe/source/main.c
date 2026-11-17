/* SDK lifecycle, file/pad I/O only. Rust validates geometry and owns camera. */
#include "terrain_app.h"
#include <inttypes.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <switch.h>

_Static_assert(sizeof(struct tp_vertex) == 16, "vertex ABI");
_Static_assert(sizeof(struct tp_mesh_info) == 28, "mesh info ABI");
_Static_assert(sizeof(struct tp_app_state) == 32, "state ABI");
_Static_assert(sizeof(struct tp_app_frame) == 96, "frame ABI");
_Static_assert(offsetof(struct tp_app_frame, mvp) == 16, "MVP ABI");

/* Centralized labels for this platform diagnostic; game UI uses translation keys. */
static const char *TITLE = "CraftRustNX: Rust Model Mesh Probe 0.8.0";
static const char *SCOPE = "Static baked model geometry. Diagnostic colors; textures/alpha/light absent.";
static const char *START = "A: start 3D | Plus: exit";
static const char *CONTROLS = "A: orbit | B: background | X: reset | right stick: camera | ZL/ZR: zoom";
static const char *FILE_PATH = "sdmc:/switch/craftrustnx/terrain-probe-mesh.bin";
static const char *EXPECT = "Expect nine static block shapes with rotation, perspective and depth.";
static const char *EXIT_LABEL = "Plus: return to hbmenu";

static void wait_exit(PadState *pad) {
    while (appletMainLoop()) {
        padUpdate(pad);
        if (padGetButtonsDown(pad) & HidNpadButton_Plus) break;
        consoleUpdate(NULL);
    }
}

static int32_t load_mesh(struct tp_vertex **out, struct tp_mesh_info *info) {
    const size_t max_bytes = 48u + 16u * TP_MAX_VERTICES;
    *out = NULL;
    FILE *file = fopen(FILE_PATH, "rb");
    if (!file) return -101;
    if (fseek(file, 0, SEEK_END) != 0) { fclose(file); return -102; }
    const long size = ftell(file);
    if (size < 48 || (uint64_t)size > max_bytes || fseek(file, 0, SEEK_SET) != 0) {
        fclose(file); return -103;
    }
    const size_t capacity = ((size_t)size - 48u) / sizeof(struct tp_vertex);
    if (capacity == 0) { fclose(file); return -103; }
    uint8_t *bytes = malloc((size_t)size);
    struct tp_vertex *vertices = malloc(capacity * sizeof(*vertices));
    if (!bytes || !vertices) {
        free(bytes); free(vertices); fclose(file); return -104;
    }
    const size_t received = fread(bytes, 1, (size_t)size, file);
    const int trailing = fgetc(file);
    const bool complete = received == (size_t)size && trailing == EOF && !ferror(file);
    fclose(file);
    if (!complete) { free(bytes); free(vertices); return -105; }
    const int32_t result = craftrustnx_mesh_decode(bytes, (size_t)size, vertices, capacity, info);
    free(bytes);
    if (result != 0) { free(vertices); return result; }
    *out = vertices;
    return 0;
}

static uint32_t input(uint64_t down, uint64_t held) {
    uint32_t b = 0;
    if (down & HidNpadButton_A) b |= TP_ORBIT;
    if (down & HidNpadButton_B) b |= TP_BACKGROUND;
    if (down & HidNpadButton_X) b |= TP_RESET;
    if (down & HidNpadButton_Plus) b |= TP_EXIT;
    if (held & HidNpadButton_ZL) b |= TP_ZOOM_IN;
    if (held & HidNpadButton_ZR) b |= TP_ZOOM_OUT;
    return b;
}

int main(int argc, char **argv) {
    (void)argc; (void)argv;
    consoleInit(NULL);
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    PadState pad;
    padInitializeDefault(&pad);
    puts(TITLE); puts(SCOPE); puts(EXPECT); puts(CONTROLS);
    puts("Input file:"); puts(FILE_PATH);
    struct tp_mesh_info info = {0};
    struct tp_vertex *vertices = NULL;
    int32_t status = load_mesh(&vertices, &info);
    const Result romfs = romfsInit();
    if (R_FAILED(romfs) || status != 0) {
        printf("Mesh status: %" PRId32 " | RomFS: 0x%08x\n", status, (unsigned)romfs);
        puts("Prepare and copy terrain-probe-mesh.bin with the NRO, then relaunch.");
        puts(EXIT_LABEL); wait_exit(&pad); free(vertices);
        if (R_SUCCEEDED(romfs)) romfsExit();
        consoleExit(NULL); return 1;
    }
    printf("Rust validated %" PRIu32 " vertices.\n", info.vertex_count);
    puts(START);
    bool start = false;
    while (appletMainLoop()) {
        padUpdate(&pad);
        const uint64_t down = padGetButtonsDown(&pad);
        if (down & HidNpadButton_Plus) break;
        if (down & HidNpadButton_A) { start = true; break; }
        consoleUpdate(NULL);
    }
    if (!start) { free(vertices); romfsExit(); consoleExit(NULL); return 0; }
    /* Console and deko3d never own the native window at the same time. */
    consoleExit(NULL);
    status = tp_graphics_initialize();
    const uint64_t gpu_bytes = tp_graphics_allocated_bytes();
    if (status == 0) status = tp_graphics_upload(vertices, info.vertex_count);
    free(vertices);
    vertices = NULL;
    struct tp_app_state state = {0};
    struct tp_app_frame frame = {0};
    const uint64_t first_tick = armGetSystemTick();
    uint64_t presented = 0;
    bool closing = false;
    while (status == 0) {
        if (!appletMainLoop()) { closing = true; break; }
        padUpdate(&pad);
        const HidAnalogStickState stick = padGetStickPos(&pad, 1);
        status = craftrustnx_terrain_update(&state, &info,
            armTicksToNs(armGetSystemTick() - first_tick),
            input(padGetButtonsDown(&pad), padGetButtons(&pad)),
            stick.x / 32767.0f, stick.y / 32767.0f, &frame);
        if (status != 0 || frame.should_exit) break;
        status = tp_graphics_present(frame.mvp, frame.clear);
        if (status == 0) ++presented;
    }
    tp_graphics_shutdown();
    romfsExit();
    if (!closing) {
        consoleInit(NULL); puts(TITLE);
        printf("Status: %" PRId32 " | submitted frames: %" PRIu64 "\n", status, presented);
        printf("Vertices: %" PRIu32 " | GPU memory blocks: %" PRIu64 " bytes\n", info.vertex_count, gpu_bytes);
        puts("Check visible shapes/depth and each camera control separately.");
        puts(SCOPE); puts(EXIT_LABEL); wait_exit(&pad); consoleExit(NULL);
    }
    return status == 0 ? 0 : 1;
}
