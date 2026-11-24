#!/usr/bin/env bash
set -euo pipefail
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd -- "$here/../../../.." && pwd)"
cd "$root"
unset CARGO_ENCODED_RUSTFLAGS RUSTFLAGS
cargo build --offline --locked --release --features column-mesh --jobs 1 \
  --manifest-path platform/switch/terrain-probe/rust/Cargo.toml \
  --target-dir platform/switch/texture-probe/build/host-target
cc -std=c11 -Wall -Wextra -Werror -g -fsanitize=address,undefined \
  -Iplatform/switch/texture-probe/include -Iplatform/switch/terrain-probe/include \
  "$here/column_mesh_host.c" platform/switch/texture-probe/build/host-target/release/libcraftrustnx_terrain_probe.a \
  -lm -o "$here/../build/column-mesh-host"
"$here/../build/column-mesh-host" \
  .local/switch-captures/2026-11-24/capture-0000/column.ctcol \
  .local/switch-models/26.3/section-model-bank.bin \
  .local/switch-models/26.3/captured-section-scene.bin
