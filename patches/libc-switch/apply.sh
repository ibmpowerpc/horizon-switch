#!/usr/bin/env bash
# Stage a project-local, patched copy of the offline-cached libc 0.2.189 crate
# for the Nintendo Switch (aarch64-none-elf newlib/libnx) ABI work.
#
# The Cargo registry source, rust-src and the toolchain are never modified:
# the crate is copied into an ignored project build directory and patched
# there. Version, source-file hashes and patch hash are verified before and
# after patching, so this script refuses to run against a different or already
# modified crate.
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd -- "$script_dir/../.." && pwd)"

crate_name="libc"
crate_version="0.2.189"

# sha256 of the pristine registry files this patch modifies, and of the results.
# Recorded from the offline Cargo cache; a mismatch means a different source.
expect_src_newlib="6bbe0874c5d6f53ea2d9f2a7cb1b27bdb27b9b02704160e6133ea6e559ad66cd"
expect_src_horizon="329f41f1703f318ea8999c2a9f48b953381e9a4fc388670ce69444ad7a32b122"
expect_out_newlib="3aa7a7a7344914b903e8cf21c7679a59e4a5ac1c5b356e3ce95b9bf794362990"
expect_out_horizon="8c35ee8338379aa9fd0108cc72f5c36a3cde33e1887376bb52ab93bc00aa732c"

patch_file="$script_dir/$crate_name-$crate_version-switch-aarch64.patch"
dest_default="$repo_dir/tools/switch-abi-audit/rust/build/patched/$crate_name-$crate_version"
marker_name=".switch-patch-applied"

usage() {
  echo "usage: $0 [--source DIR] [--dest DIR] [--check-only]" >&2
  echo "  --source     pristine $crate_name-$crate_version crate directory" >&2
  echo "               (default: first match in \$HOME/.cargo/registry/src)" >&2
  echo "  --dest       staging directory (default: $dest_default)" >&2
  echo "  --check-only verify the patch applies, do not write the staging copy" >&2
}

source_dir=""
dest_dir="$dest_default"
check_only=0
while [ $# -gt 0 ]; do
  case "$1" in
    --source) source_dir="${2:?}"; shift 2 ;;
    --dest) dest_dir="${2:?}"; shift 2 ;;
    --check-only) check_only=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage; exit 2 ;;
  esac
done

command -v python3 >/dev/null 2>&1 || { echo "python3 is required." >&2; exit 1; }
command -v patch >/dev/null 2>&1 || { echo "patch is required." >&2; exit 1; }

sha256() {
  python3 - "$1" <<'PY'
import hashlib, sys, pathlib
print(hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest())
PY
}

fail() { echo "apply.sh: $*" >&2; exit 1; }

[ -f "$patch_file" ] || fail "missing patch: $patch_file"

# Restaging removes a generated copy. Never permit a custom --dest to delete
# the registry, the repository root, or an unrelated directory.
python3 - "$dest_dir" "$repo_dir/tools/switch-abi-audit/rust/build/patched" <<'PY'
from pathlib import Path
import sys
dest = Path(sys.argv[1]).resolve()
root = Path(sys.argv[2]).resolve()
if dest == root or root not in dest.parents:
    sys.exit('apply.sh: --dest must be strictly inside the generated patched build directory')
if dest.exists() and not (dest / '.switch-patch-applied').is_file():
    sys.exit('apply.sh: refusing to replace a directory without our staging marker')
PY

# --- version and source provenance guards -----------------------------------
if [ -z "$source_dir" ]; then
  source_dir="$(find "$HOME/.cargo/registry/src" -maxdepth 2 -type d \
    -name "$crate_name-$crate_version" -print -quit 2>/dev/null || true)"
fi
[ -n "$source_dir" ] || fail "$crate_name-$crate_version is not in the offline Cargo cache"
[ -f "$source_dir/Cargo.toml" ] || fail "no Cargo.toml in $source_dir"
grep -qx "name = \"$crate_name\"" "$source_dir/Cargo.toml" \
  || fail "unexpected crate name in $source_dir/Cargo.toml"
grep -qx "version = \"$crate_version\"" "$source_dir/Cargo.toml" \
  || fail "unexpected crate version in $source_dir/Cargo.toml"

src_newlib="$source_dir/src/unix/newlib/mod.rs"
src_horizon="$source_dir/src/unix/newlib/horizon/mod.rs"
[ -f "$src_newlib" ] || fail "missing $src_newlib"
[ -f "$src_horizon" ] || fail "missing $src_horizon"
[ "$(sha256 "$src_newlib")" = "$expect_src_newlib" ] \
  || fail "sha256 mismatch for $src_newlib (unexpected $crate_version source)"
[ "$(sha256 "$src_horizon")" = "$expect_src_horizon" ] \
  || fail "sha256 mismatch for $src_horizon (unexpected $crate_version source)"
patch_sha="$(sha256 "$patch_file")"

# --- already staged? --------------------------------------------------------
if [ "$check_only" -eq 0 ] && [ -f "$dest_dir/$marker_name" ]; then
  if [ "$(sha256 "$dest_dir/src/unix/newlib/mod.rs")" = "$expect_out_newlib" ] \
     && [ "$(sha256 "$dest_dir/src/unix/newlib/horizon/mod.rs")" = "$expect_out_horizon" ] \
     && grep -qx "patch_sha256=$patch_sha" "$dest_dir/$marker_name"; then
    echo "apply.sh: already staged and patched: $dest_dir"
    exit 0
  fi
  echo "apply.sh: staging copy is stale or unexpected; restaging $dest_dir" >&2
  rm -rf "$dest_dir"
fi

if [ "$check_only" -eq 1 ]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/pkg/src/unix/newlib/horizon"
  cp "$src_newlib" "$tmp/pkg/src/unix/newlib/mod.rs"
  cp "$src_horizon" "$tmp/pkg/src/unix/newlib/horizon/mod.rs"
  ( cd "$tmp/pkg" && patch -p1 --forward --silent < "$patch_file" )
  [ "$(sha256 "$tmp/pkg/src/unix/newlib/mod.rs")" = "$expect_out_newlib" ] \
    || fail "patched newlib/mod.rs hash is unexpected"
  [ "$(sha256 "$tmp/pkg/src/unix/newlib/horizon/mod.rs")" = "$expect_out_horizon" ] \
    || fail "patched horizon/mod.rs hash is unexpected"
  echo "apply.sh: patch applies cleanly to $source_dir"
  exit 0
fi

# --- stage and patch --------------------------------------------------------
mkdir -p "$(dirname -- "$dest_dir")"
rm -rf "$dest_dir"
cp -R "$source_dir" "$dest_dir"

# Licenses and upstream provenance stay next to the patched sources.
for f in LICENSE-MIT LICENSE-APACHE README.md Cargo.toml Cargo.lock Cargo.toml.orig; do
  [ -f "$dest_dir/$f" ] || echo "apply.sh: warning: $f missing from the staged copy" >&2
done

( cd "$dest_dir" && patch -p1 --forward --silent < "$patch_file" )

[ "$(sha256 "$dest_dir/src/unix/newlib/mod.rs")" = "$expect_out_newlib" ] \
  || fail "post-patch sha256 mismatch for src/unix/newlib/mod.rs"
[ "$(sha256 "$dest_dir/src/unix/newlib/horizon/mod.rs")" = "$expect_out_horizon" ] \
  || fail "post-patch sha256 mismatch for src/unix/newlib/horizon/mod.rs"

{
  echo "crate=$crate_name"
  echo "version=$crate_version"
  echo "source=$source_dir"
  echo "dest=$dest_dir"
  echo "patch=$patch_file"
  echo "patch_sha256=$patch_sha"
  echo "pristine_newlib_mod_sha256=$expect_src_newlib"
  echo "pristine_horizon_mod_sha256=$expect_src_horizon"
  echo "patched_newlib_mod_sha256=$expect_out_newlib"
  echo "patched_horizon_mod_sha256=$expect_out_horizon"
  echo "applied_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$dest_dir/$marker_name"

echo "apply.sh: staged patched crate at $dest_dir"
echo "apply.sh: registry source untouched: $source_dir"
