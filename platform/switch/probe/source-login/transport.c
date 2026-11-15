/* POSIX C socket transport for Minecraft Java Edition server LOGIN probe.
 * No libnx, no switch.h, no threading or service calls: macOS/Linux fixtures
 * validate logic independently. Worker reads cancel atomically, checks before
 * I/O; cancellation granularity <=100ms. DNS may block; document limitations.
 * Recv stream may contain multiple frames: decode ONE with current compression;
 * on SET_COMPRESSION consume/memmove unconsumed tail/update job.compression and
 * parse ALREADY buffered frames immediately before recv (coalescing!). Threshold
 * -1 disable supported. LP_NEED_MORE recv only if room; malformed/overflow fail.
 * On validated LOGIN_SUCCESS terminal LJ_ACCEPTED; DISCONNECT LJ_DISCONNECTED;
 * ENCRYPTION_REQUEST stops in the info variant; encrypted variant replies with
 * RSA and continues through AES without a session join. Query/Cookie LJ_UNSUPPORTED. Copy event,
 * retain scratch for display, close all resources BEFORE terminal release. No
 * LoginAcknowledged. Offline vanilla normally SKIPS encryption request (not
 * empty keys false); distinct default name avoids disrupting host player.
 * Header CAP deliberately rejects over 64K unsupported packets; no 8MB
 * scratch/watchdog.
 */
#include "login_probe.h"
#include "login_codec.h"
#ifdef CRAFTRUSTNX_ENCRYPTED_LOGIN
#include "login_crypto.h"
#endif
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
static inline void set_stage(struct lj_job *job, enum lj_stage stage) {
    atomic_store_explicit(&job->stage, stage, memory_order_release);
}

// Check cancellation before I/O operations (granularity <=100ms).
static inline bool is_cancelled(struct lj_job *job) {
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
static int wait_socket(struct lj_job *job, int fd, bool for_write, uint64_t deadline) {
    if (fd >= FD_SETSIZE)
        return -1;
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

struct cipher_state {
#ifdef CRAFTRUSTNX_ENCRYPTED_LOGIN
    struct lc_session *session;
#else
    int unused;
#endif
};
static void close_connection(int fd, struct cipher_state *cipher) {
#ifdef CRAFTRUSTNX_ENCRYPTED_LOGIN
    lc_session_free(cipher->session);
    cipher->session = NULL;
#else
    (void)cipher;
#endif
    close(fd);
}
#ifdef CRAFTRUSTNX_ENCRYPTED_LOGIN
/* Key is the final plaintext packet. The caller activates stream decryption
 * only after this write completes; server authentication policy is not guessed
 * from should_authenticate. This offline probe sends no session-service join. */
static int exchange_key(struct lj_job *job, int fd, struct cipher_state *cipher, uint64_t deadline) {
    const struct lp_event *event = &job->event;
    if (cipher->session || event->public_key_offset > LP_MAX_BODY ||
        event->public_key_length > LP_MAX_BODY - event->public_key_offset ||
        event->challenge_offset > LP_MAX_BODY ||
        event->challenge_length > LP_MAX_BODY - event->challenge_offset)
        return LJ_ERR_PROTOCOL;
    uint8_t encrypted_secret[LC_MAX_KEY_BYTES], encrypted_challenge[LC_MAX_KEY_BYTES];
    uint32_t cipher_length = 0;
    job->crypto_error = lc_session_create(job->scratch + event->public_key_offset,
        event->public_key_length, job->scratch + event->challenge_offset,
        event->challenge_length, encrypted_secret, encrypted_challenge,
        &cipher_length, &cipher->session, &job->crypto_detail);
    if (job->crypto_error != LC_OK) return LJ_ERR_CRYPTO;
    uint8_t response[1024];
    uint32_t written = 0;
    job->codec_error = craftrustnx_login_key(encrypted_secret, cipher_length,
        encrypted_challenge, cipher_length, job->compression, response, sizeof(response), &written);
    if (job->codec_error != LP_OK || !written || written > sizeof(response)) return LJ_ERR_PROTOCOL;
    uint32_t sent = 0;
    while (sent < written) {
        if (is_cancelled(job)) return -1;
        if (!time_left(deadline)) return LJ_ERR_TIMEOUT;
        const ssize_t count = send(fd, response + sent, written - sent,
#ifdef MSG_NOSIGNAL
            MSG_NOSIGNAL
#else
            0
#endif
        );
        if (count < 0) {
            if (errno == EINTR) continue;
            if (errno == EAGAIN || errno == EWOULDBLOCK) {
                int ready = wait_socket(job, fd, true, deadline);
                if (ready > 0) continue;
                if (is_cancelled(job)) return -1;
                if (!ready) return LJ_ERR_TIMEOUT;
            }
            job->system_error = errno;
            return LJ_ERR_SEND;
        }
        if (!count) return LJ_ERR_SEND;
        sent += (uint32_t)count;
    }
    job->encryption_active = true;
    return LJ_ERR_NONE;
}
#endif

void lj_run(struct lj_job *job) {
    if (job == NULL) return;

    struct cipher_state cipher = {0};

    // Reset result fields before starting.
    job->error = LJ_ERR_NONE;
    job->system_error = 0;
    job->resolver_error = 0;
    job->codec_error = 0;
    job->crypto_error = 0;
    job->crypto_detail = 0;
    job->saw_encryption_request = false;
    job->server_requested_auth = false;
    job->encryption_active = false;
    job->compression = -1;
    job->received_bytes = 0;
    job->frame_count = 0;
    memset(&job->event, 0, sizeof(job->event));
    memset(job->wire, 0, sizeof(job->wire));
    memset(job->scratch, 0, sizeof(job->scratch));
    set_stage(job, LJ_IDLE);


    // Validate inputs.
    const size_t host_len = strnlen(job->host, sizeof(job->host));
    const size_t name_len = strnlen(job->name, sizeof(job->name));
    if (host_len == 0 || host_len == sizeof(job->host) || job->port == 0 ||
        name_len == 0 || name_len == sizeof(job->name)) {
        job->error = LJ_ERR_ARGUMENT;
        set_stage(job, LJ_ERROR);
        return;
    }

    // Validate name: 1..16 ASCII letters, digits, underscore.
    for (size_t i = 0; i < name_len; ++i) {
        const char c = job->name[i];
        if (!((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') ||
              (c >= '0' && c <= '9') || c == '_')) {
            job->error = LJ_ERR_ARGUMENT;
            set_stage(job, LJ_ERROR);
            return;
        }
    }

    // Establish 10-second deadline AFTER DNS resolution (DNS may block).
    // Cancellation waits for getaddrinfo; no safe way to interrupt it.
    if (is_cancelled(job)) {
        set_stage(job, LJ_CANCELLED);
        return;
    }

    // Encode handshake + LoginStart via Rust codec.
    uint32_t written = 0;
    job->codec_error = craftrustnx_login_start(job->host, (uint32_t)host_len, job->port,
        job->name, (uint32_t)name_len, job->wire, LP_MAX_WIRE, &written);
    if (job->codec_error != LP_OK || written == 0 || written > LP_MAX_WIRE) {
        job->error = LJ_ERR_PROTOCOL;
        set_stage(job, LJ_ERROR);
        return;
    }

    set_stage(job, LJ_RESOLVING);
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
        set_stage(job, LJ_CANCELLED);
        return;
    }
    if (gai_err != 0 || result == NULL) {
        job->error = LJ_ERR_DNS;
        job->resolver_error = gai_err;
        if (result != NULL)
            freeaddrinfo(result);
        set_stage(job, LJ_ERROR);
        return;
    }

    // Use first IPv4 address only.
    struct sockaddr_in *addr_in = NULL;
    for (struct addrinfo *rp = result; rp != NULL; rp = rp->ai_next) {
        if (rp->ai_family == AF_INET && rp->ai_addr != NULL) {
            addr_in = (struct sockaddr_in *)rp->ai_addr;
            break;
        }
    }
    if (addr_in == NULL) {
        job->error = LJ_ERR_DNS;
        freeaddrinfo(result);
        set_stage(job, LJ_ERROR);
        return;
    }

    // Now start the clock for connect/send/recv.
    const uint64_t start = monotonic_ms();
    if (start == 0) {
        job->error = LJ_ERR_CLOCK;
        job->system_error = errno;
        freeaddrinfo(result);
        set_stage(job, LJ_ERROR);
        return;
    }
    const uint64_t deadline = start + 10000; // 10 seconds AFTER DNS

    if (is_cancelled(job)) {
        freeaddrinfo(result);
        set_stage(job, LJ_CANCELLED);
        return;
    }

    set_stage(job, LJ_CONNECTING);
    const int sock = socket(AF_INET, SOCK_STREAM, 0);
    if (sock < 0) {
        const int saved_errno = errno;
        freeaddrinfo(result);
        job->error = LJ_ERR_CONNECT;
        job->system_error = saved_errno;
        set_stage(job, LJ_ERROR);
        return;
    }

    if (sock >= FD_SETSIZE || set_nonblocking(sock) < 0) {
        const int saved_errno = sock >= FD_SETSIZE ? EMFILE : errno;
        close_connection(sock, &cipher);
        freeaddrinfo(result);
        job->error = LJ_ERR_CONNECT;
        job->system_error = saved_errno;
        set_stage(job, LJ_ERROR);
        return;
    }

#ifdef SO_NOSIGPIPE
    int no_sigpipe = 1;
    if (setsockopt(sock, SOL_SOCKET, SO_NOSIGPIPE, &no_sigpipe, sizeof(no_sigpipe)) < 0) {
        const int saved_errno = errno;
        close_connection(sock, &cipher);
        freeaddrinfo(result);
        job->error = LJ_ERR_CONNECT;
        job->system_error = saved_errno;
        set_stage(job, LJ_ERROR);
        return;
    }
#endif

    // Attempt connect; nonblocking will return EINPROGRESS.
    const int conn_ret = connect(sock, (struct sockaddr *)addr_in, sizeof(*addr_in));
    const int conn_errno = errno;
    freeaddrinfo(result);
    result = NULL;
    if (conn_ret < 0 && conn_errno != EINPROGRESS) {
        close_connection(sock, &cipher);
        job->error = LJ_ERR_CONNECT;
        job->system_error = conn_errno;
        set_stage(job, LJ_ERROR);
        return;
    }

    // Wait for connection or timeout/cancel.
    if (conn_ret < 0) {
        const int wait_result = wait_socket(job, sock, true, deadline);
        if (wait_result <= 0) {
            const int saved_errno = errno;
            close_connection(sock, &cipher);
            job->error = is_cancelled(job) ? LJ_ERR_NONE : (wait_result == 0 ? LJ_ERR_TIMEOUT : LJ_ERR_CONNECT);
            job->system_error = wait_result < 0 && !is_cancelled(job) ? saved_errno : 0;
            set_stage(job, is_cancelled(job) ? LJ_CANCELLED : LJ_ERROR);
            return;
        }
        // Check SO_ERROR to verify connection success.
        int so_error = 0;
        socklen_t len = sizeof(so_error);
        if (getsockopt(sock, SOL_SOCKET, SO_ERROR, &so_error, &len) < 0 || so_error != 0) {
            const int saved_errno = errno;
            close_connection(sock, &cipher);
            job->error = LJ_ERR_CONNECT;
            job->system_error = so_error != 0 ? so_error : saved_errno;
            set_stage(job, LJ_ERROR);
            return;
        }
    }

    if (is_cancelled(job)) {
        close_connection(sock, &cipher);
        set_stage(job, LJ_CANCELLED);
        return;
    }

    // Send handshake + LoginStart with partial send handling.
    set_stage(job, LJ_SENDING);
    uint32_t sent = 0;
    while (sent < written) {
        if (is_cancelled(job)) {
            close_connection(sock, &cipher);
            set_stage(job, LJ_CANCELLED);
            return;
        }
        if (time_left(deadline) == 0) {
            close_connection(sock, &cipher);
            job->error = LJ_ERR_TIMEOUT;
            set_stage(job, LJ_ERROR);
            return;
        }
        const ssize_t n = send(sock, job->wire + sent, written - sent,
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
                    const int saved_errno = errno;
                    close_connection(sock, &cipher);
                    job->error = is_cancelled(job) ? LJ_ERR_NONE : (wait_result == 0 ? LJ_ERR_TIMEOUT : LJ_ERR_SEND);
                    job->system_error = wait_result < 0 && !is_cancelled(job) ? saved_errno : 0;
                    set_stage(job, is_cancelled(job) ? LJ_CANCELLED : LJ_ERROR);
                    return;
                }
                continue;
            }
            const int saved_errno = errno;
            close_connection(sock, &cipher);
            job->error = LJ_ERR_SEND;
            job->system_error = saved_errno;
            set_stage(job, LJ_ERROR);
            return;
        }
        if (n == 0) {
            close_connection(sock, &cipher);
            job->error = LJ_ERR_SEND;
            set_stage(job, LJ_ERROR);
            return;
        }
        sent += (uint32_t)n;
    }

    if (is_cancelled(job)) {
        close_connection(sock, &cipher);
        set_stage(job, LJ_CANCELLED);
        return;
    }

    // Receive response with partial recv and multi-frame decoding.
    set_stage(job, LJ_RECEIVING);
    // The request is fully sent; reuse caller-owned static storage for input.
    // A 64 KiB automatic array here would exhaust the libnx worker stack.
    uint8_t *recv_buf = job->wire;
    uint32_t buffered = 0;
    const uint32_t max_events = 16;

    while (job->frame_count < max_events) {
        // First try to decode already buffered data (compression change coalescing).
        while (buffered > 0 && job->frame_count < max_events) {
            if (is_cancelled(job) || time_left(deadline) == 0) {
                const bool cancelled = is_cancelled(job);
                close_connection(sock, &cipher);
                job->error = cancelled ? LJ_ERR_NONE : LJ_ERR_TIMEOUT;
                set_stage(job, cancelled ? LJ_CANCELLED : LJ_ERROR);
                return;
            }
            struct lp_event event = {0};
            const int32_t decode_status = craftrustnx_login_decode(
                recv_buf, buffered, job->compression, job->scratch, LP_MAX_BODY, &event);

            if (decode_status == LP_OK) {
                // Complete frame decoded.
                ++job->frame_count;
                job->event = event;

                // Consume the frame.
                if (event.consumed == 0 || event.consumed > buffered) {
                    close_connection(sock, &cipher);
                    job->error = LJ_ERR_PROTOCOL;
                    set_stage(job, LJ_ERROR);
                    return;
                }
                memmove(recv_buf, recv_buf + event.consumed, buffered - event.consumed);
                buffered -= event.consumed;

                // Handle events.
                if (event.kind == LP_SET_COMPRESSION) {
                    job->compression = event.threshold;
                    // Continue immediately to parse already buffered frames with new compression.
                    continue;
                } else if (event.kind == LP_LOGIN_SUCCESS) {
                    close_connection(sock, &cipher);
                    set_stage(job, LJ_ACCEPTED);
                    return;
                } else if (event.kind == LP_DISCONNECT) {
                    close_connection(sock, &cipher);
                    set_stage(job, LJ_DISCONNECTED);
                    return;
                } else if (event.kind == LP_ENCRYPTION_REQUEST) {
                    job->saw_encryption_request = true;
                    job->server_requested_auth = event.should_authenticate != 0;
#ifdef CRAFTRUSTNX_ENCRYPTED_LOGIN
                    set_stage(job, LJ_KEY_EXCHANGE);
                    const int error = exchange_key(job, sock, &cipher, deadline);
                    if (error != LJ_ERR_NONE) {
                        close_connection(sock, &cipher);
                        job->error = error < 0 ? LJ_ERR_NONE : error;
                        set_stage(job, error < 0 ? LJ_CANCELLED :
                            job->codec_error == LP_UNSUPPORTED ? LJ_UNSUPPORTED : LJ_ERROR);
                        return;
                    }
                    // Any residual bytes are at receive-stream position zero.
                    // They are decrypted exactly once; future recv decrypts only new bytes.
                    job->crypto_error = lc_session_decrypt(cipher.session, recv_buf, buffered);
                    if (job->crypto_error != LC_OK) {
                        close_connection(sock, &cipher);
                        job->error = LJ_ERR_CRYPTO;
                        set_stage(job, LJ_ERROR);
                        return;
                    }
                    set_stage(job, LJ_RECEIVING);
                    continue;
#else
                    close_connection(sock, &cipher);
                    set_stage(job, LJ_AUTH_REQUIRED);
                    return;
#endif
                } else if (event.kind == LP_CUSTOM_QUERY || event.kind == LP_COOKIE_REQUEST) {
                    close_connection(sock, &cipher);
                    set_stage(job, LJ_UNSUPPORTED);
                    return;
                }
            } else if (decode_status == LP_NEED_MORE) {
                // Need more data; break to recv.
                break;
            } else {
                // Malformed or overflow.
                close_connection(sock, &cipher);
                if (decode_status == LP_UNSUPPORTED) {
                    job->codec_error = decode_status;
                    set_stage(job, LJ_UNSUPPORTED);
                    return;
                }
                job->error = LJ_ERR_PROTOCOL;
                job->codec_error = decode_status;
                set_stage(job, LJ_ERROR);
                return;
            }
        }
        if (job->frame_count >= max_events) break;

        // Check cancellation and timeout before recv.
        if (is_cancelled(job)) {
            close_connection(sock, &cipher);
            set_stage(job, LJ_CANCELLED);
            return;
        }
        if (time_left(deadline) == 0) {
            close_connection(sock, &cipher);
            job->error = LJ_ERR_TIMEOUT;
            set_stage(job, LJ_ERROR);
            return;
        }

        // LP_NEED_MORE recv only if room.
        if (buffered >= LP_MAX_WIRE) {
            close_connection(sock, &cipher);
            job->error = LJ_ERR_PROTOCOL;
            set_stage(job, LJ_ERROR);
            return;
        }

        // Wait for data availability.
        const int wait_result = wait_socket(job, sock, false, deadline);
        if (wait_result <= 0) {
            const int saved_errno = errno;
            close_connection(sock, &cipher);
            job->error = is_cancelled(job) ? LJ_ERR_NONE : (wait_result == 0 ? LJ_ERR_TIMEOUT : LJ_ERR_RECV);
            job->system_error = wait_result < 0 && !is_cancelled(job) ? saved_errno : 0;
            set_stage(job, is_cancelled(job) ? LJ_CANCELLED : LJ_ERROR);
            return;
        }

        const ssize_t n = recv(sock, recv_buf + buffered, LP_MAX_WIRE - buffered, 0);
        if (n < 0) {
            if (errno == EINTR || errno == EAGAIN || errno == EWOULDBLOCK)
                continue;
            const int saved_errno = errno;
            close_connection(sock, &cipher);
            job->error = LJ_ERR_RECV;
            job->system_error = saved_errno;
            set_stage(job, LJ_ERROR);
            return;
        }
        if (n == 0) {
            close_connection(sock, &cipher);
            job->error = LJ_ERR_EOF;
            set_stage(job, LJ_ERROR);
            return;
        }
#ifdef CRAFTRUSTNX_ENCRYPTED_LOGIN
        if (job->encryption_active) {
            job->crypto_error = lc_session_decrypt(cipher.session, recv_buf + buffered, (uint32_t)n);
            if (job->crypto_error != LC_OK) {
                close_connection(sock, &cipher);
                job->error = LJ_ERR_CRYPTO;
                set_stage(job, LJ_ERROR);
                return;
            }
        }
#endif
        buffered += (uint32_t)n;
        job->received_bytes += (uint32_t)n;
    }

    // Received max events without terminal event.
    close_connection(sock, &cipher);
    job->error = LJ_ERR_PROTOCOL;
    set_stage(job, LJ_ERROR);
}
