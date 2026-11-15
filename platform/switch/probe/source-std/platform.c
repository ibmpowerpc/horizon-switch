#include <errno.h>
#include <unistd.h>

#ifndef __SWITCH__
#error This adapter is only for the pinned Nintendo Switch homebrew toolchain.
#endif

// devkitA64 newlib exposes the query ids, but this SDK does not define
// sysconf(). Rust std's thread stack alignment queries _SC_PAGESIZE.
// Horizon userland memory mappings and libnx stacks use 4 KiB pages.
// Other queries remain explicitly unsupported, rather than guessed.
_Static_assert(_SC_PAGESIZE == 8, "Re-audit the newlib/libc sysconf query ids");
long sysconf(int name)
{
    if (name == _SC_PAGESIZE)
        return 0x1000;
    errno = EINVAL;
    return -1;
}
