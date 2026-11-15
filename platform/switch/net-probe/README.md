# Java server status probe

This standalone probe combines a dependency-free Rust status codec with native
libnx TCP sockets and a diagnostic console. It does not run the Horizon client.

From the repository root:

```sh
bash scripts/build-switch.sh docker net
```

Uses installed Rust 1.100.0/rust-src and the existing devkitA64 image offline.
Reuses the no_std probe's core cache and checks the combined experiment budget
(512 MiB) and 10 GiB free-space reserve. No toolchains or dependencies are fetched.

Output: `dist/switch/craftrustnx/craftrustnx-net-probe.nro`.
Copy alongside the other probes to `sdmc:/switch/craftrustnx/`.
In hbmenu launch **CraftRustNX Java Status Probe**:

- X: enter an IPv4 address or ASCII hostname, without `:port`.
- Y: edit port (default 25565).
- A: send a Java status request (protocol 777).
- B: cancel; Plus: cancel and exit after the worker finishes.

Success shows `DONE`, received byte count and sanitized raw JSON. Report that
stage or the error codes displayed. Raw text is bounded to 800 characters;
non-ASCII/control bytes appear as `?`. This display does not parse JSON syntax.
Use a Java server with status queries enabled. An offline account is not needed
for status: this is neither offline login nor a gameplay compatibility test.

The 10-second deadline covers connect/send/receive after DNS. DNS can block, and
cancellation/exit must wait for it. IPv4 only, first resolved address only, no
SRV/IDNA, retries or latency ping. This tests native sockets, not `std::net`.
Hardware execution and live Java-server compatibility still require verification.

## Host verification

From the repository root (macOS clang, no downloads):

```sh
cargo test --offline --locked --manifest-path platform/switch/net-probe/ffi/Cargo.toml --target-dir platform/switch/net-probe/ffi/build/host-target --jobs 1
cargo build --offline --locked --release --manifest-path platform/switch/net-probe/ffi/Cargo.toml --target-dir platform/switch/net-probe/ffi/build/host-target --jobs 1
mkdir -p platform/switch/net-probe/tests/build
clang -std=gnu11 -Wall -Wextra -Werror -pthread -Iplatform/switch/net-probe/include platform/switch/net-probe/tests/transport_runner.c platform/switch/probe/source-net/transport.c platform/switch/net-probe/ffi/build/host-target/release/libcraftrustnx_status_ffi.a -o platform/switch/net-probe/tests/build/transport-runner
python3 tools/test_switch_status_transport.py platform/switch/net-probe/tests/build/transport-runner
```

The fixture server binds loopback on an ephemeral port. Independent expected
request bytes check the actual C transport + Rust archive; cases include a valid
response, chunks, EOF, wrong packet ID, oversized length, cancellation before DNS
and during receive, and a real 10-second receive deadline. No game data is used.
