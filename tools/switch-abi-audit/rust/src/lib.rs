//! Compile-measured ABI probe for libc 0.2.189 on the Switch/newlib cfg.
//!
//! Nothing here is executed. The compiler evaluates `size_of`, `align_of` and
//! `offset_of!` and stores the results in `.switch_abi`, in the same record
//! order as the C probe (`../abi_items.def`):
//!
//!   kind 1 = type size/align (value_a = sizeof, value_b = align)
//!   kind 2 = struct field offset (value_a = offsetof, value_b = 0)
//!   kind 3 = not measurable in Rust (no public field of that name)
//!
//! Kind 3 is emitted instead of guessing a value for C-only or private fields.
//!
//! With `--features patched-libc` the same 69 records are produced against the
//! locally patched libc (see `patches/libc-switch/`), where the six fields that
//! upstream has no public Rust field for do exist. The record count and order
//! never change, so one decoder reads both variants.

#![no_std]

use core::mem::{align_of, offset_of, size_of};

const MAGIC: u64 = 0x314942412d484357; // same "WCH-ABI1" marker as the C table
const SCHEMA: u64 = 1;
const COUNT: usize = 69;
#[allow(dead_code)] // only used by the unpatched record set
const ABSENT: u64 = u64::MAX;

macro_rules! size_align {
    ($t:ty) => {
        [1, size_of::<$t>() as u64, align_of::<$t>() as u64]
    };
}

macro_rules! field {
    ($t:ty, $f:ident) => {
        [2, offset_of!($t, $f) as u64, 0]
    };
}

#[allow(unused_macros)] // kept for C-only fields with no Rust representation
macro_rules! absent {
    () => {
        [3, ABSENT, 0]
    };
}

// Fields that upstream libc 0.2.189 does not represent (no public Rust field)
// but that the local Switch patch adds. Unpatched builds keep them explicitly
// absent; the patched build measures the real offset instead.
#[cfg(feature = "patched-libc")]
macro_rules! field_or_absent {
    ($t:ty, $f:ident) => {
        [2, offset_of!($t, $f) as u64, 0]
    };
}

#[cfg(not(feature = "patched-libc"))]
macro_rules! field_or_absent {
    ($t:ty, $f:ident) => {
        [3, ABSENT, 0]
    };
}

const ITEMS: [[u64; 3]; COUNT] = [
    // Types (abi_items.def order, lines 1-28).
    size_align!(libc::dev_t),
    size_align!(libc::ino_t),
    size_align!(libc::off_t),
    size_align!(libc::blksize_t),
    size_align!(libc::blkcnt_t),
    size_align!(libc::sa_family_t),
    size_align!(libc::socklen_t),
    size_align!(libc::time_t),
    size_align!(libc::mode_t),
    size_align!(libc::nlink_t),
    size_align!(libc::uid_t),
    size_align!(libc::gid_t),
    size_align!(libc::in_port_t),
    size_align!(libc::in_addr_t),
    size_align!(libc::timespec),
    size_align!(libc::stat),
    size_align!(libc::dirent),
    size_align!(libc::sockaddr),
    size_align!(libc::sockaddr_in),
    size_align!(libc::in_addr),
    size_align!(libc::sockaddr_storage),
    size_align!(libc::addrinfo),
    size_align!(libc::pthread_mutex_t),
    size_align!(libc::pthread_cond_t),
    size_align!(libc::pthread_attr_t),
    size_align!(libc::pthread_key_t),
    size_align!(libc::pthread_mutexattr_t),
    size_align!(libc::pthread_condattr_t),
    // Fields (lines 29-69).
    field!(libc::timespec, tv_sec),
    field!(libc::timespec, tv_nsec),
    field!(libc::stat, st_dev),
    field!(libc::stat, st_ino),
    field!(libc::stat, st_mode),
    field!(libc::stat, st_nlink),
    field!(libc::stat, st_uid),
    field!(libc::stat, st_gid),
    field!(libc::stat, st_rdev),
    field!(libc::stat, st_size),
    field!(libc::stat, st_atim),
    field!(libc::stat, st_mtim),
    field!(libc::stat, st_ctim),
    field!(libc::stat, st_blksize),
    field!(libc::stat, st_blocks),
    field!(libc::stat, st_spare4),
    field!(libc::dirent, d_ino),
    field!(libc::dirent, d_type),
    field!(libc::dirent, d_name),
    field_or_absent!(libc::sockaddr, sa_len), // absent upstream; patched Switch libc has it
    field!(libc::sockaddr, sa_family),
    field!(libc::sockaddr, sa_data),
    field_or_absent!(libc::sockaddr_in, sin_len), // absent upstream; patched Switch libc has it
    field!(libc::sockaddr_in, sin_family),
    field!(libc::sockaddr_in, sin_port),
    field!(libc::sockaddr_in, sin_addr),
    field!(libc::sockaddr_in, sin_zero),
    field!(libc::in_addr, s_addr),
    field_or_absent!(libc::sockaddr_storage, ss_len), // absent upstream; patched Switch libc has it
    field!(libc::sockaddr_storage, ss_family),
    field_or_absent!(libc::sockaddr_storage, __ss_pad1), // padding: unpadded upstream
    field_or_absent!(libc::sockaddr_storage, __ss_align), // padding: unpadded upstream
    field_or_absent!(libc::sockaddr_storage, __ss_pad2), // padding: unpadded upstream
    field!(libc::addrinfo, ai_flags),
    field!(libc::addrinfo, ai_family),
    field!(libc::addrinfo, ai_socktype),
    field!(libc::addrinfo, ai_protocol),
    field!(libc::addrinfo, ai_addrlen),
    field!(libc::addrinfo, ai_canonname),
    field!(libc::addrinfo, ai_addr),
    field!(libc::addrinfo, ai_next),
];

// `sa_family_t` is u16 upstream and u8 in the patched Switch libc. libnx's
// `struct sockaddr_in6` has no `sin6_len` (unlike `sockaddr`/`sockaddr_in`), so
// its layout is independent of that width. It is not one of the 69 records in
// `../abi_items.def` (that list is shared with the C probe), so the C layout
// measured from the devkitA64 headers is enforced here at compile time.
const _: () = {
    assert!(size_of::<libc::sockaddr_in6>() == 28);
    assert!(align_of::<libc::sockaddr_in6>() == 4);
    assert!(offset_of!(libc::sockaddr_in6, sin6_family) == 0);
    assert!(offset_of!(libc::sockaddr_in6, sin6_port) == 2);
    assert!(offset_of!(libc::sockaddr_in6, sin6_flowinfo) == 4);
    assert!(offset_of!(libc::sockaddr_in6, sin6_addr) == 8);
    assert!(offset_of!(libc::sockaddr_in6, sin6_scope_id) == 24);
};

const fn flatten() -> [u64; 3 + COUNT * 3] {    let mut out = [0u64; 3 + COUNT * 3];
    out[0] = MAGIC;
    out[1] = SCHEMA;
    out[2] = COUNT as u64;
    let mut i = 0;
    while i < COUNT {
        let mut j = 0;
        while j < 3 {
            out[3 + i * 3 + j] = ITEMS[i][j];
            j += 1;
        }
        i += 1;
    }
    out
}

#[used]
#[link_section = ".switch_abi"]
static TABLE: [u64; 3 + COUNT * 3] = flatten();

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
