#!/usr/bin/env bash
# Compile-measured Rust ABI probe for libc 0.2.189 (unpatched) against the
# already measured Switch C ABI. Compile-only: target code is never executed.
#
# Works from any current directory. Builds the Rust staticlib into the shared
# experiment target dir, extracts .switch_abi with devkitA64 objcopy in Docker,
# then compares the compiled constants with tools/switch-abi-audit/build/abi.json.
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
audit_dir="$(cd -- "$script_dir/.." && pwd)"
repo_dir="$(cd -- "$audit_dir/../.." && pwd)"
build_dir="$script_dir/build"
target_dir="$repo_dir/platform/switch/rust-probe/target"
lib="$target_dir/aarch64-nintendo-switch-freestanding/release/libswitch_abi_probe_rust.a"
image="devkitpro/devkita64:20260219"
c_json="$audit_dir/build/abi.json"

mkdir -p "$build_dir"

command -v cargo >/dev/null 2>&1 || { echo "cargo is required." >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "python3 is required." >&2; exit 1; }
command -v docker >/dev/null 2>&1 || { echo "Docker is required." >&2; exit 1; }
docker info >/dev/null 2>&1 || { echo "Start Docker, then retry." >&2; exit 1; }
[ -f "$c_json" ] || { echo "Missing C measurement: $c_json (run ../run.sh first)." >&2; exit 1; }

rustc_version="$(rustc --version)"
[[ "$rustc_version" == *" 1.100.0 "* ]] || {
  echo "This probe requires the installed rustc 1.100.0; found: $rustc_version" >&2
  exit 1
}
rust_sysroot="$(rustc --print sysroot)"
[[ -f "$rust_sysroot/lib/rustlib/src/rust/library/core/Cargo.toml" ]] || {
  echo "Matching rust-src is missing; no download attempted." >&2
  exit 1
}
libc_src="$(find "$HOME/.cargo/registry/src" -maxdepth 2 -type d -name 'libc-0.2.189' -print -quit)"
[ -n "$libc_src" ] || { echo "libc 0.2.189 source is missing from the offline Cargo cache." >&2; exit 1; }

check_space() {
  python3 "$repo_dir/platform/switch/check-budget.py" --reserve-mib 32
}

check_space

# Generate the lock file offline only if it is absent; no network access.
if [[ ! -f "$script_dir/Cargo.lock" ]]; then
  (cd "$script_dir" && cargo generate-lockfile --offline)
fi

# Process-local only: no global config, no toolchain installation.
unset CARGO_ENCODED_RUSTFLAGS
rustflags='-C target-cpu=cortex-a57 -C relocation-model=pic --cfg unix --cfg target_family="unix" --cfg target_env="newlib" -Aexplicit_builtin_cfgs_in_flags'
(
  cd "$script_dir"
  env CARGO_TARGET_DIR="$target_dir" \
      RUSTC_BOOTSTRAP=1 \
      RUSTFLAGS="$rustflags" \
      cargo -Z build-std=core build --offline --locked \
        --target aarch64-nintendo-switch-freestanding --release --jobs 1
)

[ -s "$lib" ] || { echo "Build did not produce $lib" >&2; exit 1; }

image_id="$(docker image inspect "$image" --format '{{.Id}}')"
repo_digest="$(docker image inspect "$image" --format '{{if .RepoDigests}}{{index .RepoDigests 0}}{{end}}')"
{
  echo "rustc=$rustc_version"
  echo "cargo=$(cargo --version)"
  echo "rust_sysroot=$rust_sysroot"
  echo "rust_src=$rust_sysroot/lib/rustlib/src/rust"
  echo "libc_version=0.2.189"
  echo "libc_source=$libc_src"
  echo "libc_patched=false"
  echo "target=aarch64-nintendo-switch-freestanding"
  echo "cfg=unix,target_family=\"unix\",target_env=\"newlib\""
  echo "rustflags=$rustflags"
  echo "cargo_flags=-Z build-std=core --offline --locked --release --jobs 1"
  echo "cargo_target_dir=$target_dir"
  echo "image=$image"
  echo "image_id=$image_id"
  echo "image_repo_digest=$repo_digest"
  echo "date_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$build_dir/env.txt"

docker run --rm --pull=never --network none --user "$(id -u):$(id -g)" \
  --mount "type=bind,source=$repo_dir,target=/workspace" \
  --workdir /workspace \
  --env WORKSPACE=/workspace \
  "$image" /bin/bash /workspace/tools/switch-abi-audit/rust/extract-container.sh

python3 "$script_dir/decode_rust_abi.py" \
  --def "$audit_dir/abi_items.def" \
  --bin "$build_dir/rust_abi_table.bin" \
  --obj "$build_dir/rust_abi_probe.o" \
  --c-json "$c_json" \
  --env "$build_dir/env.txt" \
  --json "$build_dir/rust-abi.json" \
  --report "$build_dir/comparison.txt"

check_space
size_kib="$(du -sk "$build_dir" | awk '{print $1}')"
echo
echo "Artifacts: $build_dir (${size_kib} KiB)"
echo "JSON:      $build_dir/rust-abi.json"
