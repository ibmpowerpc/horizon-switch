#!/usr/bin/env bash
set -euo pipefail
probe_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd -- "$probe_dir/../../.." && pwd)"

# The ABI tool stages only a local, hash-checked copy; no dependency downloads.
bash "$repo_dir/patches/libc-switch/apply.sh"
bash "$probe_dir/preflight.sh"
bash "$probe_dir/build/next-build.sh"

# Report all experiment outputs; the user removed the old fixed disk caps.
python3 "$repo_dir/platform/switch/check-budget.py"
