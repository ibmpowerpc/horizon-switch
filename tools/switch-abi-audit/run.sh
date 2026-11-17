#!/usr/bin/env bash
# Reproducible, offline, compile-only Switch C ABI audit.
# Works from any current directory. All generated files go to ./build/.
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd -- "$script_dir/../.." && pwd)"
build_dir="$script_dir/build"
image="devkitpro/devkita64:20260219"

mkdir -p "$build_dir"

command -v docker >/dev/null 2>&1 || { echo "Docker is required." >&2; exit 1; }
docker info >/dev/null 2>&1 || { echo "Start Docker, then retry." >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "python3 is required." >&2; exit 1; }

python3 "$repo_dir/platform/switch/check-budget.py" --reserve-mib 1

image_id="$(docker image inspect "$image" --format '{{.Id}}')"
repo_digest="$(docker image inspect "$image" --format '{{if .RepoDigests}}{{index .RepoDigests 0}}{{end}}')"
{
  echo "image=$image"
  echo "image_id=$image_id"
  echo "image_repo_digest=$repo_digest"
} > "$build_dir/image.txt"

docker run --rm --pull=never --network none --user "$(id -u):$(id -g)" \
  --mount "type=bind,source=$repo_dir,target=/workspace" \
  --workdir /workspace/tools/switch-abi-audit \
  "$image" /bin/bash /workspace/tools/switch-abi-audit/container-build.sh

python3 "$script_dir/decode.py" \
  --def "$script_dir/abi_items.def" \
  --bin "$build_dir/abi_table.bin" \
  --env "$build_dir/env.txt" \
  --image "$build_dir/image.txt" \
  --libc-src "${LIBC_SRC:-}" \
  --json "$build_dir/abi.json" \
  --report "$build_dir/report.txt"

cat "$build_dir/report.txt"

size_kib="$(du -sk "$build_dir" | awk '{ print $1 }')"
if [ "$size_kib" -gt 5120 ]; then
  echo "ERROR: build artifacts exceed 5 MiB ($size_kib KiB)." >&2
  exit 1
fi
echo
echo "Artifacts: $build_dir (${size_kib} KiB)"
echo "JSON:      $build_dir/abi.json"
