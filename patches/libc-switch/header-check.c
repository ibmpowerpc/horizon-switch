#define _DEFAULT_SOURCE 1
#include <dirent.h>
#include <netdb.h>
#include <netinet/in.h>
#include <pthread.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <time.h>

#define OK(c) _Static_assert((c), #c)

/* types the Rust patch changes */
OK(sizeof(dev_t) == 2 && _Alignof(dev_t) == 2);
OK(sizeof(ino_t) == 2 && _Alignof(ino_t) == 2);
OK(sizeof(blksize_t) == 8 && _Alignof(blksize_t) == 8);
OK(sizeof(blkcnt_t) == 8 && _Alignof(blkcnt_t) == 8);
OK(sizeof(sa_family_t) == 1 && _Alignof(sa_family_t) == 1);
OK((dev_t)-1 < 0);
OK((ino_t)-1 > 0);
OK((blksize_t)-1 < 0);
OK((blkcnt_t)-1 < 0);
OK((sa_family_t)-1 > 0);
/* socket addresses */
OK(sizeof(struct sockaddr) == 16 && _Alignof(struct sockaddr) == 1);
OK(offsetof(struct sockaddr, sa_len) == 0 && offsetof(struct sockaddr, sa_family) == 1 &&
   offsetof(struct sockaddr, sa_data) == 2);
OK(sizeof(struct sockaddr_in) == 16 && _Alignof(struct sockaddr_in) == 4);
OK(offsetof(struct sockaddr_in, sin_len) == 0 && offsetof(struct sockaddr_in, sin_family) == 1 &&
   offsetof(struct sockaddr_in, sin_port) == 2 && offsetof(struct sockaddr_in, sin_addr) == 4 &&
   offsetof(struct sockaddr_in, sin_zero) == 8);
OK(sizeof(struct sockaddr_storage) == 128 && _Alignof(struct sockaddr_storage) == 8);
OK(offsetof(struct sockaddr_storage, ss_len) == 0 && offsetof(struct sockaddr_storage, ss_family) == 1 &&
   offsetof(struct sockaddr_storage, __ss_pad1) == 2 && offsetof(struct sockaddr_storage, __ss_align) == 8 &&
   offsetof(struct sockaddr_storage, __ss_pad2) == 16);
/* not covered by abi_items.def: no sin6_len in libnx */
OK(sizeof(struct sockaddr_in6) == 28 && _Alignof(struct sockaddr_in6) == 4);
OK(offsetof(struct sockaddr_in6, sin6_family) == 0 && offsetof(struct sockaddr_in6, sin6_port) == 2 &&
   offsetof(struct sockaddr_in6, sin6_flowinfo) == 4 && offsetof(struct sockaddr_in6, sin6_addr) == 8 &&
   offsetof(struct sockaddr_in6, sin6_scope_id) == 24);
/* pthread objects: sizes are measured, field composition is checked here */
OK(sizeof(pthread_mutex_t) == 12 && _Alignof(pthread_mutex_t) == 4);
OK(offsetof(pthread_mutex_t, type) == 0);
OK(sizeof(pthread_cond_t) == 16 && _Alignof(pthread_cond_t) == 8);
OK(offsetof(pthread_cond_t, clock_id) == 0 && offsetof(pthread_cond_t, cond) == 8);
OK(sizeof(pthread_attr_t) == 24 && _Alignof(pthread_attr_t) == 8);
OK(offsetof(pthread_attr_t, stackaddr) == 0 && offsetof(pthread_attr_t, stacksize) == 8 &&
   offsetof(pthread_attr_t, schedparam) == 12 && offsetof(pthread_attr_t, detachstate) == 16);
OK(sizeof(pthread_mutexattr_t) == 4 && _Alignof(pthread_mutexattr_t) == 4);
OK(offsetof(pthread_mutexattr_t, type) == 0);
OK(sizeof(pthread_condattr_t) == 8 && _Alignof(pthread_condattr_t) == 8);
OK(offsetof(pthread_condattr_t, clock_id) == 0);
/* initializer constants (compile-time values, not measured by abi_items.def) */
OK(PTHREAD_MUTEX_NORMAL == 0);
OK(CLOCK_REALTIME == 1);
/* emitted as bytes into .switch_init, dumped with objcopy (not executed) */
struct header_initializers {
    pthread_mutex_t mutex;
    pthread_cond_t cond;
};
OK(offsetof(struct header_initializers, mutex) == 0);
OK(offsetof(struct header_initializers, cond) == 16);
OK(sizeof(struct header_initializers) == 32);
__attribute__((section(".switch_init"), used, aligned(8)))
const struct header_initializers hdr_initializers = {
    .mutex = _PTHREAD_MUTEX_INITIALIZER,
    .cond = _PTHREAD_COND_INITIALIZER,
};
/* dirent / stat */
OK(sizeof(struct dirent) == 260 && _Alignof(struct dirent) == 2);
OK(offsetof(struct dirent, d_ino) == 0 && offsetof(struct dirent, d_type) == 2 && offsetof(struct dirent, d_name) == 3);
OK(sizeof(struct stat) == 104 && _Alignof(struct stat) == 8);
int main(void) { return 0; }
