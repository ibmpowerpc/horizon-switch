#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
probe_dir="$project_dir/platform/switch/probe"
variant="${2:-hardware}"
target=craftrustnx-probe
native_args=(all)
docker_args=(all)
case "$variant" in
  hardware) ;;
  rust)
    bash "$project_dir/platform/switch/rust-probe/build.sh"
    target=craftrustnx-rust-probe
    archive_rel=platform/switch/rust-probe/target/aarch64-nintendo-switch-freestanding/release/libcraftrustnx_rust_probe.a
    native_args=("RUST_ARCHIVE=$project_dir/$archive_rel")
    docker_args=("RUST_ARCHIVE=/workspace/$archive_rel")
    ;;
  std)
    bash "$project_dir/platform/switch/std-probe/build.sh"
    target=craftrustnx-std-probe
    archive_rel=platform/switch/std-probe/build/target/switch-std/release/libcraftrustnx_std_probe.a
    native_args=("STD_ARCHIVE=$project_dir/$archive_rel")
    docker_args=("STD_ARCHIVE=/workspace/$archive_rel")
    ;;
  net)
    bash "$project_dir/platform/switch/net-probe/build.sh"
    target=craftrustnx-net-probe
    archive_rel=platform/switch/rust-probe/target/aarch64-nintendo-switch-freestanding/release/libcraftrustnx_status_ffi.a
    native_args=("NET_ARCHIVE=$project_dir/$archive_rel")
    docker_args=("NET_ARCHIVE=/workspace/$archive_rel")
    ;;
  *) echo 'Variant must be hardware, rust, std or net.' >&2; exit 2 ;;
esac

case "${1:-docker}" in
  native)
    : "${DEVKITPRO:?Set DEVKITPRO to your devkitPro installation}"
    : "${DEVKITA64:?Set DEVKITA64 to your devkitA64 installation}"
    make -C "$probe_dir" "${native_args[@]}"
    ;;
  docker)
    command -v docker >/dev/null || { echo 'Docker is required.' >&2; exit 1; }
    docker info >/dev/null 2>&1 || { echo 'Start Docker, then retry.' >&2; exit 1; }
    docker run --rm --pull=never --network none --user "$(id -u):$(id -g)" \
      --mount "type=bind,source=$project_dir,target=/workspace" \
      --workdir /workspace/platform/switch/probe \
      devkitpro/devkita64:20260219 make "${docker_args[@]}"
    ;;
  *)
    echo 'Usage: bash scripts/build-switch.sh [docker|native] [hardware|rust|std|net]' >&2
    exit 2
    ;;
esac

mkdir -p "$project_dir/dist/switch/craftrustnx"
cp "$probe_dir/$target.nro" "$project_dir/dist/switch/craftrustnx/"
echo "NRO: $project_dir/dist/switch/craftrustnx/$target.nro"
