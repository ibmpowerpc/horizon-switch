# Encrypted offline login diagnostic (Java 26.3 / protocol 777)

From the repository root: `bash scripts/build-switch.sh docker encrypted-login`.
Output: `dist/switch/craftrustnx/craftrustnx-encrypted-login-probe.nro`.
The hbmenu entry is **CraftRustNX Encrypted Login Probe**, version **0.2.0**.
Use X for host, Y for the current port, L for profile name, A to connect,
B to cancel, Plus to exit. Default name is `CraftRustNX`.
The earlier status and Login Info NROs remain separate.

This diagnostic answers EncryptionRequest with a random shared secret and the
server's challenge encrypted using RSA PKCS#1 v1.5, then receives AES-128-CFB8
frames. It sends no session-service join or account tokens. The observed
`should_authenticate` flag is displayed separately; the server decides whether
to accept or reject the offline profile. The user's LAN world was reported to
accept offline TLauncher profiles despite setting that flag to true.

`LOGIN_ACCEPTED` means a structurally validated LoginFinished was received; the
probe closes before configuration/play and sends no LoginAcknowledged.
`DISCONNECTED` shows sanitized server text. `ERROR` includes network, codec or
crypto codes. `KEY_EXCHANGE` is intermediate, not success. Hardware encryption
and login acceptance are **not yet verified**.

## Bounds and SDK

The installed devkitA64 image supplies mbedTLS 2.28.10 RSA and libnx AES/CSRNG.
No downloads, new images or dependencies are required. CSRNG initialization and
generation errors stop the exchange; there is no fixed seed or fallback key.
DER public key length is capped at 1024 bytes; RSA-1024/2048 only. Challenge
length must fit PKCS#1 padding (1..k-11, at most 245). Session material is wiped
on cleanup. Dependency notices: [LICENSES](../LICENSES/README.md).

EncryptionResponse is the last plaintext packet. Residual receive bytes are
decrypted once at the switch; later recv calls decrypt only new bytes. CFB8
registers persist across chunks, independently in each direction. Existing
64 KiB body, 16-event and first-IPv4-address limits remain. The 10-second network
deadline starts after DNS; blocking DNS or synchronous SDK crypto calls can
delay cancellation. Only socket waits use 100 ms slices. Sending compressed
EncryptionResponse packets is unsupported; an existing compression envelope
is supported only when the response body is below its threshold.

## Verification

Rust: 34 tests, clippy and fmt pass. Original TCP regression: 19 fixtures.
Six CFB8 scenarios use independent NIST vectors. Thirteen encrypted loopback
scenarios verify acceptance with flag false/true, RSA-2048, fragmentation,
server refusal, bad checksum, repeated encryption request, partial EOF,
cancellation, invalid/trailing DER, RSA-512 rejection and oversized challenge.
The same 13 pass with ASan/UBSan.

The host backend uses installed OpenSSL 3 and shares the CFB8 byte algorithm with
Switch. The server independently uses OpenSSL's CFB8 and RSA decryption.
Synthetic private keys live in an ignored temporary directory and are deleted.
Host tests do not execute mbedTLS/libnx or prove SDK RNG correctness.

Host commands (repository root):

```sh
cargo build --offline --locked --release --manifest-path platform/switch/login-probe/codec/Cargo.toml --target-dir platform/switch/login-probe/codec/build/host-target --jobs 1
clang -std=gnu11 -Wall -Wextra -Werror -DCRAFTRUSTNX_ENCRYPTED_LOGIN -Iplatform/switch/login-probe/include -I/opt/homebrew/opt/openssl@3/include -pthread platform/switch/login-probe/tests/transport_runner.c platform/switch/probe/source-login/transport.c platform/switch/login-probe/tests/crypto_host.c platform/switch/login-probe/crypto/cfb8.c platform/switch/login-probe/codec/build/host-target/release/libcraftrustnx_login_codec.a -L/opt/homebrew/opt/openssl@3/lib -Wl,-rpath,/opt/homebrew/opt/openssl@3/lib -lcrypto -o platform/switch/login-probe/tests/build/encrypted-transport-runner
python3 tools/test_switch_encrypted_login.py platform/switch/login-probe/tests/build/encrypted-transport-runner
```

Add `-fsanitize=address,undefined` to clang for sanitizer verification.
