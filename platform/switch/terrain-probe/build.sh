#!/usr/bin/env bash
set -euo pipefail
probe_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd -- "$probe_dir/../../.." && pwd)"
[[ "$(rustc --version | awk '{print $2}')" == 1.100.0 ]] || {
  echo 'Installed rustc 1.100.0 is required; no download attempted.' >&2; exit 1;
}
rust_sysroot="$(rustc --print sysroot)"
[[ -f "$rust_sysroot/lib/rustlib/src/rust/library/core/Cargo.toml" ]] || {
  echo 'Matching rust-src is missing; no download attempted.' >&2; exit 1;
}
python3 "$project_dir/platform/switch/check-budget.py" --reserve-mib 32
unset CARGO_ENCODED_RUSTFLAGS
export CARGO_TARGET_DIR="$project_dir/platform/switch/rust-probe/target"
export RUSTFLAGS='-C target-cpu=cortex-a57 -C relocation-model=pic'
cd "$probe_dir/rust"
RUSTC_BOOTSTRAP=1 cargo -Z build-std=core build --offline --locked \
  --target aarch64-nintendo-switch-freestanding --release --jobs 1
test -s "$CARGO_TARGET_DIR/aarch64-nintendo-switch-freestanding/release/libcraftrustnx_terrain_probe.a"
python3 "$project_dir/platform/switch/check-budget.py"
