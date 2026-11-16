/* Platform lifecycle and SDK I/O only; Rust owns frame geometry and controls. */
#include "gpu_probe.h"
#include <inttypes.h>
#include <stddef.h>
#include <stdio.h>
#include <switch.h>

_Static_assert(sizeof(struct gp_state) == 32, "Rust state ABI");
_Static_assert(sizeof(struct gp_frame) == 128, "Rust frame ABI");
_Static_assert(offsetof(struct gp_frame, positions) == 16, "positions ABI");
_Static_assert(offsetof(struct gp_frame, colors) == 64, "colors ABI");

/* Centralized diagnostic labels; the eventual game UI uses translation keys. */
static const char *LABEL_TITLE = "CraftRustNX: Rust GPU Probe 0.6.0";
static const char *LABEL_START = "A: start GPU test | Plus: exit";
static const char *LABEL_CONTROLS = "During graphics: A pause | B palette | X reset | Plus summary";
static const char *LABEL_EXPECTED = "Expect a moving colored triangle on a dark background.";
static const char *LABEL_SCOPE = "Native deko3d smoke test. No Minecraft world or performance claim.";
static const char *LABEL_ROMFS = "RomFS initialization error: 0x%08x\n";
static const char *LABEL_RESULT = "SDK/Rust status: %" PRId32 " | submitted frames: %" PRIu64 "\n";
static const char *LABEL_MEMORY = "GPU memory blocks: %" PRIu64 " bytes\n";
static const char *LABEL_MANUAL = "Confirm visible animation, pause, palette and reset separately.";
static const char *LABEL_EXIT = "Plus: return to hbmenu";

static uint32_t buttons(uint64_t down) {
    uint32_t out = 0;
    if (down & HidNpadButton_A) out |= GP_PAUSE;
    if (down & HidNpadButton_B) out |= GP_PALETTE;
    if (down & HidNpadButton_X) out |= GP_RESET;
    if (down & HidNpadButton_Plus) out |= GP_EXIT;
    return out;
}

static void wait_exit(PadState *pad) {
    while (appletMainLoop()) {
        padUpdate(pad);
        if (padGetButtonsDown(pad) & HidNpadButton_Plus) break;
        consoleUpdate(NULL);
    }
}

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;
    consoleInit(NULL);
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    PadState pad;
    padInitializeDefault(&pad);
    puts(LABEL_TITLE);
    puts(LABEL_EXPECTED);
    puts(LABEL_CONTROLS);
    puts(LABEL_SCOPE);
    puts(LABEL_START);
    const Result romfs = romfsInit();
    if (R_FAILED(romfs)) {
        printf(LABEL_ROMFS, (unsigned)romfs);
        puts(LABEL_EXIT);
        wait_exit(&pad);
        consoleExit(NULL);
        return 1;
    }
    bool start = false;
    while (appletMainLoop()) {
        padUpdate(&pad);
        const uint64_t down = padGetButtonsDown(&pad);
        if (down & HidNpadButton_Plus) break;
        if (down & HidNpadButton_A) { start = true; break; }
        consoleUpdate(NULL);
    }
    if (!start) {
        romfsExit();
        consoleExit(NULL);
        return 0;
    }
    /* libnx console and deko3d must never own the native window together. */
    consoleExit(NULL);
    int32_t status = gp_graphics_initialize();
    const uint64_t gpu_bytes = gp_graphics_allocated_bytes();
    struct gp_state state = {0};
    struct gp_frame frame = {0};
    const uint64_t tick_start = armGetSystemTick();
    uint64_t presented = 0;
    bool app_closing = false;
    while (status == 0) {
        if (!appletMainLoop()) { app_closing = true; break; }
        padUpdate(&pad);
        const uint64_t ns = armTicksToNs(armGetSystemTick() - tick_start);
        status = craftrustnx_gpu_update(&state, ns,
            buttons(padGetButtonsDown(&pad)), &frame);
        if (status != 0 || frame.should_exit) break;
        status = gp_graphics_present(&frame);
        if (status == 0) ++presented;
    }
    gp_graphics_shutdown();
    romfsExit();
    if (!app_closing) {
        consoleInit(NULL);
        puts(LABEL_TITLE);
        printf(LABEL_RESULT, status, presented);
        printf(LABEL_MEMORY, gpu_bytes);
        puts(LABEL_MANUAL);
        puts(LABEL_SCOPE);
        puts(LABEL_EXIT);
        wait_exit(&pad);
        consoleExit(NULL);
    }
    return status == 0 ? 0 : 1;
}
