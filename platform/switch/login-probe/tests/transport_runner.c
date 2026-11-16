#include "login_probe.h"
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

_Static_assert(sizeof(struct pl_state) == 320, "play state ABI");
_Static_assert(sizeof(struct pl_event) == 32, "play event ABI");
static struct lj_job job;
static void *run(void *unused) {
    (void)unused;
    lj_run(&job);
    return NULL;
}
int main(int argc, char **argv) {
    if (argc != 4 || strlen(argv[1]) >= sizeof(job.host)) return 2;
    strcpy(job.host, argv[1]);
    strcpy(job.name, "CraftRustNX");
    job.port = (uint16_t)strtoul(argv[2], NULL, 10);
    atomic_init(&job.stage, LJ_IDLE);
    atomic_init(&job.cancel, strcmp(argv[3], "cancel-before") == 0);
    if (strcmp(argv[3], "cancel-live") == 0 || strcmp(argv[3], "cancel-configuration") == 0 || strcmp(argv[3], "cancel-play") == 0) {
        pthread_t worker;
        if (pthread_create(&worker, NULL, run, NULL)) return 3;
        if (strcmp(argv[3], "cancel-configuration") == 0 || strcmp(argv[3], "cancel-play") == 0) {
            // Deterministic test synchronization: do not race the RSA exchange.
            struct timespec tick = {0, 1000000};
            for (int tries = 0; tries < 2000; ++tries) {
                const int expected = strcmp(argv[3], "cancel-play") == 0 ? LJ_PLAY_RECEIVING : LJ_CONFIGURING;
                if (atomic_load_explicit(&job.stage, memory_order_acquire) == expected) break;
                nanosleep(&tick, NULL);
            }
        } else {
            struct timespec delay = {0, 150000000};
            nanosleep(&delay, NULL);
        }
        atomic_store(&job.cancel, true);
        if (pthread_join(worker, NULL)) return 4;
    } else lj_run(&job);
    printf("%d %d %d %d %d %d %u %u %u %u %d %d %u %u %u %u %u %d %u %u %u %d %u %u %u %u %u %u %u %d %u %u %d %d %d %d %d\n", atomic_load(&job.stage), job.error,
        job.system_error, job.resolver_error, job.codec_error, job.compression,
        job.received_bytes, job.frame_count, job.server_requested_auth,
        job.encryption_active, job.crypto_error, job.crypto_detail,
        job.login_accepted, job.configuration_acknowledged, job.configuration_frames,
        job.configuration_bytes, job.registry_frames, job.first_play_id,
        job.first_play_body_bytes, job.resource_pack_declined, job.required_pack_declined,
        job.last_configuration_id, job.play_frames, job.play_bytes,
        job.teleports_acked, job.keepalives_replied, job.pongs_replied, job.chunk_frames,
        job.chunk_batches, job.last_play_id, job.play.login_seen, job.play.position_seen,
        job.play.player_id, job.play.game_mode, job.play.teleport_id, job.last_chunk_x, job.last_chunk_z);
    for (size_t i = 0; i < sizeof(job.event.uuid); ++i) printf("%02x", job.event.uuid[i]);
    putchar('\n');
    for (size_t i = 0; i < sizeof(job.event.session); ++i) printf("%02x", job.event.session[i]);
    putchar('\n');
    printf("%.16s\n", job.event.name);
    printf("%.256s\n", job.play.dimension);
    double position[3];
    float yaw, pitch;
    memcpy(position, job.play.position_bits, sizeof(position));
    memcpy(&yaw, &job.play.yaw_bits, sizeof(yaw));
    memcpy(&pitch, &job.play.pitch_bits, sizeof(pitch));
    printf("%a %a %a %a %a\n", position[0], position[1], position[2], (double)yaw, (double)pitch);
    return 0;
}
