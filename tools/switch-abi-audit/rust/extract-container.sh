#!/usr/bin/env bash
# Runs inside devkitpro/devkita64. Extract .switch_abi from the Rust staticlib
# with devkitA64 objcopy. Compile-only: no execution of target code.
set -euo pipefail

workspace="${WORKSPACE:-/workspace}"
# Defaults are the unpatched probe. run-patched.sh overrides both to extract
# the patched staticlib into build/patched/.
out_dir="$workspace/tools/switch-abi-audit/rust/build${ABI_OUT_SUBDIR:-}"
lib="$workspace/platform/switch/rust-probe/target/aarch64-nintendo-switch-freestanding/release/${ABI_STATICLIB:-libswitch_abi_probe_rust.a}"

DEVKITPRO="${DEVKITPRO:-/opt/devkitpro}"
AR="$DEVKITPRO/devkitA64/bin/aarch64-none-elf-ar"
OBJCOPY="$DEVKITPRO/devkitA64/bin/aarch64-none-elf-objcopy"
READELF="$DEVKITPRO/devkitA64/bin/aarch64-none-elf-readelf"

[ -f "$lib" ] || { echo "Missing staticlib: $lib" >&2; exit 1; }
mkdir -p "$out_dir"
rm -f "$out_dir/rust_abi_table.bin" "$out_dir/rust_abi_probe.o" \
      "$out_dir/object-header.txt" "$out_dir/members.txt"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"
"$AR" t "$lib" > "$out_dir/members.txt"
"$AR" x "$lib"

found=0
for obj in *.o; do
  if "$READELF" -S "$obj" | grep -q '\.switch_abi'; then
    found=$((found + 1))
    "$OBJCOPY" --dump-section .switch_abi="$out_dir/rust_abi_table.bin" "$obj"
    cp "$obj" "$out_dir/rust_abi_probe.o"
    {
      echo "member=$obj"
      "$READELF" -h "$obj"
      "$READELF" -S "$obj" | grep -E '\.switch_abi|Machine'
    } > "$out_dir/object-header.txt"
  fi
done

[ "$found" -eq 1 ] || { echo "Expected exactly one member with .switch_abi, found $found" >&2; exit 1; }

{
  echo "image=devkitpro/devkita64:20260219"
  echo "objcopy=$("$OBJCOPY" --version | head -n 1)"
  echo "date_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$out_dir/container-env.txt"

echo "Extracted $out_dir/rust_abi_table.bin from $out_dir/rust_abi_probe.o"
