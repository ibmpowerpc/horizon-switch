#!/usr/bin/env bash
# Compile-measured Rust ABI probe for the *locally patched* libc 0.2.189
# (patches/libc-switch/) against the already measured Switch C ABI.
#
# Compile-only: target code is never executed, nothing is linked into a NRO.
# The unpatched baseline (`./run.sh`, outputs in `build/`) is untouched and can
# be re-run at any time; this mode writes only to `build/patched/`.
#
# Works from any current directory. Reuses the same RUSTFLAGS, release profile
# and CARGO_TARGET_DIR as the baseline probe, so build-std=core artifacts are
# shared instead of rebuilt.
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
audit_dir="$(cd -- "$script_dir/.." && pwd)"
repo_dir="$(cd -- "$audit_dir/../.." && pwd)"
build_dir="$script_dir/build/patched"
patched_crate_dir="$script_dir/patched"
libc_stage="$build_dir/libc-0.2.189"
target_dir="$repo_dir/platform/switch/rust-probe/target"
staticlib="libswitch_abi_probe_rust_patched.a"
lib="$target_dir/aarch64-nintendo-switch-freestanding/release/$staticlib"
image="devkitpro/devkita64:20260219"
c_json="$audit_dir/build/abi.json"
apply_script="$repo_dir/patches/libc-switch/apply.sh"
patch_file="$repo_dir/patches/libc-switch/libc-0.2.189-switch-aarch64.patch"
rustflags='-C target-cpu=cortex-a57 -C relocation-model=pic --cfg unix --cfg target_family="unix" --cfg target_env="newlib" -Aexplicit_builtin_cfgs_in_flags'

mkdir -p "$build_dir"

command -v cargo >/dev/null 2>&1 || { echo "cargo is required." >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "python3 is required." >&2; exit 1; }
command -v docker >/dev/null 2>&1 || { echo "Docker is required." >&2; exit 1; }
docker info >/dev/null 2>&1 || { echo "Start Docker, then retry." >&2; exit 1; }
[ -f "$c_json" ] || { echo "Missing C measurement: $c_json (run ../run.sh first)." >&2; exit 1; }
[ -f "$apply_script" ] || { echo "Missing patch script: $apply_script" >&2; exit 1; }
[ -f "$patch_file" ] || { echo "Missing patch: $patch_file" >&2; exit 1; }

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

check_space() {
  python3 "$repo_dir/platform/switch/check-budget.py" --reserve-mib 32
}

check_space

# The comparison is only meaningful with the baseline's exact compiler flags.
if [ -f "$script_dir/build/env.txt" ]; then
  baseline_flags="$(awk -F= '/^rustflags=/ {sub(/^rustflags=/, ""); print; exit}' "$script_dir/build/env.txt")"
  if [ -n "$baseline_flags" ] && [ "$baseline_flags" != "$rustflags" ]; then
    echo "RUSTFLAGS differ from the unpatched baseline; refusing to compare." >&2
    echo "baseline: $baseline_flags" >&2
    echo "patched:  $rustflags" >&2
    exit 1
  fi
fi

# Stage the patched libc from the offline registry copy (idempotent, hashed).
bash "$apply_script" --dest "$libc_stage"

# Offline lock file for the patched crate; the only dependency is a path.
if [[ ! -f "$patched_crate_dir/Cargo.lock" ]]; then
  (cd "$patched_crate_dir" && cargo generate-lockfile --offline)
fi

# Process-local only: no global config, no toolchain installation.
unset CARGO_ENCODED_RUSTFLAGS
(
  cd "$patched_crate_dir"
  env CARGO_TARGET_DIR="$target_dir" \
      RUSTC_BOOTSTRAP=1 \
      RUSTFLAGS="$rustflags" \
      cargo -Z build-std=core build --offline --locked \
        --features patched-libc \
        --target aarch64-nintendo-switch-freestanding --release --jobs 1
)

[ -s "$lib" ] || { echo "Build did not produce $lib" >&2; exit 1; }

image_id="$(docker image inspect "$image" --format '{{.Id}}')"
repo_digest="$(docker image inspect "$image" --format '{{if .RepoDigests}}{{index .RepoDigests 0}}{{end}}')"
patch_sha="$(python3 -c 'import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$patch_file")"
{
  echo "rustc=$rustc_version"
  echo "cargo=$(cargo --version)"
  echo "rust_sysroot=$rust_sysroot"
  echo "rust_src=$rust_sysroot/lib/rustlib/src/rust"
  echo "libc_version=0.2.189"
  echo "libc_source=$libc_stage"
  echo "libc_registry_source_unmodified=$(awk -F= '/^source=/ {print $2; exit}' "$libc_stage/.switch-patch-applied")"
  echo "libc_patched=true"
  echo "patch=$patch_file"
  echo "patch_sha256=$patch_sha"
  echo "patch_stage=$libc_stage"
  echo "target=aarch64-nintendo-switch-freestanding"
  echo "cfg=unix,target_family=\"unix\",target_env=\"newlib\""
  echo "rustflags=$rustflags"
  echo "cargo_flags=-Z build-std=core --offline --locked --release --jobs 1 --features patched-libc"
  echo "cargo_target_dir=$target_dir"
  echo "staticlib=$lib"
  echo "image=$image"
  echo "image_id=$image_id"
  echo "image_repo_digest=$repo_digest"
  echo "date_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$build_dir/env.txt"

docker run --rm --pull=never --network none --user "$(id -u):$(id -g)" \
  --mount "type=bind,source=$repo_dir,target=/workspace" \
  --workdir /workspace \
  --env WORKSPACE=/workspace \
  --env ABI_OUT_SUBDIR=/patched \
  --env ABI_STATICLIB="$staticlib" \
  "$image" /bin/bash /workspace/tools/switch-abi-audit/rust/extract-container.sh

python3 "$script_dir/decode_rust_abi.py" \
  --def "$audit_dir/abi_items.def" \
  --bin "$build_dir/rust_abi_table.bin" \
  --obj "$build_dir/rust_abi_probe.o" \
  --c-json "$c_json" \
  --env "$build_dir/env.txt" \
  --json "$build_dir/rust-abi.json" \
  --report "$build_dir/comparison.txt" \
  --strict

check_space
size_kib="$(du -sk "$build_dir" | awk '{print $1}')"
echo
echo "Artifacts: $build_dir (${size_kib} KiB)"
echo "JSON:      $build_dir/rust-abi.json"
echo "Report:    $build_dir/comparison.txt"
echo "Baseline (unpatched) evidence left in: $script_dir/build"
