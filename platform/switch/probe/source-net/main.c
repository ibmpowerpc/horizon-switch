/* Native Nintendo Switch Java server STATUS probe: UI and worker thread.
 * libnx console/pad/swkb with static job and native Thread. Sockets initialized
 * once; all failures visible/released. Host uses the shared editable default, port 25565,
 * protocol 777. X edit host, Y port, A connect explicitly, B cancel, Plus exit.
 * UI stays responsive; edits/connect disabled while busy. Join/close thread
 * before reuse/exit. Show stages/errors/count and bounded sanitized raw JSON.
 * No server-controlled format strings/ANSI/control codes. No JSON parsing claim,
 * no files/credentials/login. Diagnostic labels centralized for localization.
 */
#include "net_probe.h"
#include "probe_defaults.h"
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <switch.h>

// UI diagnostic labels (centralized for later localization).
static const char *LABEL_TITLE = "CraftRustNX: Java Server STATUS Probe";
static const char *LABEL_CONTROLS = "X: edit host | Y: edit port | A: connect | B: cancel | Plus: exit";
static const char *LABEL_HOST = "Host:";
static const char *LABEL_PORT = "Port:";
static const char *LABEL_PROTOCOL = "Protocol:";
static const char *LABEL_STAGE = "Stage:";
static const char *LABEL_ERROR = "Error:";
static const char *LABEL_RESPONSE_LEN = "Response length:";
static const char *LABEL_JSON_RANGE = "JSON range:";
static const char *LABEL_RAW_DATA = "Raw JSON data (sanitized):";
static const char *LABEL_NO_RESPONSE = "(no response yet)";
static const char *LABEL_SOCKETS_OK = "Sockets initialized: yes";
static const char *LABEL_SOCKETS_FAIL = "Sockets initialization FAILED";
static const char *LABEL_DNS_WAIT = "DNS may block; cancellation waits for getaddrinfo.";
static const char *LABEL_NO_MOTD = "No MOTD parsing; this tests framing/native sockets.";

static const char *stage_name(enum np_stage stage) {
    switch (stage) {
    case NP_IDLE: return "IDLE";
    case NP_RESOLVING: return "RESOLVING";
    case NP_CONNECTING: return "CONNECTING";
    case NP_SENDING: return "SENDING";
    case NP_RECEIVING: return "RECEIVING";
    case NP_DONE: return "DONE";
    case NP_ERROR: return "ERROR";
    case NP_CANCELLED: return "CANCELLED";
    default: return "UNKNOWN";
    }
}

static const char *error_name(enum np_error error) {
    switch (error) {
    case NP_ERR_NONE: return "NONE";
    case NP_ERR_ARGUMENT: return "ARGUMENT";
    case NP_ERR_DNS: return "DNS";
    case NP_ERR_CONNECT: return "CONNECT";
    case NP_ERR_TIMEOUT: return "TIMEOUT";
    case NP_ERR_SEND: return "SEND";
    case NP_ERR_RECV: return "RECV";
    case NP_ERR_EOF: return "EOF";
    case NP_ERR_PROTOCOL: return "PROTOCOL";
    case NP_ERR_CLOCK: return "CLOCK";
    default: return "UNKNOWN";
    }
}

// Sanitize JSON for display: replace control chars/non-ASCII with '?'.
// Truncate to max_display chars. No ANSI, no format string expansion.
static void sanitize_json(const uint8_t *data, uint32_t length, uint32_t max_display) {
    const uint32_t display_len = length < max_display ? length : max_display;
    for (uint32_t i = 0; i < display_len; ++i) {
        const uint8_t c = data[i];
        if (c < 0x20 || c >= 0x7F)
            putchar('?');
        else
            putchar((int)c);
    }
    if (length > max_display)
        printf("... (truncated)");
}

// Worker thread entry: runs np_run synchronously.
static void worker_thread(void *arg) {
    struct np_job *job = (struct np_job *)arg;
    np_run(job);
}

int main(void) {
    consoleInit(NULL);
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    PadState pad;
    padInitializeDefault(&pad);

    // Initialize sockets once; display failure prominently.
    const Result sock_init = socketInitializeDefault();
    const bool sockets_ok = R_SUCCEEDED(sock_init);

    // Static job: host uses the shared editable default, port 25565, protocol 777.
    static struct np_job job = {
        .host = CRAFTRUSTNX_DEFAULT_HOST,
        .port = 25565,
        .protocol = 777,
        .cancel = false,
        .stage = NP_IDLE,
        .error = NP_ERR_NONE,
        .system_error = 0,
        .resolver_error = 0,
        .codec_error = 0,
        .response_length = 0,
        .json_offset = 0,
        .json_length = 0,
    };

    Thread thread;
    bool thread_active = false;
    bool quitting = false;
    Result platform_error = 0;

    while (appletMainLoop()) {
        padUpdate(&pad);
        const uint64_t down = padGetButtonsDown(&pad);

        // Exit on Plus.
        if (down & HidNpadButton_Plus) {
            quitting = true;
            if (thread_active) {
                atomic_store(&job.cancel, true);
            }
        }

        // Acquire stage with acquire semantics to see result fields.
        int current_stage = atomic_load_explicit(&job.stage, memory_order_acquire);
        bool is_terminal = (current_stage == NP_DONE || current_stage == NP_ERROR || current_stage == NP_CANCELLED);
        const bool is_busy = thread_active && !is_terminal;

        // Join thread when terminal stage reached.
        if (thread_active && is_terminal) {
            if (R_FAILED(threadWaitForExit(&thread))) abort();
            platform_error = threadClose(&thread);
            thread_active = false;
        }
        if (quitting && !thread_active) break;

        // X: edit host (disabled while busy).
        if ((down & HidNpadButton_X) && !is_busy && !quitting && sockets_ok) {
            SwkbdConfig kbd;
            char tmp_host[NP_HOST_CAPACITY] = {0};
            const Result rc = swkbdCreate(&kbd, 0);
            if (R_SUCCEEDED(rc)) {
                swkbdConfigMakePresetDefault(&kbd);
                swkbdConfigSetHeaderText(&kbd, "Enter server hostname or IP:");
                swkbdConfigSetInitialText(&kbd, job.host);
                swkbdConfigSetStringLenMax(&kbd, NP_HOST_CAPACITY - 1);
                platform_error = swkbdShow(&kbd, tmp_host, sizeof(tmp_host));
                if (R_SUCCEEDED(platform_error)) {
                    strncpy(job.host, tmp_host, NP_HOST_CAPACITY - 1);
                    job.host[NP_HOST_CAPACITY - 1] = '\0';
                }
                swkbdClose(&kbd);
            } else platform_error = rc;
        }

        // Y: edit port (disabled while busy).
        if ((down & HidNpadButton_Y) && !is_busy && !quitting && sockets_ok) {
            SwkbdConfig kbd;
            char tmp_port[16] = {0};
            const Result rc = swkbdCreate(&kbd, 0);
            if (R_SUCCEEDED(rc)) {
                swkbdConfigMakePresetDefault(&kbd);
                swkbdConfigSetHeaderText(&kbd, "Enter port (1-65535):");
                snprintf(tmp_port, sizeof(tmp_port), "%u", job.port);
                swkbdConfigSetInitialText(&kbd, tmp_port);
                swkbdConfigSetStringLenMax(&kbd, 5);
                platform_error = swkbdShow(&kbd, tmp_port, sizeof(tmp_port));
                if (R_SUCCEEDED(platform_error)) {
                    char *end = NULL;
                    const unsigned long port_val = strtoul(tmp_port, &end, 10);
                    if (tmp_port[0] >= '0' && tmp_port[0] <= '9' && end != tmp_port && *end == '\0' && port_val > 0 && port_val <= 65535)
                        job.port = (uint16_t)port_val;
                }
                swkbdClose(&kbd);
            } else platform_error = rc;
        }

        // A: connect (disabled if host empty, busy, or sockets failed).
        if ((down & HidNpadButton_A) && !is_busy && !quitting && sockets_ok && job.host[0] != '\0') {
            atomic_store(&job.cancel, false);
            atomic_store(&job.stage, NP_IDLE);
            job.error = NP_ERR_NONE;
            job.system_error = 0;
            job.resolver_error = 0;
            job.codec_error = 0;
            job.response_length = 0;
            job.json_offset = 0;
            job.json_length = 0;
            memset(job.response, 0, sizeof(job.response));
            const Result trc = threadCreate(&thread, worker_thread, &job, NULL, 0x10000, 0x2C, -2);
            platform_error = trc;
            if (R_SUCCEEDED(trc)) {
                platform_error = threadStart(&thread);
                if (R_SUCCEEDED(platform_error)) thread_active = true;
                else threadClose(&thread);
            }
        }

        // Events may have started a new worker. Never display a prior terminal
        // snapshot while that worker is resetting/writing the result fields.
        current_stage = atomic_load_explicit(&job.stage, memory_order_acquire);
        is_terminal = current_stage == NP_DONE || current_stage == NP_ERROR || current_stage == NP_CANCELLED;

        // B: cancel (only if thread active and not terminal).
        if ((down & HidNpadButton_B) && thread_active && !is_terminal) {
            atomic_store(&job.cancel, true);
        }

        // Display UI.
        printf("\x1b[2J\x1b[1;1H%s\n\n", LABEL_TITLE);
        if (sockets_ok)
            printf("%s\n", LABEL_SOCKETS_OK);
        else
            printf("%s (Result: 0x%08" PRIx32 ")\n", LABEL_SOCKETS_FAIL, sock_init);
        printf("%s\n", LABEL_DNS_WAIT);
        printf("%s\n", LABEL_NO_MOTD);
        printf("\n%s\n\n", LABEL_CONTROLS);
        if (quitting || atomic_load(&job.cancel)) printf("Cancellation requested; waiting for worker (DNS may block).\n");
        if (R_FAILED(platform_error)) printf("Platform operation failed: 0x%08" PRIx32 "\n", platform_error);

        printf("%s ", LABEL_HOST);
        if (job.host[0]) sanitize_json((const uint8_t *)job.host, (uint32_t)strlen(job.host), 253);
        else printf("(empty - press X)");
        printf("\x1b[K\n");
        printf("%s %u\x1b[K\n", LABEL_PORT, job.port);
        printf("%s %" PRId32 "\x1b[K\n", LABEL_PROTOCOL, job.protocol);
        printf("%s %s\x1b[K\n", LABEL_STAGE, stage_name(current_stage));

        if (is_terminal) {
            printf("%s %s", LABEL_ERROR, error_name(job.error));
            if (job.error != NP_ERR_NONE) {
                if (job.system_error != 0)
                    printf(" (sys: %d)", job.system_error);
                if (job.resolver_error != 0)
                    printf(" (dns: %d)", job.resolver_error);
                if (job.codec_error != 0)
                    printf(" (codec: %" PRId32 ")", job.codec_error);
            }
            printf("\x1b[K\n");

            if (current_stage == NP_DONE) {
                printf("%s %" PRIu32 "\x1b[K\n", LABEL_RESPONSE_LEN, job.response_length);
                printf("%s [%" PRIu32 ", %" PRIu32 ")\x1b[K\n", LABEL_JSON_RANGE,
                       job.json_offset, job.json_offset + job.json_length);
                printf("\n%s\n", LABEL_RAW_DATA);
                if (job.json_length > 0 && job.json_offset < job.response_length) {
                    const uint32_t safe_len = (job.json_offset + job.json_length <= job.response_length)
                        ? job.json_length : (job.response_length - job.json_offset);
                    sanitize_json(job.response + job.json_offset, safe_len, 800);
                } else {
                    printf("%s", LABEL_NO_RESPONSE);
                }
            }
        }

        consoleUpdate(NULL);
    }

    // appletMainLoop may end without a Plus event. The job outlives its worker.
    if (thread_active) {
        atomic_store(&job.cancel, true);
        if (R_FAILED(threadWaitForExit(&thread))) abort();
        threadClose(&thread);
    }
    if (sockets_ok)
        socketExit();
    consoleExit(NULL);
    return 0;
}
