#!/usr/bin/env bash
# Host ABI check for the frozen CTCOL001 snapshot codec. Cargo is offline and
# locked. ASan/UBSan instrument the C caller only; the Rust staticlib is not
# built with sanitizers (no Rust instrumentation is claimed).
set -euo pipefail
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
probe="$(cd -- "$here/.." && pwd)"
root="$(cd -- "$probe/../../.." && pwd)"
codec="$probe/codec"

unset CARGO_ENCODED_RUSTFLAGS RUSTFLAGS
cd "$root"
cargo build --offline --locked --release --jobs 1 \
  --manifest-path "$codec/Cargo.toml" \
  --target-dir "$codec/build/host-target"

lib="$codec/build/host-target/release/libcraftrustnx_login_codec.a"
if [[ ! -s "$lib" ]]; then
  echo "staticlib missing: $lib" >&2
  exit 1
fi

mkdir -p "$here/build"
binary="$here/build/snapshot-host"
echo "Linking $binary (ASan/UBSan on C only; Rust staticlib not instrumented)."
echo "The staticlib already carries miniz_oxide; no separate miniz archive."
clang -std=c11 -O1 -g -Wall -Wextra -Werror -fno-omit-frame-pointer \
  -fsanitize=address,undefined \
  -I "$probe/include" \
  "$here/snapshot_host.c" "$lib" \
  -o "$binary"

export ASAN_OPTIONS="${ASAN_OPTIONS:-detect_leaks=0:halt_on_error=1}"
export UBSAN_OPTIONS="${UBSAN_OPTIONS:-halt_on_error=1:print_stacktrace=1}"
"$binary"
