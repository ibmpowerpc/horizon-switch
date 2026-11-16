# World-data host checks

The loopback driver constructs independent synthetic Java 26.3 registry and chunk
bodies. It uses installed OpenSSL for RSA/continuous AES-CFB8 and Python zlib for
frame checks, without invoking Rust to generate expected wire packets. No real
world data or Minecraft resources are included.

From the repository root, reuse the existing offline host release archive:

```sh
cargo build --offline --locked --release --manifest-path platform/switch/login-probe/codec/Cargo.toml --target-dir platform/switch/login-probe/codec/build/host-target --jobs 1
cc -std=gnu11 -O1 -g -Wall -Wextra -Werror \
  -DCRAFTRUSTNX_ENCRYPTED_LOGIN -DCRAFTRUSTNX_CONFIGURATION_PROBE \
  -DCRAFTRUSTNX_PLAY_SESSION -DCRAFTRUSTNX_WORLD_DATA \
  -Iplatform/switch/login-probe/include -Iplatform/switch/login-probe/crypto \
  -I/opt/homebrew/opt/openssl@3/include \
  platform/switch/login-probe/tests/transport_runner.c \
  platform/switch/probe/source-login/transport.c \
  platform/switch/login-probe/tests/crypto_host.c \
  platform/switch/login-probe/crypto/cfb8.c \
  platform/switch/login-probe/codec/build/host-target/release/libcraftrustnx_login_codec.a \
  -L/opt/homebrew/opt/openssl@3/lib -lcrypto -pthread \
  -o platform/switch/login-probe/tests/build/world-data-runner
python3 tools/test_switch_world_transport.py platform/switch/login-probe/tests/build/world-data-runner
```

Fourteen scenarios cover single/local/global palettes, sample resolution,
threshold zero and encrypted raw framing, a >64 KiB column with a multi-byte
light mask, invalid dimension height, missing biome data, bad heightmaps/palette
indices/light lengths, no matching sample, a sample invalidated by a new pose,
and a separate LightUpdate. Error cases check terminal status and transactional
results. A literal Rust vector separately pins low-byte-first light mask order.
The 65 previous login/encryption/configuration/play transport cases also pass.

For C ASan/UBSan add `-fsanitize=address,undefined -fno-omit-frame-pointer` and
choose a separate runner output. On this macOS use
`UBSAN_OPTIONS=halt_on_error=1 ASAN_OPTIONS=detect_leaks=0` when running the driver;
LeakSanitizer is unsupported. All 14 scenarios passed. Rust and installed OpenSSL
are not instrumented by that C build.

The runner preserves the first 37 output fields. Appended indices: 37 world
status, 38 dimension count, 39 biome count, 40 checked chunks, 41 light updates,
42 sections, 43 heightmaps, 44 block entities, 45 sky layers, 46 block layers,
47 sample valid, 48/49/50 sample coordinates, 51 state ID, 52 biome ID,
53 selected min Y, 54 selected height, 55 selected skylight. Stage 18 is
WORLD_DATA_OBSERVED. Profile/dimension/pose lines retain their previous format.

A live-host pass against Java 26.3 complements the synthetic fixtures; it does
not replace a Switch run or establish full vanilla parity. The probe validates
wire structure and returns raw IDs without storing terrain or sending
PlayerLoaded. No performance claim follows from host timing.
