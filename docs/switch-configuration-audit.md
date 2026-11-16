# Switch configuration diagnostic: source audit

Reference: Horizon `df8c20422`, inspected with `git show` without expanding the
checkout. This documents the source contract; it does not establish parity with
a running vanilla server. The preceding encrypted-login diagnostic passed on a
physical Switch Lite. Configuration has not yet been tested on that hardware.

## Sequence and packet IDs

`crates/horizon-client/src/net.rs::run` sends login `LoginAcknowledged` (3),
then configuration brand and ClientInformation after LoginFinished. The source
IDs are in `crates/horizon-protocol/src/ids.rs`; packet field codecs are in
`crates/horizon-protocol/src/packets/configuration.rs`.

Direction matters: clientbound CustomPayload is 1; serverbound CustomPayload is
**2**. The probe identifies itself truthfully as `CraftRustNX` on
`minecraft:brand`. ClientInformation (serverbound 0) has nine fields, in order:
language string, view-distance byte, chat-visibility VarInt, chat-colors boolean,
model-customisation unsigned byte, main-hand VarInt, text-filtering boolean,
listing boolean and particle-status VarInt. Probe defaults are `en_us`, distance
2, enabled colors, model bits 127, right hand, no filtering, listing allowed,
and all particles. These describe protocol preferences, not implemented gameplay.

`net.rs::configure` handles the following exchanges:

| Clientbound | Probe reply, serverbound |
|---|---|
| 0 CookieRequest | 1 CookieResponse, same identifier, absent payload |
| 3 FinishConfiguration | 3 FinishConfiguration, then switch to play framing |
| 4 KeepAlive, i64 | 4 KeepAlive, exact same eight bytes |
| 5 Ping, i32 | 5 Pong, exact same four bytes |
| 9 ResourcePackPush | 6 ResourcePack, same UUID, action 1 (declined) |
| 15 SelectKnownPacks | 7 SelectKnownPacks, empty list |
| 20 CodeOfConduct | Stop as USER_ACTION_REQUIRED, send no acceptance |
| 12 Transfer | Stop as UNSUPPORTED, open no new connection |
| 2 Disconnect | Stop and display bounded sanitized opaque component bytes |

The game client can acknowledge installed known packs because it has resources.
This diagnostic has none and always replies with an empty list. It neither
claims `minecraft:core` nor pretends to download a resource pack. A required
pack can therefore cause a normal server refusal. In the source, resource-pack
actions are 0 loaded, 1 declined, 2 failed download, 3 accepted, 4 downloaded,
5 invalid URL, 6 failed reload, 7 discarded. None but declined is used here.

Serverbound AcceptCodeOfConduct is **9**. The diagnostic does not send it.
The conduct text is a protocol string, not an NBT component. Interactive consent
and resource-pack installation are outside this experiment.

## Opaque data and limits

Known data-only configuration IDs 1, 6, 7, 8, 10, 11, 13, 14, 16, 17, 18 and 19
are consumed as bounded opaque bodies. RegistryData (7), tags, NBT components,
features, cookies supplied by StoreCookie and dialogs are not loaded, persisted,
interpreted or schema-validated. Unknown configuration IDs stop as UNSUPPORTED.
Scalar control packets that produce replies are validated before sending them.

`crates/horizon-protocol/src/frame.rs` establishes a maximum framed body of
2,097,151 bytes and uncompressed body of 8,388,608 bytes. The diagnostic allocates
approximately 10 MiB of heap for the wire and decoded bodies after successful
login. Allocation failure is a reported MEMORY error. Large buffers never live
on the 128 KiB worker stack. This is runtime memory, not an enlarged NRO asset.
Additional diagnostic limits are 4096 packets after login, 64 MiB cumulative
decoded bytes, 256 known-pack offers, 1024-byte reply bodies and 30 seconds for
the whole operation after DNS. Blocking DNS remains a documented limitation.

Compression remains enabled across phase changes. Below threshold, write a
zero-length compression envelope; at or above it, write a valid zlib stream.
A stored DEFLATE block is sufficient for the probe's small client replies,
including thresholds zero and one. Without compression, use exactly one packet
length prefix. Decode requires exact zlib consumption, output size and checksum.

## Cipher and phase transitions

`crates/horizon-protocol/src/connection.rs` keeps independent send/receive CFB8
states. The RSA response is the final plaintext client packet. After it, encrypt
the entire framed packet once, including the outer length, and advance the send
context once. Partial socket writes then send remaining ciphertext unchanged;
partial writes themselves do not break the cipher. Neither context resets at
LoginAcknowledged or FinishConfiguration. Coalesced residual bytes are consumed
once and already-decrypted tails are preserved when the receive buffer grows.

After acknowledging FinishConfiguration the probe receives one complete bounded
play frame, checks its nonnegative packet ID and reports PLAY_PACKET_SEEN. The
first packet may be Login (ID 50 in this source) or another play packet. Its body
is deliberately opaque. This result proves the framed phase transition and
bidirectional transport, not loaded registries, spawned player, world rendering,
or a playable client. Configuration and play results still require hardware
verification and comparison with the user's Java server.
