#include <inttypes.h>
#include <stdio.h>
#include <sys/stat.h>
#include <switch.h>

#ifdef CRAFTRUSTNX_RUST_PROBE
extern uint64_t craftrustnx_rust_mix(uint64_t input);

static uint64_t c_reference(uint64_t input)
{
    const uint64_t product = input * UINT64_C(0x9e3779b97f4a7c15);
    return ((product << 17) | (product >> 47)) ^ UINT64_C(0x4352414654525553);
}

static unsigned rust_self_test(void)
{
    const uint64_t inputs[] = {0, 1, UINT64_MAX, UINT64_C(0x8000000000000000)};
    const uint64_t expected[] = {
        UINT64_C(0x4352414654525553), UINT64_C(0xb020bfd2ac79693d),
        UINT64_C(0x4fdf402d538496c2), UINT64_C(0x4352414654535553)
    };
    unsigned failures = 0;
    for (unsigned i = 0; i < sizeof(inputs) / sizeof(inputs[0]); ++i)
        if (craftrustnx_rust_mix(inputs[i]) != expected[i])
            ++failures;
    return failures;
}
#endif

/* Platform bring-up only. No Horizon engine or Minecraft assets are linked. */
int main(void)
{
    consoleInit(NULL);
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    PadState pad;
    padInitializeDefault(&pad);

    struct stat sd_stat;
    const bool sd_available = stat("sdmc:/", &sd_stat) == 0 && S_ISDIR(sd_stat.st_mode);
    uint64_t frames = 0;
    uint64_t seen = 0;
    bool green = false;
#ifdef CRAFTRUSTNX_RUST_PROBE
    unsigned rust_failures = rust_self_test();
    uint64_t rust_checks = 4;
#endif

#ifdef CRAFTRUSTNX_RUST_PROBE
    printf("\x1b[2J\x1b[1;1HCraftRustNX: Rust + libnx probe\n\n");
#else
    printf("\x1b[2J\x1b[1;1HCraftRustNX: Switch hardware probe\n\n");
#endif
    printf("Target: original Nintendo Switch family / Switch Lite\n");
    printf("Framebuffer text console; game and GPU renderer not loaded.\n\n");
    printf("A: toggle color | B: clear seen buttons | Plus: exit\n");
    printf("Move both sticks and press each button to check input.\n");
    printf("SD root readable: %s (no files written)\n", sd_available ? "yes" : "no");

    while (appletMainLoop()) {
        padUpdate(&pad);
        const uint64_t down = padGetButtonsDown(&pad);
        const uint64_t held = padGetButtons(&pad);
        if (down & HidNpadButton_Plus)
            break;
        if (down & HidNpadButton_A)
            green = !green;
        seen |= held;
        if (down & HidNpadButton_B)
            seen = 0;

        const HidAnalogStickState left = padGetStickPos(&pad, 0);
        const HidAnalogStickState right = padGetStickPos(&pad, 1);
        printf("\x1b[10;1H%s", green ? CONSOLE_GREEN : CONSOLE_WHITE);
        printf("Frames: %12" PRIu64 "\x1b[K\n", ++frames);
        printf("Controller connected: %s\x1b[K\n", padIsConnected(&pad) ? "yes" : "no");
        printf("Handheld input:       %s\x1b[K\n", padIsHandheld(&pad) ? "yes" : "no");
        printf("Held buttons: 0x%016" PRIx64 "\x1b[K\n", held);
        printf("Seen buttons: 0x%016" PRIx64 "\x1b[K\n", seen);
        printf("Left stick:  %7" PRId32 " %7" PRId32 "\x1b[K\n", left.x, left.y);
        printf("Right stick: %7" PRId32 " %7" PRId32 "\x1b[K\n", right.x, right.y);
#ifdef CRAFTRUSTNX_RUST_PROBE
        const uint64_t input = frames ^ held ^ ((uint64_t)(uint32_t)left.x << 32)
            ^ (uint32_t)right.y;
        const uint64_t actual = craftrustnx_rust_mix(input);
        ++rust_checks;
        if (actual != c_reference(input))
            ++rust_failures;
        printf("Rust ABI: %s | checks: %12" PRIu64 "\x1b[K\n",
               rust_failures ? "FAIL" : "PASS", rust_checks);
        printf("Rust result: %016" PRIx64 " | errors: %u\x1b[K\n", actual, rust_failures);
        printf("no_std/core only; files/threads/network not tested.\x1b[K\n");
#endif
        printf(CONSOLE_RESET);
        consoleUpdate(NULL);
    }

    consoleExit(NULL);
    return 0;
}
