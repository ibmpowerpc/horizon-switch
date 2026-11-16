# Configuration integration tests

Independent loopback server for the native Switch diagnostic, protocol 777.
Synthetic RSA keys are generated temporarily; Python ctypes calls the installed
OpenSSL EVP streaming CFB8 implementation. The server constructs wire packets
independently of Rust and validates every client reply. No game files or user
tokens are used. These checks establish host transport behavior, not vanilla
parity or actual SDK execution on Switch.

Run from the repository root with the existing Rust/OpenSSL installations:

```sh
cargo build --offline --locked --release --manifest-path platform/switch/login-probe/codec/Cargo.toml --target-dir platform/switch/login-probe/codec/build/host-target --jobs 1
mkdir -p platform/switch/login-probe/tests/build
cc -std=gnu11 -O1 -g -Wall -Wextra -Werror \
  -DCRAFTRUSTNX_ENCRYPTED_LOGIN -DCRAFTRUSTNX_CONFIGURATION_PROBE \
  -Iplatform/switch/login-probe/include -Iplatform/switch/login-probe/crypto \
  -I/opt/homebrew/opt/openssl@3/include \
  platform/switch/login-probe/tests/transport_runner.c \
  platform/switch/probe/source-login/transport.c \
  platform/switch/login-probe/tests/crypto_host.c \
  platform/switch/login-probe/crypto/cfb8.c \
  platform/switch/login-probe/codec/build/host-target/release/libcraftrustnx_login_codec.a \
  -L/opt/homebrew/opt/openssl@3/lib -lcrypto -pthread \
  -o platform/switch/login-probe/tests/build/configuration_runner
python3 tools/test_switch_configuration.py platform/switch/login-probe/tests/build/configuration_runner
```

For C memory/undefined-behavior checks add `-fsanitize=address,undefined
-fno-omit-frame-pointer` to the compiler invocation and use a separate output
name. The Rust archive and installed OpenSSL are not sanitizer-instrumented.
On macOS use `UBSAN_OPTIONS=halt_on_error=1 ASAN_OPTIONS=detect_leaks=0`;
this host does not support LeakSanitizer. The fixture rejects runner stderr.
No dependencies or toolchains are fetched. Check the combined project artifact
budget with `python3 platform/switch/check-budget.py` before/after building.

All twelve scenarios pass normally and with C ASan/UBSan:

1. Threshold 256, cookie identifier echoed with absent payload, nonempty known-pack
   offer answered with an empty list, exact brand/ClientInformation and controls.
2. Threshold zero: independently inflate and validate compressed client replies.
3. Fragment encrypted login and keepalive bytes across reads.
4. Coalesce metadata with LoginFinished and play with FinishConfiguration;
   also receive >64 KiB of decoded registry data with another control frame.
5. Receive a separate >64 KiB registry body, deliberately opaque.
6. Truncated keepalive scalar in a complete frame: protocol error.
7. Unknown configuration ID: unsupported, then close.
8. CodeOfConduct: USER_ACTION_REQUIRED, without any acceptance packet.
9. Transfer: unsupported, without any reconnect.
10. Required resource pack: UUID/action-1 decline, then server disconnect before
    FinishConfiguration (configuration acknowledgement must remain false).
11. Encrypted transport without SetCompression, exercising raw client framing.
12. Deterministic cancellation after the worker publishes CONFIGURING; no play ACK.

After RSA negotiation, independent TX/RX cipher contexts remain continuous
through both phase transitions. Compression is set in LOGIN and persists;
configuration ID 3 is FinishConfiguration, not another compression setting.
The fixture validates zlib length, threshold and stream termination. All test
connections use ephemeral loopback ports and close without sending gameplay.
Registry and first-play bodies are synthetic opaque bytes, not schema fixtures.

The runner's first twelve stdout fields retain the login fixture contract.
Appended fields (zero-based): 12 login_accepted; 13 configuration_acknowledged;
14 configuration/play frame count; 15 cumulative decoded body bytes; 16 registry
frame count; 17 first play ID; 18 first play body bytes; 19 any pack declined;
20 any required pack declined; 21 last configuration ID (including Finish).
The remaining lines report accepted player UUID, session UUID and name.

Stages: 14 PLAY_PACKET_SEEN, 15 ACTION_REQUIRED, 8 UNSUPPORTED, 6 DISCONNECTED,
9 ERROR, 10 CANCELLED. Error 8 is protocol, 7 EOF, 12 session/packet limit.
PLAY_PACKET_SEEN means one complete bounded opaque play frame was received.
It does not mean world state was loaded or a player could play.
