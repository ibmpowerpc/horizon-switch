#!/usr/bin/env bash
set -euo pipefail

# Experiment only: rebuild core with the installed compiler, not a std port.
probe_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$probe_dir"
[[ "$(rustc --version | awk '{print $2}')" == 1.100.0 ]] || {
  echo 'This experiment requires the installed rustc 1.100.0.' >&2; exit 1;
}
rust_sysroot="$(rustc --print sysroot)"
[[ -f "$rust_sysroot/lib/rustlib/src/rust/library/core/Cargo.toml" ]] || {
  echo 'Matching rust-src is missing; no download attempted.' >&2; exit 1;
}

check_space() {
  local free_kib artifacts_kib=0
  free_kib="$(df -Pk "$probe_dir" | awk 'NR==2 {print $4}')"
  [[ "$free_kib" -ge 10485760 ]] || { echo 'Less than 10 GiB free; stopped.' >&2; exit 1; }
  if [[ -d "$probe_dir/target" ]]; then
    artifacts_kib="$(du -sk "$probe_dir/target" | awk '{print $1}')"
  fi
  [[ "$artifacts_kib" -lt 524288 ]] || { echo 'Rust artifacts reached 512 MiB; stopped.' >&2; exit 1; }
}

check_space
# Keep all overrides scoped to this script, with no toolchain installation.
unset CARGO_ENCODED_RUSTFLAGS
export CARGO_TARGET_DIR="$probe_dir/target"
export RUSTFLAGS='-C target-cpu=cortex-a57 -C relocation-model=pic'
RUSTC_BOOTSTRAP=1 cargo -Z build-std=core build --offline --locked \
  --target aarch64-nintendo-switch-freestanding --release --jobs 1
check_space
test -s "$CARGO_TARGET_DIR/aarch64-nintendo-switch-freestanding/release/libcraftrustnx_rust_probe.a"
du -sh "$CARGO_TARGET_DIR"
