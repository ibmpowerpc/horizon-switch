#!/usr/bin/env bash
set -euo pipefail
test_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
probe_dir="$(cd "$test_dir/.." && pwd)"
unset CARGO_ENCODED_RUSTFLAGS RUSTFLAGS
cargo build --manifest-path "$probe_dir/codec/Cargo.toml" --offline --locked \
  --release --jobs 1 --target-dir "$probe_dir/codec/build/host-target"
mkdir -p "$test_dir/build"
cc -std=c11 -Wall -Wextra -Werror -g -fsanitize=address,undefined \
  -DCRAFTRUSTNX_COLUMN_EXPORT -I"$probe_dir/include" \
  "$test_dir/column_store_host.c" "$probe_dir/../probe/source-login/column_store.c" \
  "$probe_dir/codec/build/host-target/release/libcraftrustnx_login_codec.a" \
  -o "$test_dir/build/column_store_host"
"$test_dir/build/column_store_host"
