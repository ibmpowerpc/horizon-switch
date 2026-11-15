#!/usr/bin/env bash
set -euo pipefail
probe_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd -- "$probe_dir/../../.." && pwd)"

# The ABI tool stages only a local, hash-checked copy; no dependency downloads.
bash "$repo_dir/patches/libc-switch/apply.sh"
bash "$probe_dir/preflight.sh"
bash "$probe_dir/build/next-build.sh"

# Account for all experiment targets, not just this crate's output.
python3 - "$repo_dir" <<'PY'
from pathlib import Path
import shutil, subprocess, sys
root = Path(sys.argv[1])
paths = (
    'platform/switch/rust-probe/target', 'platform/switch/std-probe/build/target',
    'platform/switch/std-probe/build/host-target', 'platform/switch/std-probe/target',
    'platform/switch/net-probe/target', 'platform/switch/net-probe/codec/target',
    'tools/switch-abi-audit/rust/build',
    'platform/switch/probe/build', 'platform/switch/probe/build-rust',
    'platform/switch/probe/build-std',
)
kib = sum(int(subprocess.check_output(['du', '-sk', str(root/p)]).split()[0])
          for p in paths if (root/p).is_dir())
if kib > 512*1024 or shutil.disk_usage(root).free < 10*1024**3:
    sys.exit('Experiment disk limit exceeded; stop and inspect task artifacts.')
print(f'Experiment artifacts: {kib/1024:.1f} MiB / 512 MiB')
PY
