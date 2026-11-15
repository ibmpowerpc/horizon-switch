#!/usr/bin/env bash
# Compile-only check that the patch's type/offset claims match the real
# devkitA64 + libnx headers. Nothing is executed; the initializer objects are
# only dumped from the object file.
#
# Covers the items that `tools/switch-abi-audit/abi_items.def` does not measure
# (sockaddr_in6, pthread field composition, pthread initializer bytes).
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
image="devkitpro/devkita64:20260219"

command -v docker >/dev/null 2>&1 || { echo "Docker is required." >&2; exit 1; }
docker info >/dev/null 2>&1 || { echo "Start Docker, then retry." >&2; exit 1; }

docker run --rm --pull=never --network none --user "$(id -u):$(id -g)" \
  --mount "type=bind,source=$script_dir,target=/src,readonly" \
  --workdir /tmp \
  "$image" /bin/bash -lc '
    set -euo pipefail
    export PATH=/opt/devkitpro/devkitA64/bin:$PATH
    cp /src/header-check.c .
    aarch64-none-elf-gcc -std=gnu11 -D__SWITCH__ -I/opt/devkitpro/libnx/include \
      -Wall -Wextra -Werror -O2 -c header-check.c -o header-check.o
    echo "header-check: all _Static_assert checks passed (compile-only)"
    aarch64-none-elf-objcopy --dump-section .switch_init=init.bin header-check.o
    echo "header-check: initializer bytes (mutex at 0, 4 padding bytes, cond at 16):"
    od -An -tx1 init.bin
  '
