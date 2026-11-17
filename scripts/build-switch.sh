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
  gpu)
    bash "$project_dir/platform/switch/gpu-probe/build.sh"
    target=craftrustnx-gpu-probe
    archive_rel=platform/switch/rust-probe/target/aarch64-nintendo-switch-freestanding/release/libcraftrustnx_gpu_probe.a
    native_args=("GPU_ARCHIVE=$project_dir/$archive_rel")
    docker_args=("GPU_ARCHIVE=/workspace/$archive_rel")
    ;;
  terrain)
    bash "$project_dir/platform/switch/terrain-probe/build.sh"
    target=craftrustnx-terrain-probe
    archive_rel=platform/switch/rust-probe/target/aarch64-nintendo-switch-freestanding/release/libcraftrustnx_terrain_probe.a
    native_args=("TERRAIN_ARCHIVE=$project_dir/$archive_rel")
    docker_args=("TERRAIN_ARCHIVE=/workspace/$archive_rel")
    ;;
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
  login|encrypted-login|configuration|play-session|world-data|column-snapshot)
    bash "$project_dir/platform/switch/login-probe/build.sh"
    target=craftrustnx-login-probe
    archive_rel=platform/switch/rust-probe/target/aarch64-nintendo-switch-freestanding/release/libcraftrustnx_login_codec.a
    native_args=("LOGIN_ARCHIVE=$project_dir/$archive_rel")
    docker_args=("LOGIN_ARCHIVE=/workspace/$archive_rel")
    if [[ "$variant" != login ]]; then
      target=craftrustnx-encrypted-login-probe
      native_args+=(ENCRYPTED_LOGIN=1)
      docker_args+=(ENCRYPTED_LOGIN=1)
      if [[ "$variant" == configuration || "$variant" == play-session || "$variant" == world-data || "$variant" == column-snapshot ]]; then
        target=craftrustnx-configuration-probe
        native_args+=(CONFIGURATION_PROBE=1)
        docker_args+=(CONFIGURATION_PROBE=1)
        if [[ "$variant" == play-session || "$variant" == world-data || "$variant" == column-snapshot ]]; then
          target=craftrustnx-play-session-probe
          native_args+=(PLAY_SESSION=1)
          docker_args+=(PLAY_SESSION=1)
          if [[ "$variant" == world-data || "$variant" == column-snapshot ]]; then
            target=craftrustnx-world-data-probe
            native_args+=(WORLD_DATA=1)
            docker_args+=(WORLD_DATA=1)
            if [[ "$variant" == column-snapshot ]]; then
              target=craftrustnx-column-snapshot-probe
              native_args+=(COLUMN_SNAPSHOT=1)
              docker_args+=(COLUMN_SNAPSHOT=1)
            fi
          fi
        fi
      fi
    fi
    ;;
  *) echo 'Variant must be hardware, rust, std, net, login, encrypted-login, configuration, play-session, world-data, column-snapshot, gpu or terrain.' >&2; exit 2 ;;
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
    echo 'Usage: bash scripts/build-switch.sh [docker|native] [hardware|rust|std|net|login|encrypted-login|configuration|play-session|world-data|column-snapshot|gpu|terrain]' >&2
    exit 2
    ;;
esac

mkdir -p "$project_dir/dist/switch/craftrustnx"
cp "$probe_dir/$target.nro" "$project_dir/dist/switch/craftrustnx/"
if [[ "$variant" == gpu ]]; then
  cp "$project_dir/platform/switch/gpu-probe/SDK-NOTICES.txt" \
    "$project_dir/dist/switch/craftrustnx/craftrustnx-gpu-notices.txt"
fi
if [[ "$variant" == terrain ]]; then
  cp "$project_dir/platform/switch/terrain-probe/SDK-NOTICES.txt" \
    "$project_dir/dist/switch/craftrustnx/craftrustnx-terrain-notices.txt"
  mesh="$project_dir/.local/switch-models/26.3/terrain-probe-mesh.bin"
  if [[ -f "$mesh" ]]; then
    cp "$mesh" "$project_dir/dist/switch/craftrustnx/terrain-probe-mesh.bin"
  fi
fi
echo "NRO: $project_dir/dist/switch/craftrustnx/$target.nro"
python3 "$project_dir/platform/switch/check-budget.py"
