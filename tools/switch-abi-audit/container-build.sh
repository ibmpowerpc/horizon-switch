#!/usr/bin/env bash
# Runs inside devkitpro/devkita64. Compile-only: no link, no execution of target code.
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$here"

DEVKITPRO="${DEVKITPRO:-/opt/devkitpro}"
CC="$DEVKITPRO/devkitA64/bin/aarch64-none-elf-gcc"
OBJCOPY="$DEVKITPRO/devkitA64/bin/aarch64-none-elf-objcopy"

for tool in "$CC" "$OBJCOPY"; do
  if [ ! -x "$tool" ]; then
    echo "Missing tool: $tool" >&2
    exit 1
  fi
done

mkdir -p build
rm -f build/abi_probe.o build/abi_table.bin build/env.txt

{
  echo "compiler=$("$CC" --version | head -n 1)"
  echo "gcc_target=$("$CC" -dumpmachine)"
  echo "gcc_version=$("$CC" -dumpversion)"
  echo "cflags=-std=gnu11 -D__SWITCH__ -I$DEVKITPRO/libnx/include -Wall -Wextra -Werror -O2"
  echo "devkitpro=$DEVKITPRO"
  echo "libnx_include=$DEVKITPRO/libnx/include"
  echo "date_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > build/env.txt

"$CC" \
  -std=gnu11 \
  -D__SWITCH__ \
  -I"$DEVKITPRO/libnx/include" \
  -Wall -Wextra -Werror \
  -O2 \
  -c abi_probe.c \
  -o build/abi_probe.o

"$OBJCOPY" --dump-section .switch_abi=build/abi_table.bin build/abi_probe.o

echo "Compiled build/abi_probe.o and dumped .switch_abi to build/abi_table.bin"
