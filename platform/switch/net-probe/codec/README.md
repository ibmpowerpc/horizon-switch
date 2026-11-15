# Java status probe codec

Dependency-free `no_std` Rust library with caller-owned buffers. This is a
probe component; production gameplay continues to use Horizon's protocol crate.
It does not open a socket, parse JSON syntax, log in, or prove Switch multiplayer.

`encode_status_request(host, port, protocol, out)` emits two **complete TCP frames**:

1. Length-prefixed handshake: id 0, signed protocol VarInt, host string, big-endian
   u16 port, next-state 1.
2. Empty status-request frame `[1, 0]`.

Host input is 1..=253 printable ASCII bytes; no IDNA or SRV is implemented. An
invalid host or small buffer returns an error without changing the output.

`parse_status_frame(input)` reads an outer frame length, status-response id **0**,
a string length and UTF-8 bytes. JSON is borrowed, capped at 16384 bytes, and is
not parsed for MOTD/version. `consumed` excludes any following TCP frame.
Incomplete outer data returns `NeedMore`; malformed complete inner data fails.
Oversized/negative frame lengths and overflowing VarInts fail before a large body
is buffered. Packet id 1 is pong, not the status response, and is rejected here.

```sh
cargo test --offline --manifest-path platform/switch/net-probe/codec/Cargo.toml --jobs 1
cargo clippy --offline --manifest-path platform/switch/net-probe/codec/Cargo.toml --all-targets -- -D warnings
```

Seven grouped tests cover independent wire vectors (localhost:25565, protocols
777 and -1), every incomplete prefix of a valid response, malformed lengths,
UTF-8, maximum JSON size, trailing frames, unchanged output on errors, and all
65536 two-byte inputs. No network or target execution occurs.

The status ids agree with `crates/horizon-protocol/src/ids.rs`; response data is
modelled by `crates/horizon-protocol/src/packets/status.rs`. Host tests are useful
codec evidence, not proof of compatibility with a live Java server.
