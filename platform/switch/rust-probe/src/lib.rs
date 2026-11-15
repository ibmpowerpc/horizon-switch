//! Rust no_std staticlib for Switch C ABI smoke test
//!
//! This is a bootstrap experiment, NOT a production std port.
//! It uses only core functionality and no runtime dependencies.

#![no_std]

// Panic handler: calls C abort (linked later through libnx)
extern "C" {
    fn abort() -> !;
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe { abort() }
}

/// C ABI compatible function: returns input.wrapping_mul(0x9e3779b97f4a7c15).rotate_left(17) ^ 0x4352414654525553
#[unsafe(no_mangle)]
pub extern "C" fn craftrustnx_rust_mix(input: u64) -> u64 {
    input
        .wrapping_mul(0x9e3779b97f4a7c15)
        .rotate_left(17)
        ^ 0x4352414654525553
}
