#!/usr/bin/env bash
set -euo pipefail
probe_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd "$probe_dir/../../.." && pwd)"
[[ "$(rustc --version | awk '{print $2}')" == 1.100.0 ]] || {
  echo 'Installed rustc 1.100.0 is required; no download attempted.' >&2; exit 1;
}
check_budget() {
  python3 - "$project_dir" "$1" <<'PY'
import pathlib, shutil, subprocess, sys
root = pathlib.Path(sys.argv[1])
patterns = ('platform/switch/*/target', 'platform/switch/std-probe/build',
            'platform/switch/net-probe/*/target', 'platform/switch/net-probe/*/build',
            'platform/switch/probe/build*', 'platform/switch/probe/source-net/build',
            'tools/switch-abi-audit/build', 'tools/switch-abi-audit/rust/build')
paths = {p for pattern in patterns for p in root.glob(pattern) if p.is_dir()}
kib = sum(int(subprocess.check_output(['du', '-sk', str(p)], text=True).split()[0]) for p in paths)
reserve = int(sys.argv[2])
free = shutil.disk_usage(root).free
print(f'Experiment artifacts: {kib / 1024:.1f} MiB; free: {free / 2**30:.1f} GiB')
if free < 10 * 2**30 + reserve * 1024 or kib + reserve > 512 * 1024:
    sys.exit('Insufficient space within 10 GiB reserve / 512 MiB experiment budget.')
PY
}
# Reuse the existing core cache: target, flags and profile match rust-probe.
# This is an estimate plus a post-build guard, not a filesystem quota.
check_budget 16384
unset CARGO_ENCODED_RUSTFLAGS
export CARGO_TARGET_DIR="$project_dir/platform/switch/rust-probe/target"
export RUSTFLAGS='-C target-cpu=cortex-a57 -C relocation-model=pic'
cd "$probe_dir/ffi"
RUSTC_BOOTSTRAP=1 cargo -Z build-std=core build --offline --locked \
  --target aarch64-nintendo-switch-freestanding --release --jobs 1
check_budget 0
test -s "$CARGO_TARGET_DIR/aarch64-nintendo-switch-freestanding/release/libcraftrustnx_status_ffi.a"
