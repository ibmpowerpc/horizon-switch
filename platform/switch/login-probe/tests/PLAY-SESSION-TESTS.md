# Play-session host checks

The independent driver binds ephemeral loopback ports and constructs Java 26.3
wire packets without using the Rust encoder. Temporary synthetic RSA keys are
removed after each run; installed OpenSSL provides separate continuous CFB8
send/receive contexts. Client frame lengths, zlib envelopes and full reply bodies
are checked independently. Chunk bodies are synthetic opaque bytes, not a
vanilla chunk-schema oracle. These tests do not execute Switch SDK crypto.

Build the existing host release archive, then the C runner, from the repo root:

```sh
cargo build --offline --locked --release --manifest-path platform/switch/login-probe/codec/Cargo.toml --target-dir platform/switch/login-probe/codec/build/host-target --jobs 1
cc -std=gnu11 -O1 -g -Wall -Wextra -Werror \
  -DCRAFTRUSTNX_ENCRYPTED_LOGIN -DCRAFTRUSTNX_CONFIGURATION_PROBE -DCRAFTRUSTNX_PLAY_SESSION \
  -Iplatform/switch/login-probe/include -Iplatform/switch/login-probe/crypto \
  -I/opt/homebrew/opt/openssl@3/include \
  platform/switch/login-probe/tests/transport_runner.c \
  platform/switch/probe/source-login/transport.c \
  platform/switch/login-probe/tests/crypto_host.c \
  platform/switch/login-probe/crypto/cfb8.c \
  platform/switch/login-probe/codec/build/host-target/release/libcraftrustnx_login_codec.a \
  -L/opt/homebrew/opt/openssl@3/lib -lcrypto -pthread \
  -o platform/switch/login-probe/tests/build/play-session-runner
python3 tools/test_switch_play_session.py platform/switch/login-probe/tests/build/play-session-runner
```

For the C sanitizer run, add `-fsanitize=address,undefined
-fno-omit-frame-pointer` to the compiler command and use a different output path.
On this macOS run with `UBSAN_OPTIONS=halt_on_error=1 ASAN_OPTIONS=detect_leaks=0`;
LeakSanitizer is unsupported. Rust and installed OpenSSL are not instrumented.
The fixture rejects runner stderr and nonzero exits.

Twenty-one scenarios cover ordinary encrypted flow, threshold zero, raw framing,
fragmentation, relative pose/pitch clamp, success after the closing bundle,
relative-first rejection, position before Login, duplicate Login, NaN, mismatched
or nested batches, no false success without a batch ACK, reconfiguration,
respawn, transfer, malformed/unknown packets, server disconnect, required pack
refusal and deterministic cancellation in play. Full-join optional fields,
argument bounds, ABI layouts and transactional errors are also covered by the
standalone Rust tests.

The runner preserves the first 22 login/configuration output fields. Appended
zero-based indices are 22 play frames, 23 decoded play bytes, 24 pose ACKs,
25 keepalive replies, 26 pong replies, 27 opaque chunks, 28 batch ACKs, 29 last
play ID, 30 Login parsed, 31 pose parsed, 32 entity ID, 33 game mode, 34 teleport
ID, 35/36 last chunk x/z. Following the three accepted-profile lines are the
dimension and hexadecimal floating-point pose. Stage 16 is observing play;
terminal 17 is PLAY_SESSION_OBSERVED. Previous stages retain their values.

The success predicate requires a parsed Login, acknowledged pose, at least one
chunk, a completed acknowledged batch, a keepalive response and no open batch
or bundle. No PlayerLoaded, movement or client tick is expected. Host fixtures
and any live-server host check do not replace a physical `.nro` test.
