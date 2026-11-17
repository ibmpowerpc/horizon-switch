#!/usr/bin/env bash
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd -- "$script_dir/../.." && pwd)"
python3 "$repo_dir/platform/switch/check-budget.py" --reserve-mib 32
cargo build --offline --locked --jobs 1 --manifest-path "$script_dir/Cargo.toml" \
  --target-dir "$script_dir/build/host-target"
"$script_dir/build/host-target/debug/craftrustnx-model-prep" "$@"
python3 "$repo_dir/platform/switch/check-budget.py"
