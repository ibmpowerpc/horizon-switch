#include "login_probe.h"
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

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
    if (strcmp(argv[3], "cancel-live") == 0) {
        pthread_t worker;
        if (pthread_create(&worker, NULL, run, NULL)) return 3;
        struct timespec delay = {0, 150000000};
        nanosleep(&delay, NULL);
        atomic_store(&job.cancel, true);
        if (pthread_join(worker, NULL)) return 4;
    } else lj_run(&job);
    printf("%d %d %d %d %d %d %u %u %u\n", atomic_load(&job.stage), job.error,
        job.system_error, job.resolver_error, job.codec_error, job.compression,
        job.received_bytes, job.frame_count, job.event.should_authenticate);
    for (size_t i = 0; i < sizeof(job.event.uuid); ++i) printf("%02x", job.event.uuid[i]);
    putchar('\n');
    for (size_t i = 0; i < sizeof(job.event.session); ++i) printf("%02x", job.event.session[i]);
    putchar('\n');
    printf("%.16s\n", job.event.name);
    return 0;
}
