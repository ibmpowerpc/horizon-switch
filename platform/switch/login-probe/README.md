# Java 26.3 offline login diagnostic

This standalone probe targets protocol 777. It sends a login handshake and
LoginStart with an offline UUID, then stops after validating LoginFinished.
It sends no LoginAcknowledged and does not enter configuration or play.

Build/link, 34 Rust tests and 19 host TCP fixtures pass. The separate encrypted
variant has Switch Lite hardware confirmation through login acceptance; the
info-only variant still stops at EncryptionRequest.

The separate [encrypted variant](ENCRYPTED.md) implements RSA/AES transport
negotiation without a session-service join and observes the server's decision.
The `login` variant remains the info-only diagnostic.
The separate [configuration variant](CONFIGURATION.md) continues through
configuration and stops at the first opaque play frame; it has no world renderer.
The [play-session variant](PLAY-SESSION.md) continues through pose acknowledgement,
opaque chunk batches and a keep-alive response, without sending PlayerLoaded.

## Build

From the repository root, using installed Rust 1.100.0/rust-src and the existing
devkitA64 Docker image:

```sh
bash scripts/build-switch.sh docker login
```

This creates `dist/switch/craftrustnx/craftrustnx-login-probe.nro`. All builds
are offline, reuse the no_std core cache, and check the combined 512 MiB artifact
budget and 10 GiB free-space reserve. No toolchains/dependencies are downloaded.

## Hardware controls

- X edits host (IPv4 or ASCII hostname, without port).
- Y edits port; a LAN world's port can change each time it is reopened.
- L edits profile name. Default `CraftRustNX` keeps it distinct from the host player.
- A connects, B cancels, Plus requests cancellation and exits after worker completion.

`LOGIN_ACCEPTED` confirms the server returned a validated login profile; the
diagnostic immediately closes. `DISCONNECTED` displays bounded sanitized server
payload. `AUTH_OR_ENCRYPTION_REQUIRED` means encryption/authentication is not
implemented, including encryption requests with should-authenticate=false.
The screen also reports `Server should_authenticate: true/false` for encryption
requests. False still needs transport encryption; true additionally requests a
real session join. Queries/cookies return `UNSUPPORTED`. No passwords or Microsoft
tokens are used.

The updated hbmenu title is **CraftRustNX Login Info Probe**, version 0.1.1.
The top explanatory label is constant; only the **Stage** line is the result.

## Limits

Only first resolved IPv4 address; ASCII usernames of 1..16 letters/digits/underscore.
Body/wire caps 65536/65541 bytes, at most 16 decoded events. Both packet buffers
are static; worker stack is 128 KiB for codec/decompressor calls. The 10-second
connect/send/receive deadline starts after DNS; blocking DNS can delay cancel/exit.
No retries, SRV, IDNA, online login, configuration, resource packs or gameplay.

## Host verification

The fixture driver `tools/test_switch_login_transport.py` binds only loopback
with an ephemeral port. It compares actual transport output with independent
Python hashlib/UUID and zlib vectors and checks closing without a login acknowledgement.
Build the real host Rust archive, then link the C runner:

```sh
cargo build --offline --locked --release --manifest-path platform/switch/login-probe/codec/Cargo.toml --target-dir platform/switch/login-probe/codec/build/host-target --jobs 1
mkdir -p platform/switch/login-probe/tests/build
clang -std=gnu11 -Wall -Wextra -Werror -pthread -Iplatform/switch/login-probe/include platform/switch/login-probe/tests/transport_runner.c platform/switch/probe/source-login/transport.c platform/switch/login-probe/codec/build/host-target/release/libcraftrustnx_login_codec.a -o platform/switch/login-probe/tests/build/transport-runner
python3 tools/test_switch_login_transport.py platform/switch/login-probe/tests/build/transport-runner
```

See [source audit](../../../docs/switch-login-audit.md) for the current boundary
and remaining compatibility questions.
