//! Compile-only std runtime probe for the native Nintendo Switch (Atmosphere /
//! hbmenu) Horizon port.
//!
//! Scope: this crate is built as a C-callable staticlib so that the six runtime
//! facts a Rust `std` needs on Horizon can be checked *individually* from the
//! console-side C/libnx harness:
//!
//! 1. bounded `Vec`/`String` allocation through the global allocator,
//! 2. `Instant` / `SystemTime` (monotone clock + RTC-backed wall clock),
//! 3. `thread_local!` on the main thread,
//! 4. `std::thread` spawn + join,
//! 5. `Mutex` + `Condvar` (timeout and notify paths),
//! 6. `thread_local!` destructors on a spawned thread (separately selectable,
//!    because it is the part that depends on pthread-key TLS).
//!
//! Design rules, so a green result means something:
//!
//! * No `HashMap` / `RandomState`: its SipHash keys come from `std`'s random
//!   source, which for `target_os = "horizon"` is `libc::getrandom` (see
//!   `library/std/src/sys/random/mod.rs:47`) and is *not* verified here.
//! * No fixed RNG keys and no fabricated randomness.
//! * No filesystem, no environment, no stdout/stderr: nothing writes to the SD
//!   card implicitly, and the result travels only through the returned bits.
//! * A check that panics aborts the process (`panic = "abort"`); the C side must
//!   treat "the call never returned" as *not passed*. Unexecuted checks are
//!   never reported as passed: the result word carries the requested mask
//!   separately from the passed and failed masks, and by construction
//!   `passed | failed == requested` and `passed & failed == 0`.
//!
//! The target code is never executed by this repository's tooling. Running it
//! on a console is a separate, explicit step.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Check identity and result encoding (C ABI)
// ---------------------------------------------------------------------------

/// Bounded `Vec`/`String` allocation and deallocation.
pub const CHECK_ALLOC: u32 = 1 << 0;
/// `Instant` monotonicity and `SystemTime` against the Unix epoch.
pub const CHECK_TIME: u32 = 1 << 1;
/// `thread_local!` read/write on the main thread (no destructor).
pub const CHECK_TLS_MAIN: u32 = 1 << 2;
/// `std::thread::Builder` spawn + join with a bounded stack.
pub const CHECK_THREAD: u32 = 1 << 3;
/// `Mutex` + `Condvar`: timeout path and notify/wakeup path.
pub const CHECK_SYNC: u32 = 1 << 4;
/// `thread_local!` destructors on a spawned thread (pthread-key TLS path).
pub const CHECK_TLS_DTOR: u32 = 1 << 5;

/// Every check this build knows about.
pub const KNOWN_CHECKS: u32 =
    CHECK_ALLOC | CHECK_TIME | CHECK_TLS_MAIN | CHECK_THREAD | CHECK_SYNC | CHECK_TLS_DTOR;

/// First mask to request on a console: everything except the destructor check,
/// so that a TLS-destructor failure cannot hide the other results.
pub const RECOMMENDED_FIRST_MASK: u32 = KNOWN_CHECKS & !CHECK_TLS_DTOR;

/// Result-word signature, ASCII `"ST"`.
const RESULT_SIGNATURE: u64 = 0x5354;
/// A result word must carry this signature in bits 48..64.
pub const RESULT_SIGNATURE_MASK: u64 = 0xffff << 48;
pub const RESULT_SIGNATURE_VALUE: u64 = RESULT_SIGNATURE << 48;

const fn encode_result(requested: u32, passed: u32, failed: u32) -> u64 {
    (RESULT_SIGNATURE << 48)
        | (((failed & 0xffff) as u64) << 32)
        | (((passed & 0xffff) as u64) << 16)
        | ((requested & 0xffff) as u64)
}

/// ABI revision of the exported surface. Bump on any incompatible change.
#[no_mangle]
pub extern "C" fn craftrustnx_std_probe_abi_version() -> u32 {
    1
}

/// Bit mask of the checks this build can execute.
#[no_mangle]
pub extern "C" fn craftrustnx_std_probe_checks() -> u32 {
    KNOWN_CHECKS
}

/// Run the requested checks and return the result word.
///
/// Layout, little-endian bit numbering inside the word:
///
/// | bits     | meaning                                            |
/// |----------|----------------------------------------------------|
/// | 0..16    | requested mask, echoed back                        |
/// | 16..32   | passed mask                                        |
/// | 32..48   | failed mask (includes requested-but-unknown bits)   |
/// | 48..64   | `0x5354` signature                                 |
///
/// Invariants the C side must re-check instead of trusting the crate:
/// signature matches, `passed & failed == 0`, `passed | failed == requested`,
/// and only then read individual bits.
#[no_mangle]
pub extern "C" fn craftrustnx_std_probe_run(requested: u32) -> u64 {
    // The wire result has 16-bit masks. Reject an unrepresentable request
    // with an invalid signature rather than silently truncating its failures.
    if requested > u16::MAX as u32 {
        return 0;
    }
    let mut passed = 0u32;
    let mut failed = requested & !KNOWN_CHECKS; // unknown requests fail closed

    for (bit, check) in CHECKS {
        if requested & bit == 0 {
            continue;
        }
        if check() {
            passed |= bit;
        } else {
            failed |= bit;
        }
    }

    debug_assert_eq!(passed & failed, 0);
    debug_assert_eq!(passed | failed, requested);

    encode_result(requested, passed, failed)
}

type Check = fn() -> bool;

const CHECKS: [(u32, Check); 6] = [
    (CHECK_ALLOC, check_alloc_bounded),
    (CHECK_TIME, check_time),
    (CHECK_TLS_MAIN, check_tls_main),
    (CHECK_THREAD, check_thread),
    (CHECK_SYNC, check_sync),
    (CHECK_TLS_DTOR, check_tls_destructor),
];

// ---------------------------------------------------------------------------
// 1. Bounded allocation through the global allocator (newlib malloc/free)
// ---------------------------------------------------------------------------

/// Every allocation here is explicitly bounded; the expected sums are computed
/// independently of the loop that fills the containers.
fn check_alloc_bounded() -> bool {
    const COUNT: u32 = 32;

    let mut values: Vec<u32> = Vec::with_capacity(COUNT as usize);
    if values.capacity() < COUNT as usize {
        return false;
    }
    let mut sum = 0u64;
    for value in 0..COUNT {
        values.push(value);
        sum = sum.wrapping_add(u64::from(value));
    }
    std::hint::black_box(&mut values);
    // 0 + 1 + ... + 31 == 496, independent of the loop above.
    if sum != 496 || values.len() != COUNT as usize {
        return false;
    }
    if values.first() != Some(&0) || values.last() != Some(&(COUNT - 1)) {
        return false;
    }
    values.clear();
    drop(values);

    let mut text = String::with_capacity(64);
    text.push_str("switch-std-probe");
    std::hint::black_box(&mut text);
    if text.len() != 16 || text.as_bytes().first() != Some(&b's') {
        return false;
    }
    text.push('!');
    if text.pop() != Some('!') || text.len() != 16 {
        return false;
    }
    drop(text);

    // Exercise another allocation after freeing the earlier containers.
    let mut refill = Vec::with_capacity(8);
    refill.resize(8, 7u8);
    std::hint::black_box(&mut refill);
    refill.len() == 8 && refill.iter().all(|byte| *byte == 7)
}

// ---------------------------------------------------------------------------
// 2. Time: monotone Instant, RTC-backed SystemTime
// ---------------------------------------------------------------------------

/// 2020-01-01T00:00:00Z: a console whose RTC was never set reports a much
/// smaller value, which must not be counted as a pass for `SystemTime`.
const SYSTEM_TIME_FLOOR_SECS: u64 = 1_577_836_800;

fn check_time() -> bool {
    let since_epoch = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration,
        Err(_) => return false,
    };
    if since_epoch.as_secs() < SYSTEM_TIME_FLOOR_SECS {
        return false;
    }

    let first = Instant::now();
    let second = Instant::now();
    if second < first {
        return false;
    }
    if second.duration_since(first) > Duration::from_secs(60) {
        return false;
    }

    // Bounded work, then a monotone read: the clock must not go backwards and
    // must not jump by minutes while the CPU never sleeps.
    let mut acc = 0u64;
    for step in 0..20_000u64 {
        acc = acc.wrapping_add(step);
        std::hint::black_box(acc);
    }
    let third = Instant::now();
    acc != 0 && third > second && third.duration_since(second) <= Duration::from_secs(60)
}

// ---------------------------------------------------------------------------
// 3. thread_local! on the main thread
// ---------------------------------------------------------------------------

thread_local! {
    static MAIN_SLOT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

fn check_tls_main() -> bool {
    let wrote = MAIN_SLOT
        .try_with(|slot| {
            if slot.get() != 0 {
                return false;
            }
            slot.set(0x5a5a);
            slot.get() == 0x5a5a
        })
        .unwrap_or(false);
    if !wrote {
        return false;
    }
    // A second access from the same thread must see the same storage.
    MAIN_SLOT
        .try_with(|slot| {
            let correct = slot.get() == 0x5a5a;
            slot.set(0);
            correct
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// 4. std::thread spawn + join (pthread_create/pthread_join in libsysbase)
// ---------------------------------------------------------------------------

/// Bounded stack: the console harness owns a small heap and a fixed thread
/// budget, and a 2 MiB default stack would hide allocation failures.
const THREAD_STACK_BYTES: usize = 96 * 1024;

fn check_thread() -> bool {
    let handle = match std::thread::Builder::new()
        .name("std-probe-worker".to_string())
        .stack_size(THREAD_STACK_BYTES)
        .spawn(|| {
            let payload: Vec<u32> = (0..16u32).collect();
            let total: u32 = payload.iter().sum();
            // 0 + 1 + ... + 15 == 120
            total == 120 && payload.len() == 16
        }) {
        Ok(handle) => handle,
        Err(_) => return false,
    };

    handle.join().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// 5. Mutex + Condvar: bounded timeout, then a real wakeup
// ---------------------------------------------------------------------------

const WAIT_STEP: Duration = Duration::from_millis(20);
const WAIT_STEPS: u32 = 100;
const WAIT_DEADLINE: Duration = Duration::from_secs(5);

fn check_sync() -> bool {
    let shared = Arc::new((Mutex::new(0u32), Condvar::new()));

    // (a) The timeout path must return on its own, and quickly.
    let started = Instant::now();
    {
        let (mutex, condvar) = &*shared;
        let guard = match mutex.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let (guard, timed_out) = match condvar.wait_timeout_while(guard, WAIT_STEP, |v| *v == 0) {
            Ok(result) => result,
            Err(poisoned) => poisoned.into_inner(),
        };
        if *guard != 0 || !timed_out.timed_out() {
            return false;
        }
    }
    if started.elapsed() > WAIT_DEADLINE {
        return false;
    }

    // (b) A spawned thread sets the value and notifies; the wait must observe it.
    let worker = {
        let shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("std-probe-sync".to_string())
            .stack_size(THREAD_STACK_BYTES)
            .spawn(move || {
                let (mutex, condvar) = &*shared;
                let mut guard = match mutex.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                *guard = 42;
                condvar.notify_one();
            })
    };
    let worker = match worker {
        Ok(handle) => handle,
        Err(_) => return false,
    };

    let mut observed = false;
    {
        let (mutex, condvar) = &*shared;
        let mut guard = match mutex.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        for _ in 0..WAIT_STEPS {
            if *guard == 42 {
                observed = true;
                break;
            }
            guard = match condvar.wait_timeout(guard, WAIT_STEP) {
                Ok((guard, _timed_out)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
        if *guard != 42 {
            observed = false;
        }
    }

    if worker.join().is_err() {
        return false;
    }
    if !observed {
        return false;
    }
    // Bind the result so the guard is dropped before `shared` is, rather than
    // in the tail expression where it outlives the local.
    let unlocked = match shared.0.try_lock() {
        Ok(guard) => *guard == 42,
        Err(_) => false,
    };
    unlocked
}

// ---------------------------------------------------------------------------
// 6. thread_local! destructors on a spawned thread
// ---------------------------------------------------------------------------

static DTOR_RAN: AtomicBool = AtomicBool::new(false);

struct DtorFlag;

impl Drop for DtorFlag {
    fn drop(&mut self) {
        DTOR_RAN.store(true, Ordering::SeqCst);
    }
}

thread_local! {
    static DTOR_SLOT: std::cell::Cell<Option<DtorFlag>> = const { std::cell::Cell::new(None) };
}

fn check_tls_destructor() -> bool {
    DTOR_RAN.store(false, Ordering::SeqCst);

    let handle = match std::thread::Builder::new()
        .name("std-probe-dtor".to_string())
        .stack_size(THREAD_STACK_BYTES)
        .spawn(|| {
            DTOR_SLOT.with(|slot| slot.set(Some(DtorFlag)));
        }) {
        Ok(handle) => handle,
        Err(_) => return false,
    };

    if handle.join().is_err() {
        return false;
    }
    // The destructor runs when the thread exits, i.e. before join returns.
    DTOR_RAN.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------------------
// Compile-time proof that the *patched* libc layout is in the graph
// ---------------------------------------------------------------------------

/// These assertions are the build-time gate for `patches/libc-switch/`: if the
/// `[patch.crates-io]` entry above stops feeding the graph, these fail with the
/// upstream layouts (mutex 40/8, socket storage 28/2), so a build cannot
/// silently link `std` against the wrong `pthread_*` sizes.
#[cfg(feature = "patched-libc")]
mod patched_abi_assertions {
    const _: () = {
        assert!(std::mem::size_of::<libc::pthread_mutex_t>() == 12);
        assert!(std::mem::align_of::<libc::pthread_mutex_t>() == 4);
        assert!(std::mem::size_of::<libc::pthread_cond_t>() == 16);
        assert!(std::mem::align_of::<libc::pthread_cond_t>() == 8);
        assert!(std::mem::size_of::<libc::sockaddr_storage>() == 128);
        assert!(std::mem::align_of::<libc::sockaddr_storage>() == 8);
        assert!(libc::_SC_PAGESIZE == 8);
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_checks_and_failure_reporting_on_host() {
        // Host validation of the harness only; this is not Switch execution.
        let result = craftrustnx_std_probe_run(KNOWN_CHECKS);
        assert_eq!(result & RESULT_SIGNATURE_MASK, RESULT_SIGNATURE_VALUE);
        assert_eq!((result >> 16) & 0xffff, u64::from(KNOWN_CHECKS));
        assert_eq!((result >> 32) & 0xffff, 0);
        let repeated = craftrustnx_std_probe_run(CHECK_TLS_MAIN);
        assert_eq!((repeated >> 16) & 0xffff, u64::from(CHECK_TLS_MAIN));
        assert_eq!((repeated >> 32) & 0xffff, 0);
        let unknown = craftrustnx_std_probe_run(0x8000);
        assert_eq!((unknown >> 16) & 0xffff, 0);
        assert_eq!((unknown >> 32) & 0xffff, 0x8000);
        assert_eq!(craftrustnx_std_probe_run(1 << 31), 0);
        assert_eq!((craftrustnx_std_probe_run(0) >> 16) & 0xffff_ffff, 0);
    }
}
