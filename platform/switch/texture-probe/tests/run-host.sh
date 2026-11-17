#!/usr/bin/env bash
# Host ABI check. Cargo is offline and locked. ASan/UBSan instrument the C
# harness only; the Rust staticlib is not rebuilt with sanitizers.
set -euo pipefail
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd -- "$here/../../../.." && pwd)"
probe="$root/platform/switch/texture-probe"
unset CARGO_ENCODED_RUSTFLAGS RUSTFLAGS
cd "$root"
cargo build --offline --locked --release --features textured --jobs 1 \
  --manifest-path platform/switch/terrain-probe/rust/Cargo.toml \
  --target-dir platform/switch/texture-probe/build/host-target
lib="$probe/build/host-target/release/libcraftrustnx_terrain_probe.a"
if [[ ! -s "$lib" ]]; then
  echo "staticlib missing: $lib" >&2
  exit 1
fi
mkdir -p "$probe/build"
echo "Linking host_check: ASan/UBSan on C only; Rust staticlib is not instrumented."
clang -std=c11 -Wall -Wextra -Werror -g -fno-omit-frame-pointer \
  -fsanitize=address,undefined \
  -I "$probe/include" \
  -I "$root/platform/switch/terrain-probe/include" \
  "$probe/tests/host_check.c" "$lib" -lm \
  -o "$probe/build/host-check"
export ASAN_OPTIONS="${ASAN_OPTIONS:-detect_leaks=0:halt_on_error=1}"
export UBSAN_OPTIONS="${UBSAN_OPTIONS:-halt_on_error=1:print_stacktrace=1}"
scene="$root/.local/switch-models/26.3/texture-probe-scene.bin"
if [[ $# -gt 0 ]]; then
  "$probe/build/host-check" "$@"
elif [[ -f "$scene" ]]; then
  "$probe/build/host-check" "$scene"
else
  echo "Optional scene absent; synthetic fixtures only."
  "$probe/build/host-check"
fi
