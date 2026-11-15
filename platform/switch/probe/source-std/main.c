#include <inttypes.h>
#include <stdio.h>
#include <switch.h>

extern uint32_t craftrustnx_std_probe_abi_version(void);
extern uint32_t craftrustnx_std_probe_checks(void);
extern uint64_t craftrustnx_std_probe_run(uint32_t requested);

enum { CHECK_COUNT = 6 };
static const char *const names[CHECK_COUNT] = {
    "ALLOC: Vec / String", "TIME: monotonic / wall clock", "TLS: main thread",
    "THREAD: spawn / join", "SYNC: mutex / condition", "TLS: worker destructor"
};
static const char *const states[] = {"NOT RUN", "PASS", "FAIL", "INVALID RESULT"};

int main(void)
{
    consoleInit(NULL);
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    PadState pad;
    padInitializeDefault(&pad);
    unsigned selected = 0;
    unsigned results[CHECK_COUNT] = {0};
    uint64_t last_result = 0;
    const bool abi_ok = craftrustnx_std_probe_abi_version() == 1
        && craftrustnx_std_probe_checks() == 0x3f;

    while (appletMainLoop()) {
        padUpdate(&pad);
        const u64 down = padGetButtonsDown(&pad);
        if (down & HidNpadButton_Plus)
            break;
        if (down & HidNpadButton_Down)
            selected = (selected + 1) % CHECK_COUNT;
        if (down & HidNpadButton_Up)
            selected = (selected + CHECK_COUNT - 1) % CHECK_COUNT;

        printf("\x1b[1;1HCraftRustNX: Rust std runtime probe\x1b[K\n\n");
        printf("Up/Down: select | A: run one check | Plus: exit\x1b[K\n");
        printf("Start with ALLOC. Run the other checks one at a time.\x1b[K\n");
        printf("ABI: %s | Not the Horizon game client.\x1b[K\n\n", abi_ok ? "OK" : "ERROR");
        for (unsigned i = 0; i < CHECK_COUNT; ++i)
            printf("%c %-32s %s\x1b[K\n", selected == i ? '>' : ' ', names[i], states[results[i]]);
        printf("\nLast result: %016" PRIx64 "\x1b[K\n", last_result);
        printf("Files, network, rendering and gameplay are not tested.\x1b[K\n");
        printf("\x1b[17;1HReady.\x1b[K");

        if (abi_ok && (down & HidNpadButton_A)) {
            // Flush the selected name before entering unproven runtime code.
            // An abort/hang can then be attributed to this individual check.
            printf("\x1b[17;1HRUNNING: %s\x1b[K", names[selected]);
            consoleUpdate(NULL);
            const uint32_t requested = UINT32_C(1) << selected;
            last_result = craftrustnx_std_probe_run(requested);
            const uint32_t echoed = last_result & 0xffff;
            const uint32_t passed = (last_result >> 16) & 0xffff;
            const uint32_t failed = (last_result >> 32) & 0xffff;
            const bool valid = (last_result >> 48) == 0x5354
                && echoed == requested && (passed | failed) == requested
                && (passed & failed) == 0;
            results[selected] = !valid ? 3 : passed == requested ? 1 : 2;
        }
        consoleUpdate(NULL);
    }
    consoleExit(NULL);
    return 0;
}
