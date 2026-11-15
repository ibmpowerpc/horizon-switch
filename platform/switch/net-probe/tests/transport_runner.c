#include "net_probe.h"
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static struct np_job job;
static void *run(void *unused) {
    (void)unused;
    np_run(&job);
    return NULL;
}

int main(int argc, char **argv) {
    if (argc != 4 || strlen(argv[1]) >= sizeof(job.host)) return 2;
    strcpy(job.host, argv[1]);
    job.port = (uint16_t)strtoul(argv[2], NULL, 10);
    job.protocol = 777;
    atomic_init(&job.stage, NP_IDLE);
    atomic_init(&job.cancel, strcmp(argv[3], "cancel-before") == 0);
    if (strcmp(argv[3], "cancel-live") == 0) {
        pthread_t worker;
        if (pthread_create(&worker, NULL, run, NULL)) return 3;
        struct timespec delay = {0, 150000000};
        nanosleep(&delay, NULL);
        atomic_store(&job.cancel, true);
        if (pthread_join(worker, NULL)) return 4;
    } else {
        np_run(&job);
    }
    printf("%d %d %d %d %d %u %u %u\n", atomic_load(&job.stage),
        job.error, job.system_error, job.resolver_error, job.codec_error,
        job.response_length, job.json_offset, job.json_length);
    if (job.json_offset > sizeof(job.response) ||
        job.json_length > sizeof(job.response) - job.json_offset) return 5;
    for (uint32_t i = 0; i < job.json_length; ++i)
        printf("%02x", job.response[job.json_offset + i]);
    putchar('\n');
    return 0;
}
