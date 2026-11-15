/*
 * Compile-only ABI probe for Nintendo Switch (devkitA64 + libnx + newlib).
 *
 * This translation unit is only compiled, never linked or executed.  It emits
 * one flat table of little-endian uint64 values into a dedicated ELF section.
 * Each record is three uint64 values:
 *
 *   kind, value_a, value_b
 *
 *   kind 1 = C type/struct size and alignment: value_a = sizeof, value_b = align
 *   kind 2 = struct field offset:             value_a = offsetof, value_b = 0
 *
 * The names for the records are not stored in the ELF section.  They come
 * from abi_items.def, which is the single source for both the C table order
 * and the Python decoder.
 */

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

#define ABI_MAGIC UINT64_C(0x314942412d484357) /* "WCH-ABI1" when read as LE bytes */
#define ABI_SCHEMA UINT64_C(1)

/* Count one record per item without expanding to data. */
#define TYPE(t) +1
#define FIELD(s, f) +1
enum { ABI_RECORD_COUNT = 0
#include "abi_items.def"
};
#undef TYPE
#undef FIELD

/* Emit the actual measurement records. */
#define TYPE(t) 1, (uint64_t)sizeof(t), (uint64_t)_Alignof(t),
#define FIELD(s, f) 2, (uint64_t)offsetof(s, f), UINT64_C(0),

__attribute__((section(".switch_abi"), used, aligned(8)))
const uint64_t switch_abi_table[] = {
    ABI_MAGIC,
    ABI_SCHEMA,
    (uint64_t)ABI_RECORD_COUNT,
#include "abi_items.def"
};
