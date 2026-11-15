/* POSIX C socket transport for Minecraft Java Edition server STATUS ping.
 * No libnx, no switch.h, no threading or service calls: macOS/Linux fixtures
 * validate logic independently. Worker reads cancel atomically, checks before
 * I/O; cancellation granularity <=100ms. DNS may block; document limitations.
 */
#include "net_probe.h"
#include "status_codec.h"
#include <errno.h>
#include <fcntl.h>
#include <netdb.h>
#include <netinet/in.h>
#include <stdatomic.h>
#include <stdio.h>
#include <string.h>
#include <sys/select.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

// Publish stage with release semantics; main thread acquires terminal stages.
static inline void set_stage(struct np_job *job, enum np_stage stage) {
    atomic_store_explicit(&job->stage, stage, memory_order_release);
}

// Check cancellation before I/O operations (granularity <=100ms).
static inline bool is_cancelled(struct np_job *job) {
    return atomic_load_explicit(&job->cancel, memory_order_acquire);
}

// Get monotonic time in milliseconds; 0 on clock failure.
static uint64_t monotonic_ms(void) {
    struct timespec ts;
    if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0)
        return 0;
    return (uint64_t)ts.tv_sec * 1000 + (uint64_t)ts.tv_nsec / 1000000;
}

// Compute remaining time; 0 if expired or clock unavailable.
static uint64_t time_left(uint64_t deadline) {
    const uint64_t now = monotonic_ms();
    if (now == 0 || now >= deadline)
        return 0;
    return deadline - now;
}

// Set socket to nonblocking mode; returns 0 on success, -1 on error.
static int set_nonblocking(int fd) {
    const int flags = fcntl(fd, F_GETFL, 0);
    if (flags == -1)
        return -1;
    return fcntl(fd, F_SETFL, flags | O_NONBLOCK);
}

// Wait for socket writability (connect) or readability (recv) with timeout.
// Returns 1 ready, 0 timeout, -1 error/cancel. Slices to 100ms for cancel.
static int wait_socket(struct np_job *job, int fd, bool for_write, uint64_t deadline) {
    while (time_left(deadline) > 0 && !is_cancelled(job)) {
        const uint64_t slice = time_left(deadline);
        const uint64_t wait_ms = slice > 100 ? 100 : slice;
        struct timeval tv = {.tv_sec = (long)(wait_ms / 1000),
                              .tv_usec = (long)((wait_ms % 1000) * 1000)};
        fd_set fds;
        FD_ZERO(&fds);
        FD_SET(fd, &fds);
        const int ready = select(fd + 1, for_write ? NULL : &fds,
                                  for_write ? &fds : NULL, NULL, &tv);
        if (ready > 0)
            return 1;
        if (ready < 0 && errno != EINTR)
            return -1;
    }
    return is_cancelled(job) ? -1 : 0;
}

void np_run(struct np_job *job) {
    if (job == NULL) return;
    if (job->host[0] == '\0' || strnlen(job->host, sizeof(job->host)) == sizeof(job->host) || job->port == 0) {
        if (job != NULL) {
            job->error = NP_ERR_ARGUMENT;
            set_stage(job, NP_ERROR);
        }
        return;
    }

    // Reset result fields before starting.
    job->error = NP_ERR_NONE;
    job->system_error = 0;
    job->resolver_error = 0;
    job->codec_error = 0;
    job->response_length = 0;
    job->json_offset = 0;
    job->json_length = 0;
    memset(job->response, 0, sizeof(job->response));
    set_stage(job, NP_IDLE);

    // Establish 10-second deadline AFTER DNS resolution (DNS may block).
    // Cancellation waits for getaddrinfo; no safe way to interrupt it.
    if (is_cancelled(job)) {
        set_stage(job, NP_CANCELLED);
        return;
    }

    uint8_t send_buf[512];
    uint32_t send_len = 0;
    job->codec_error = craftrustnx_status_encode(job->host, (uint32_t)strlen(job->host),
        job->port, job->protocol, send_buf, sizeof(send_buf), &send_len);
    if (job->codec_error != STATUS_OK || send_len == 0) {
        job->error = NP_ERR_PROTOCOL;
        set_stage(job, NP_ERROR);
        return;
    }

    set_stage(job, NP_RESOLVING);
    struct addrinfo hints = {0};
    hints.ai_family = AF_INET;
    hints.ai_socktype = SOCK_STREAM;
    hints.ai_protocol = IPPROTO_TCP;
    struct addrinfo *result = NULL;
    char port_str[8];
    snprintf(port_str, sizeof(port_str), "%u", job->port);
    const int gai_err = getaddrinfo(job->host, port_str, &hints, &result);
    if (is_cancelled(job)) {
        if (result != NULL) freeaddrinfo(result);
        set_stage(job, NP_CANCELLED);
        return;
    }
    if (gai_err != 0 || result == NULL) {
        job->error = NP_ERR_DNS;
        job->resolver_error = gai_err;
        if (result != NULL)
            freeaddrinfo(result);
        set_stage(job, NP_ERROR);
        return;
    }

    // Now start the clock for connect/send/recv.
    const uint64_t start = monotonic_ms();
    if (start == 0) {
        job->error = NP_ERR_CLOCK;
        job->system_error = errno;
        freeaddrinfo(result);
        set_stage(job, NP_ERROR);
        return;
    }
    const uint64_t deadline = start + 10000; // 10 seconds

    if (is_cancelled(job)) {
        freeaddrinfo(result);
        set_stage(job, NP_CANCELLED);
        return;
    }

    set_stage(job, NP_CONNECTING);
    const int sock = socket(AF_INET, SOCK_STREAM, 0);
    if (sock < 0) {
        job->error = NP_ERR_CONNECT;
        job->system_error = errno;
        freeaddrinfo(result);
        set_stage(job, NP_ERROR);
        return;
    }

    if (sock >= FD_SETSIZE || set_nonblocking(sock) < 0) {
        job->error = NP_ERR_CONNECT;
        job->system_error = sock >= FD_SETSIZE ? EMFILE : errno;
        close(sock);
        freeaddrinfo(result);
        set_stage(job, NP_ERROR);
        return;
    }

#ifdef SO_NOSIGPIPE
    int no_sigpipe = 1;
    if (setsockopt(sock, SOL_SOCKET, SO_NOSIGPIPE, &no_sigpipe, sizeof(no_sigpipe)) < 0) {
        job->error = NP_ERR_CONNECT;
        job->system_error = errno;
        close(sock);
        freeaddrinfo(result);
        set_stage(job, NP_ERROR);
        return;
    }
#endif

    // Attempt connect; nonblocking will return EINPROGRESS.
    const int conn_ret = connect(sock, result->ai_addr, result->ai_addrlen);
    const int conn_errno = errno;
    freeaddrinfo(result);
    result = NULL;
    if (conn_ret < 0 && conn_errno != EINPROGRESS) {
        job->error = NP_ERR_CONNECT;
        job->system_error = conn_errno;
        close(sock);
        set_stage(job, NP_ERROR);
        return;
    }

    // Wait for connection or timeout/cancel.
    if (conn_ret < 0) {
        const int wait_result = wait_socket(job, sock, true, deadline);
        if (wait_result <= 0) {
            job->error = is_cancelled(job) ? NP_ERR_NONE : (wait_result == 0 ? NP_ERR_TIMEOUT : NP_ERR_CONNECT);
            job->system_error = wait_result < 0 && !is_cancelled(job) ? errno : 0;
            close(sock);
            set_stage(job, is_cancelled(job) ? NP_CANCELLED : NP_ERROR);
            return;
        }
        // Check SO_ERROR to verify connection success.
        int so_error = 0;
        socklen_t len = sizeof(so_error);
        if (getsockopt(sock, SOL_SOCKET, SO_ERROR, &so_error, &len) < 0 || so_error != 0) {
            job->error = NP_ERR_CONNECT;
            job->system_error = so_error != 0 ? so_error : errno;
            close(sock);
            set_stage(job, NP_ERROR);
            return;
        }
    }

    if (is_cancelled(job)) {
        close(sock);
        set_stage(job, NP_CANCELLED);
        return;
    }

    // Encode STATUS handshake + request via Rust codec.
    set_stage(job, NP_SENDING);

    // Send with partial send handling.
    uint32_t sent = 0;
    while (sent < send_len) {
        if (is_cancelled(job)) {
            close(sock);
            set_stage(job, NP_CANCELLED);
            return;
        }
        if (time_left(deadline) == 0) {
            job->error = NP_ERR_TIMEOUT;
            close(sock);
            set_stage(job, NP_ERROR);
            return;
        }
        const ssize_t n = send(sock, send_buf + sent, send_len - sent,
#ifdef MSG_NOSIGNAL
            MSG_NOSIGNAL
#else
            0
#endif
        );
        if (n < 0) {
            if (errno == EINTR) continue;
            if (errno == EAGAIN || errno == EWOULDBLOCK) {
                const int wait_result = wait_socket(job, sock, true, deadline);
                if (wait_result <= 0) {
                    job->error = is_cancelled(job) ? NP_ERR_NONE : (wait_result == 0 ? NP_ERR_TIMEOUT : NP_ERR_SEND);
                    job->system_error = wait_result < 0 && !is_cancelled(job) ? errno : 0;
                    close(sock);
                    set_stage(job, is_cancelled(job) ? NP_CANCELLED : NP_ERROR);
                    return;
                }
                continue;
            }
            job->error = NP_ERR_SEND;
            job->system_error = errno;
            close(sock);
            set_stage(job, NP_ERROR);
            return;
        }
        if (n == 0) {
            job->error = NP_ERR_SEND;
            close(sock);
            set_stage(job, NP_ERROR);
            return;
        }
        sent += (uint32_t)n;
    }

    if (is_cancelled(job)) {
        close(sock);
        set_stage(job, NP_CANCELLED);
        return;
    }

    // Receive response with partial recv and framing via Rust codec.
    set_stage(job, NP_RECEIVING);
    uint8_t recv_buf[NP_REPLY_CAPACITY];
    uint32_t received = 0;
    uint32_t json_off = 0, json_len = 0;
    const uint32_t max_frame = craftrustnx_status_max_frame_bytes();
    const uint32_t recv_limit = max_frame < NP_REPLY_CAPACITY ? max_frame : NP_REPLY_CAPACITY;

    while (received < recv_limit) {
        if (is_cancelled(job)) {
            close(sock);
            set_stage(job, NP_CANCELLED);
            return;
        }
        if (time_left(deadline) == 0) {
            job->error = NP_ERR_TIMEOUT;
            close(sock);
            set_stage(job, NP_ERROR);
            return;
        }

        // Wait for data availability.
        const int wait_result = wait_socket(job, sock, false, deadline);
        if (wait_result <= 0) {
            job->error = is_cancelled(job) ? NP_ERR_NONE : (wait_result == 0 ? NP_ERR_TIMEOUT : NP_ERR_RECV);
            job->system_error = wait_result < 0 && !is_cancelled(job) ? errno : 0;
            close(sock);
            set_stage(job, is_cancelled(job) ? NP_CANCELLED : NP_ERROR);
            return;
        }

        const ssize_t n = recv(sock, recv_buf + received, recv_limit - received, 0);
        if (n < 0) {
            if (errno == EINTR || errno == EAGAIN || errno == EWOULDBLOCK)
                continue;
            job->error = NP_ERR_RECV;
            job->system_error = errno;
            close(sock);
            set_stage(job, NP_ERROR);
            return;
        }
        if (n == 0) {
            job->error = NP_ERR_EOF;
            close(sock);
            set_stage(job, NP_ERROR);
            return;
        }
        received += (uint32_t)n;
        job->response_length = received;

        // Attempt decode after each chunk.
        uint32_t consumed = 0;
        const int32_t decode_status = craftrustnx_status_decode(
            recv_buf, received, &json_off, &json_len, &consumed);
        if (decode_status == STATUS_OK) {
            // Complete frame received.
            job->response_length = received;
            job->json_offset = json_off;
            job->json_length = json_len;
            memcpy(job->response, recv_buf, received);
            close(sock);
            set_stage(job, NP_DONE);
            return;
        }
        if (decode_status != STATUS_NEED_MORE) {
            job->error = NP_ERR_PROTOCOL;
            job->codec_error = decode_status;
            close(sock);
            set_stage(job, NP_ERROR);
            return;
        }
    }

    // Received max frame bytes without complete frame.
    job->error = NP_ERR_PROTOCOL;
    close(sock);
    set_stage(job, NP_ERROR);
}
