# Switch login boundary: Java 26.3 / protocol 777

Source audit of fork commit `627b26f325161904bfe08037a124c75849c68635`.
The user identified the tested Java world as **26.3**. Switch Lite status exchange
has hardware confirmation; the login path below has not been tested on hardware.

## Source basis and minimum exchange

The reference is Horizon's implementation, not a fresh vanilla oracle:
`crates/horizon-protocol/src/packets/login.rs`, the login section of `ids.rs`,
and `crypto.rs::offline_uuid`. Their round-trip tests establish internal
consistency; they alone do not prove compatibility with an actual Java server.

1. Handshake: protocol VarInt 777, host string, big-endian port, next-state 2.
2. Serverbound LoginStart (ID 0): player name and 16-byte UUID. The UUID belongs
   to LoginStart, not to the handshake. Offline UUID follows Java
   `nameUUIDFromBytes("OfflinePlayer:" + name)`: MD5, UUID version 3/IETF variant.
3. Optional clientbound SetCompression (ID 3): threshold VarInt. Subsequent
   frames use the compression envelope. TCP may deliver this and the next frame
   together; consume the old frame and decode the buffered tail with new settings.
4. Clientbound LoginFinished (ID 2): GameProfile (UUID, name, property list),
   followed by a 16-byte session UUID in the pinned Horizon source. Its broader
   session semantics were not established by this audit.
5. The diagnostic closes after validated LoginFinished. It sends no
   LoginAcknowledged (serverbound ID 3), and implements no configuration or play.
   `LOGIN_ACCEPTED` therefore describes this boundary, not joining a world.

GameProfile properties carry name/value and optional signature. LoginDisconnect
(ID 0) carries a UTF-8 JSON string. EncryptionRequest (ID 1) carries server ID,
public-key bytes, challenge bytes, and a should-authenticate boolean.

## Explicitly unsupported paths

Offline vanilla login normally skips encryption negotiation. Any encryption
request requires an implementation this probe does not have, even when its
should-authenticate flag is false. Return `AUTH_OR_ENCRYPTION_REQUIRED`; do not
attempt an authentication bypass or claim Microsoft login is implemented.

CustomQuery (ID 4) and CookieRequest (ID 5) get dedicated unsupported outcomes.
Do not assume cookies are exclusive to proxies or that ignoring a request lets
login proceed. Online sessions, query/cookie replies, configuration and play are
separate future work. Use the distinct default name `CraftRustNX`; duplicate
nickname behavior has not been verified and must not be assumed harmless.

## Memory and stream limits

The diagnostic intentionally caps each body at 65536 bytes and wire frame at
65541 bytes, including its outer length prefix. These are probe limits, not a
claim about vanilla's maximum packet sizes. Reject larger packets explicitly.
Use bounded property counts/strings and caller-provided decompression output;
check declared length, actual output, checksum and exact compressed consumption.
Cached miniz_oxide/adler2 permit an offline no-alloc implementation.

Both wire and scratch buffers live in a static C job. They must not be automatic
64 KiB arrays on a 64 KiB worker stack. The integrated UI reserves a 128 KiB
worker stack for codec/decompressor state. C compilation warns on stack frames
larger than 32 KiB; this does not measure Rust's complete call-stack usage.

The worker parses already-buffered frames before another recv, handles partial
I/O, caps the event count, and observes cancellation between bounded operations.
A 10-second network deadline starts **after DNS**. Blocking getaddrinfo delays
cancellation/exit. Decompression has bounded input/output but no independent
wall-clock watchdog; no latency estimate is claimed before measurement.

Only atomic stage/cancel may be accessed while working. Publish terminal state
after closing socket/resolver resources; the UI acquires that state before
reading results and joins before reuse or shutdown.

## Verification still required

Host fixtures must use independent UUID/hashlib and zlib vectors and exercise
coalesced SetCompression, fragmented frames, declared-size/checksum failures,
EOF, cancellation, deadlines, and closing without an acknowledgement. A real
Java 26.3 response must then confirm the assumed LoginFinished session layout.
ASCII profile names are a deliberate restriction; full Java string-decoding
compatibility and support for oversized properties remain outside this probe.
