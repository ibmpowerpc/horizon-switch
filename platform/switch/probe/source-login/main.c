/* Native Switch diagnostic console and responsive worker lifecycle.
 * Result fields are read only after acquiring a terminal stage. Join before
 * reuse or shutdown. Server-controlled strings are bounded and sanitized.
 * Centralized diagnostic labels can be localized with the future game UI.
 */
#include "login_probe.h"
#include "probe_defaults.h"
#ifdef CRAFTRUSTNX_COLUMN_EXPORT
#include "column_store.h"
#endif
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <switch.h>

// UI diagnostic labels (centralized for later localization).
#if defined(CRAFTRUSTNX_COLUMN_EXPORT)
static const char *LABEL_TITLE = "CraftRustNX: Column Export Probe 0.10.0";
#elif defined(CRAFTRUSTNX_COLUMN_SNAPSHOT)
static const char *LABEL_TITLE = "CraftRustNX: Column Snapshot Probe";
#elif defined(CRAFTRUSTNX_WORLD_DATA)
static const char *LABEL_TITLE = "CraftRustNX: Structured World Data Probe";
#elif defined(CRAFTRUSTNX_PLAY_SESSION)
static const char *LABEL_TITLE = "CraftRustNX: Play Session Probe";
#elif defined(CRAFTRUSTNX_CONFIGURATION_PROBE)
static const char *LABEL_TITLE = "CraftRustNX: Configuration to First Play Packet";
#elif defined(CRAFTRUSTNX_ENCRYPTED_LOGIN)
static const char *LABEL_TITLE = "CraftRustNX: Encrypted Offline Login Probe";
#else
static const char *LABEL_TITLE = "CraftRustNX: Java Server LOGIN Probe (Offline)";
#endif
#ifdef CRAFTRUSTNX_COLUMN_EXPORT
static const char *LABEL_CONTROLS = "X: host | Y: port | L: name | A: connect | B: cancel | R: save | Plus: exit";
static const char *LABEL_SAVE_READY = "R: save this column to SD (raw IDs, no lighting or models).";
static const char *LABEL_SAVE_OK = "Saved and read back: %s\n";
static const char *LABEL_SAVE_FAILED = "Save failed: %d | snapshot codec: %d\n";
#else
static const char *LABEL_CONTROLS = "X: host | Y: port | L: name | A: connect | B: cancel | Plus: exit";
#endif
static const char *LABEL_HOST = "Host:";
static const char *LABEL_PORT = "Port:";
static const char *LABEL_NAME = "Player:";
static const char *LABEL_STAGE = "Stage:";
static const char *LABEL_ERROR = "Error:";
static const char *LABEL_COMPRESSION = "Compression:";
static const char *LABEL_FRAMES = "Frames:";
static const char *LABEL_RECEIVED = "Received:";
static const char *LABEL_PROFILE_NAME = "Accepted player:";
static const char *LABEL_PROFILE_UUID = "Player UUID:";
static const char *LABEL_SESSION_UUID = "Session UUID:";
static const char *LABEL_EVENT_DATA = "Event data (sanitized):";
static const char *LABEL_NO_DATA = "(no event data)";
static const char *LABEL_SOCKETS_OK = "Sockets initialized: yes";
static const char *LABEL_SOCKETS_FAIL = "Sockets initialization FAILED";
static const char *LABEL_DNS_WAIT = "DNS may block; cancellation waits for getaddrinfo.";
static const char *LABEL_NO_CONFIG = "Diagnostic only. The Stage line shows the result. No gameplay.";
static const char *LABEL_AUTH_FLAG = "Server should_authenticate:";
static const char *LABEL_ENCRYPTION = "Encryption active:";
#ifdef CRAFTRUSTNX_CONFIGURATION_PROBE
static const char *LABEL_CONFIGURATION = "Configuration ACK: %s | packets: %" PRIu32 " | registries: %" PRIu32 "\n";
static const char *LABEL_DECODED = "Decoded configuration/play bytes: %" PRIu32 "\n";
static const char *LABEL_FIRST_PLAY = "First play packet: id=%" PRId32 " body=%" PRIu32 " bytes\n";
static const char *LABEL_PACK_DECLINED = "Resource pack declined%s.\n";
static const char *LABEL_PACK_REQUIRED = " (required by server)";
#ifdef CRAFTRUSTNX_WORLD_DATA
static const char *LABEL_OPAQUE = "Registry/chunk structure checked; no world loaded.";
#else
static const char *LABEL_OPAQUE = "Metadata is opaque. No registries or world loaded.";
#endif
#endif

#ifdef CRAFTRUSTNX_PLAY_SESSION
static const char *LABEL_PLAY_COUNTS = "Play packets: %" PRIu32 " | decoded bytes: %" PRIu32 "\n";
static const char *LABEL_PLAY_REPLIES = "Pose ACKs: %" PRIu32 " | keepalive: %" PRIu32 " | pong: %" PRIu32 "\n";
#ifdef CRAFTRUSTNX_WORLD_DATA
static const char *LABEL_CHUNKS = "Received chunks: %" PRIu32 " | batch ACKs: %" PRIu32 " | last: %" PRId32 ",%" PRId32 "\n";
#else
static const char *LABEL_CHUNKS = "Opaque chunks: %" PRIu32 " | batch ACKs: %" PRIu32 " | last: %" PRId32 ",%" PRId32 "\n";
#endif
static const char *LABEL_DIMENSION = "Dimension:";
static const char *LABEL_ENTITY = "Entity: %" PRId32 " | game mode: %" PRId32 " | last play ID: %" PRId32 "\n";
static const char *LABEL_POSE = "Position: %.3f %.3f %.3f | angles: %.1f %.1f\n";
#ifdef CRAFTRUSTNX_WORLD_DATA
static const char *LABEL_NOT_READY = "No terrain installed; PlayerLoaded was not sent.";
#else
static const char *LABEL_NOT_READY = "No terrain validated; PlayerLoaded was not sent.";
#endif
#endif

#ifdef CRAFTRUSTNX_WORLD_DATA
#ifdef CRAFTRUSTNX_COLUMN_SNAPSHOT
static const char *LABEL_COLUMN = "Snapshot: %" PRIu32 " sections | blocks: %" PRIu32 " | biomes: %" PRIu32 " | status: %" PRId32 "\n";
static const char *LABEL_COLUMN_MATCH = "Snapshot sample agrees: %s. No live updates or terrain renderer.";
#endif
static const char *LABEL_WORLD_ERROR = " (world codec: %" PRId32 ")";
static const char *LABEL_WORLD_REGISTRIES = "Dimension entries: %" PRIu32 " | biome entries: %" PRIu32 "\n";
static const char *LABEL_WORLD_SHAPE = "World min Y: %" PRId32 " | height: %" PRId32 " | skylight: %s\n";
static const char *LABEL_WORLD_CHUNKS = "Checked chunks: %" PRIu32 " | sections: %" PRIu32 " | heightmaps: %" PRIu32 "\n";
static const char *LABEL_WORLD_LIGHT = "Light updates: %" PRIu32 " | sky layers: %" PRIu32 " | block layers: %" PRIu32 "\n";
static const char *LABEL_WORLD_SAMPLE = "Block below pose (%" PRId32 ",%" PRId32 ",%" PRId32 "): state ID %" PRId32 " | biome ID %" PRId32 "\n";
static const char *LABEL_WORLD_NO_SAMPLE = "Matching chunk sample not received.";
#ifndef CRAFTRUSTNX_COLUMN_SNAPSHOT
static const char *LABEL_WORLD_LIMIT = "Raw IDs only; no block names, terrain cache or renderer.";
#endif
#endif

static const char *stage_name(enum lj_stage stage) {
    switch (stage) {
    case LJ_IDLE: return "IDLE";
    case LJ_RESOLVING: return "RESOLVING";
    case LJ_CONNECTING: return "CONNECTING";
    case LJ_SENDING: return "SENDING";
    case LJ_RECEIVING: return "RECEIVING";
    case LJ_KEY_EXCHANGE: return "KEY_EXCHANGE";
    case LJ_CONFIGURING: return "CONFIGURING";
    case LJ_PLAY_WAIT: return "WAITING_FOR_PLAY_PACKET";
    case LJ_PLAY_RECEIVING: return "OBSERVING_PLAY_SESSION";
    case LJ_PLAY_SESSION_OBSERVED: return "PLAY_SESSION_OBSERVED";
    case LJ_COLUMN_SNAPSHOT: return "COLUMN_SNAPSHOT_OBSERVED";
    case LJ_WORLD_DATA_OBSERVED: return "WORLD_DATA_OBSERVED";
    case LJ_PLAY_PACKET_SEEN: return "PLAY_PACKET_SEEN";
    case LJ_ACTION_REQUIRED: return "USER_ACTION_REQUIRED";
    case LJ_ACCEPTED: return "LOGIN_ACCEPTED";
    case LJ_DISCONNECTED: return "DISCONNECTED";
    case LJ_AUTH_REQUIRED: return "AUTH_OR_ENCRYPTION_REQUIRED";
    case LJ_UNSUPPORTED: return "UNSUPPORTED";
    case LJ_ERROR: return "ERROR";
    case LJ_CANCELLED: return "CANCELLED";
    default: return "UNKNOWN";
    }
}

static bool terminal_stage(int stage) {
    return stage == LJ_ACCEPTED || stage == LJ_DISCONNECTED ||
        stage == LJ_AUTH_REQUIRED || stage == LJ_UNSUPPORTED ||
        stage == LJ_ERROR || stage == LJ_CANCELLED ||
        stage == LJ_PLAY_PACKET_SEEN || stage == LJ_ACTION_REQUIRED ||
        stage == LJ_PLAY_SESSION_OBSERVED || stage == LJ_WORLD_DATA_OBSERVED || stage == LJ_COLUMN_SNAPSHOT;
}

static const char *error_name(enum lj_error error) {
    switch (error) {
    case LJ_ERR_NONE: return "NONE";
    case LJ_ERR_ARGUMENT: return "ARGUMENT";
    case LJ_ERR_DNS: return "DNS";
    case LJ_ERR_CONNECT: return "CONNECT";
    case LJ_ERR_TIMEOUT: return "TIMEOUT";
    case LJ_ERR_SEND: return "SEND";
    case LJ_ERR_RECV: return "RECV";
    case LJ_ERR_EOF: return "EOF";
    case LJ_ERR_PROTOCOL: return "PROTOCOL";
    case LJ_ERR_CLOCK: return "CLOCK";
    case LJ_ERR_CRYPTO: return "CRYPTO";
    case LJ_ERR_MEMORY: return "MEMORY";
    case LJ_ERR_LIMIT: return "SESSION_LIMIT";
    default: return "UNKNOWN";
    }
}

// Sanitize payload for display: replace control chars/non-ASCII with '?'.
// Truncate to max_display chars. No ANSI, no format string expansion.
static void sanitize_display(const uint8_t *data, uint32_t length, uint32_t max_display) {
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

// Format UUID as hex: 8-4-4-4-12.
static void format_uuid(const uint8_t uuid[16], char *out, size_t capacity) {
    if (capacity < 37) return;
    snprintf(out, capacity,
        "%02x%02x%02x%02x-%02x%02x-%02x%02x-%02x%02x-%02x%02x%02x%02x%02x%02x",
        uuid[0], uuid[1], uuid[2], uuid[3],
        uuid[4], uuid[5], uuid[6], uuid[7],
        uuid[8], uuid[9], uuid[10], uuid[11],
        uuid[12], uuid[13], uuid[14], uuid[15]);
}

// Worker thread entry: runs lj_run synchronously.
static void worker_thread(void *arg) {
    struct lj_job *job = (struct lj_job *)arg;
    lj_run(job);
}

int main(void) {
    consoleInit(NULL);
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    PadState pad;
    padInitializeDefault(&pad);

    // Initialize sockets once; display failure prominently.
    const Result sock_init = socketInitializeDefault();
    const bool sockets_ok = R_SUCCEEDED(sock_init);

    // Static job: host uses the shared editable default, port 25565, name CraftRustNX.
    /* Keep the large snapshot arrays in BSS, not zero-filled NRO data. */
    static struct lj_job job;
    strcpy(job.host, CRAFTRUSTNX_DEFAULT_HOST);
    strcpy(job.name, "CraftRustNX");
    job.port = 25565;
    job.compression = -1;
    atomic_init(&job.cancel, false);
    atomic_init(&job.stage, LJ_IDLE);

    Thread thread;
    bool thread_active = false;
    bool quitting = false;
    Result platform_error = 0;
#ifdef CRAFTRUSTNX_COLUMN_EXPORT
    char saved_path[512] = {0};
    bool save_attempted = false;
    int save_status = 0, save_codec_error = 0;
#endif

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
        bool is_terminal = terminal_stage(current_stage);
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
            char tmp_host[254] = {0};
            const Result rc = swkbdCreate(&kbd, 0);
            if (R_SUCCEEDED(rc)) {
                swkbdConfigMakePresetDefault(&kbd);
                swkbdConfigSetHeaderText(&kbd, "Enter server hostname or IP:");
                swkbdConfigSetInitialText(&kbd, job.host);
                swkbdConfigSetStringLenMax(&kbd, 253);
                platform_error = swkbdShow(&kbd, tmp_host, sizeof(tmp_host));
                if (R_SUCCEEDED(platform_error)) {
                    strncpy(job.host, tmp_host, 253);
                    job.host[253] = '\0';
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

        // L: edit player name (disabled while busy).
        if ((down & HidNpadButton_L) && !is_busy && !quitting && sockets_ok) {
            SwkbdConfig kbd;
            char tmp_name[17] = {0};
            const Result rc = swkbdCreate(&kbd, 0);
            if (R_SUCCEEDED(rc)) {
                swkbdConfigMakePresetDefault(&kbd);
                swkbdConfigSetHeaderText(&kbd, "Enter player name (1-16 chars):");
                swkbdConfigSetInitialText(&kbd, job.name);
                swkbdConfigSetStringLenMax(&kbd, 16);
                platform_error = swkbdShow(&kbd, tmp_name, sizeof(tmp_name));
                if (R_SUCCEEDED(platform_error) && tmp_name[0] != '\0') {
                    strncpy(job.name, tmp_name, 16);
                    job.name[16] = '\0';
                }
                swkbdClose(&kbd);
            } else platform_error = rc;
        }

        // A: connect (disabled if host empty, busy, or sockets failed).
        if ((down & HidNpadButton_A) && !is_busy && !quitting && sockets_ok && job.host[0] != '\0') {
#ifdef CRAFTRUSTNX_COLUMN_EXPORT
            saved_path[0] = 0;
            save_attempted = false;
#endif
            atomic_store(&job.cancel, false);
            atomic_store(&job.stage, LJ_IDLE);
            job.error = LJ_ERR_NONE;
            job.system_error = 0;
            job.resolver_error = 0;
            job.codec_error = 0;
            job.compression = -1;
            job.received_bytes = 0;
            job.frame_count = 0;
            memset(&job.event, 0, sizeof(job.event));
            memset(job.wire, 0, sizeof(job.wire));
            memset(job.scratch, 0, sizeof(job.scratch));
            // Reserve room for Rust's bounded zlib decoder state. Packet
            // buffers themselves live in static job storage, not this stack.
            const Result trc = threadCreate(&thread, worker_thread, &job, NULL, 0x20000, 0x2C, -2);
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
        is_terminal = terminal_stage(current_stage);

#ifdef CRAFTRUSTNX_COLUMN_EXPORT
        // Only the joined worker's immutable, validated result may be saved.
        if ((down & HidNpadButton_R) && !thread_active && !quitting &&
            current_stage == LJ_COLUMN_SNAPSHOT && job.column_error == COL_OK &&
            job.column_sample_match) {
            save_status = lj_store_column("sdmc:/switch/craftrustnx/captures", &job.column,
                job.column_blocks, job.column_biomes, saved_path, sizeof(saved_path),
                &save_codec_error);
            save_attempted = true;
        }
#endif

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
        printf("%s\n", LABEL_NO_CONFIG);
#ifdef CRAFTRUSTNX_ENCRYPTED_LOGIN
        printf("Offline attempt: no session join sent. Server decides acceptance.\n");
#endif
        printf("\n%s\n\n", LABEL_CONTROLS);
        if (quitting || atomic_load(&job.cancel)) printf("Cancellation requested; waiting for worker (DNS may block).\n");
        if (R_FAILED(platform_error)) printf("Platform operation failed: 0x%08" PRIx32 "\n", platform_error);

        printf("%s ", LABEL_HOST);
        if (job.host[0]) sanitize_display((const uint8_t *)job.host, (uint32_t)strlen(job.host), 253);
        else printf("(empty - press X)");
        printf("\x1b[K\n");
        printf("%s %u\x1b[K\n", LABEL_PORT, job.port);
        printf("%s ", LABEL_NAME);
        sanitize_display((const uint8_t *)job.name, (uint32_t)strlen(job.name), 16);
        printf("\x1b[K\n");
        printf("%s %s\x1b[K\n", LABEL_STAGE, stage_name(current_stage));
#ifdef CRAFTRUSTNX_COLUMN_EXPORT
        if (save_attempted) {
            if (save_status == 0) printf(LABEL_SAVE_OK, saved_path);
            else printf(LABEL_SAVE_FAILED, save_status, save_codec_error);
        } else if (!thread_active && current_stage == LJ_COLUMN_SNAPSHOT &&
            job.column_error == COL_OK && job.column_sample_match) puts(LABEL_SAVE_READY);
#endif

        if (is_terminal) {
            if (job.saw_encryption_request) {
                printf("%s %s\n", LABEL_AUTH_FLAG, job.server_requested_auth ? "true" : "false");
                printf("%s %s\n", LABEL_ENCRYPTION, job.encryption_active ? "yes" : "no");
            }
            printf("%s %s", LABEL_ERROR, error_name(job.error));
            if (job.error != LJ_ERR_NONE) {
                if (job.system_error != 0)
                    printf(" (sys: %d)", job.system_error);
                if (job.resolver_error != 0)
                    printf(" (dns: %d)", job.resolver_error);
                if (job.codec_error != 0)
                    printf(" (codec: %" PRId32 ")", job.codec_error);
                if (job.crypto_error != 0)
                    printf(" (crypto: %d, detail: 0x%08x)", job.crypto_error, (unsigned)job.crypto_detail);
            }
#ifdef CRAFTRUSTNX_WORLD_DATA
            if (job.world_error != WC_OK) printf(LABEL_WORLD_ERROR, job.world_error);
#endif
            printf("\x1b[K\n");

            printf("%s %" PRId32 "\x1b[K\n", LABEL_COMPRESSION, job.compression);
            printf("%s %" PRIu32 "\x1b[K\n", LABEL_FRAMES, job.frame_count);
            printf("%s %" PRIu32 " bytes\x1b[K\n", LABEL_RECEIVED, job.received_bytes);

#ifdef CRAFTRUSTNX_CONFIGURATION_PROBE
            printf(LABEL_CONFIGURATION,
                job.configuration_acknowledged ? "yes" : "no", job.configuration_frames, job.registry_frames);
            printf(LABEL_DECODED, job.configuration_bytes);
            if (current_stage == LJ_PLAY_PACKET_SEEN || job.play_frames > 0)
                printf(LABEL_FIRST_PLAY,
                    job.first_play_id, job.first_play_body_bytes);
            if (job.resource_pack_declined)
                printf(LABEL_PACK_DECLINED, job.required_pack_declined ? LABEL_PACK_REQUIRED : "");
            puts(LABEL_OPAQUE);
#endif

#ifdef CRAFTRUSTNX_PLAY_SESSION
            printf(LABEL_PLAY_COUNTS, job.play_frames, job.play_bytes);
            printf(LABEL_PLAY_REPLIES, job.teleports_acked, job.keepalives_replied, job.pongs_replied);
            printf(LABEL_CHUNKS, job.chunk_frames, job.chunk_batches, job.last_chunk_x, job.last_chunk_z);
            if (job.play.login_seen) {
                printf("%s ", LABEL_DIMENSION);
                sanitize_display((const uint8_t *)job.play.dimension,
                    (uint32_t)strnlen(job.play.dimension, sizeof(job.play.dimension)), 100);
                putchar('\n');
                printf(LABEL_ENTITY, job.play.player_id, job.play.game_mode, job.last_play_id);
            }
            if (job.play.position_seen) {
                double position[3];
                float yaw, pitch;
                memcpy(position, job.play.position_bits, sizeof(position));
                memcpy(&yaw, &job.play.yaw_bits, sizeof(yaw));
                memcpy(&pitch, &job.play.pitch_bits, sizeof(pitch));
                printf(LABEL_POSE, position[0], position[1], position[2], (double)yaw, (double)pitch);
            }
            puts(LABEL_NOT_READY);
#endif

#ifdef CRAFTRUSTNX_WORLD_DATA
            printf(LABEL_WORLD_REGISTRIES, job.world.dimension_count, job.world.biome_count);
            const int32_t type = job.play.dimension_type;
            if (job.play.login_seen && type >= 0 && type < WC_MAX_DIMENSIONS &&
                (uint32_t)type < job.world.dimension_count && job.world.dimensions[type].valid) {
                const struct wc_dimension *dimension = &job.world.dimensions[type];
                printf(LABEL_WORLD_SHAPE, dimension->min_y, dimension->height,
                    dimension->has_skylight ? "yes" : "no");
            }
            printf(LABEL_WORLD_CHUNKS, job.validated_chunks, job.validated_sections, job.validated_heightmaps);
            printf(LABEL_WORLD_LIGHT, job.validated_light_updates, job.validated_sky_layers, job.validated_block_layers);
            if (job.world_sample.sample_valid) {
                const struct wc_event *sample = &job.world_sample;
                printf(LABEL_WORLD_SAMPLE, sample->sample_x, sample->sample_y, sample->sample_z,
                    sample->state_id, sample->biome_id);
            } else puts(LABEL_WORLD_NO_SAMPLE);
#ifdef CRAFTRUSTNX_COLUMN_SNAPSHOT
            printf(LABEL_COLUMN, job.column.sections, job.column.blocks_written,
                job.column.biomes_written, job.column_error);
            printf(LABEL_COLUMN_MATCH, job.column_sample_match ? "yes" : "no");
            putchar('\n');
#else
            puts(LABEL_WORLD_LIMIT);
#endif
#endif

            if (job.login_accepted) {
                char uuid_str[37];
                printf("\n%s ", LABEL_PROFILE_NAME);
                sanitize_display((const uint8_t *)job.event.name, (uint32_t)strlen(job.event.name), 16);
                printf("\x1b[K\n");
                format_uuid(job.event.uuid, uuid_str, sizeof(uuid_str));
                printf("%s %s\x1b[K\n", LABEL_PROFILE_UUID, uuid_str);
                format_uuid(job.event.session, uuid_str, sizeof(uuid_str));
                printf("%s %s\x1b[K\n", LABEL_SESSION_UUID, uuid_str);
            }

            if ((current_stage == LJ_DISCONNECTED || current_stage == LJ_UNSUPPORTED || current_stage == LJ_ACTION_REQUIRED) &&
                job.event.data_length > 0 && job.event.data_offset < LP_MAX_BODY) {
                printf("\n%s\n", LABEL_EVENT_DATA);
                const uint32_t safe_len = (job.event.data_length <= LP_MAX_BODY - job.event.data_offset)
                    ? job.event.data_length : (LP_MAX_BODY - job.event.data_offset);
                sanitize_display(job.scratch + job.event.data_offset, safe_len, 600);
            } else if (job.event.data_length == 0 && (current_stage == LJ_DISCONNECTED || current_stage == LJ_UNSUPPORTED || current_stage == LJ_ACTION_REQUIRED)) {
                printf("\n%s\n", LABEL_NO_DATA);
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
