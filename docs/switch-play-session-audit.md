# Switch play-session diagnostic: source audit

Reference: Horizon source in tree `d3c97c522`, read with `git show` without
expanding the checkout. Configuration/first-frame already passed on Switch Lite;
this audit does not establish hardware play-session success or vanilla parity.

## Join and pose

`crates/horizon-protocol/src/packets/play.rs` defines clientbound Login (50):
entity Int, hardcore Bool, level identifiers, max players/view/simulation VarInts,
three booleans, SpawnInfo, online_mode Bool and enforces_secure_chat Bool.
SpawnInfo contains dimension-type VarInt, dimension identifier, seed Long,
game-type VarInt, previous-game-type OptVarInt, debug/flat booleans, optional
GlobalPos (identifier plus packed Long), portal cooldown and sea level VarInts.
Online mode belongs to Login, not SpawnInfo. OptVarInt uses zero for absent and
mode+1 otherwise. The diagnostic retains entity ID, dimension type/name and mode;
it does not interpret a dimension registry.

PlayerPosition (73) carries teleport VarInt, position and velocity-delta triples
of Doubles, yaw/pitch Floats and a big-endian Int relative mask.
`game/remote.rs::remote_teleport` resolves X/Y/Z and yaw by adding the stored
value when its relative bit is set. `position_packet_pitch` adds relative pitch
and clamps the result to [-90,90]. Relative bits are X=1,Y=2,Z=4,Y_ROT=8,
X_ROT=16,DELTA_X/Y/Z=32/64/128,ROTATE_DELTA=256. Velocity adjustment exists in
the full client but is outside this diagnostic's simulation scope.

Serverbound AcceptTeleportation (0) carries **teleport ID and full pose**:
x/y/z Doubles and yaw/pitch Floats. It is not the old teleport-ID-only format.
The probe requires finite wire/resolved numbers, a nonnegative teleport ID and
known relative bits. It accepts a first absolute pose and later relative poses;
a relative first pose stops as unsupported instead of inventing a baseline.
It does not implement mounted-player handling or velocity simulation.

## Controls and chunks

Packet IDs come from `crates/horizon-protocol/src/ids.rs`, with directions
kept distinct:

| Clientbound | Serverbound response |
|---|---|
| KeepAlive 45, Long | KeepAlive 28, exact same Long |
| Ping 62, Int | Pong 45, exact same Int |
| CookieRequest 21, identifier | CookieResponse 21, same key and absent payload |
| ResourcePackPush 82 | ResourcePack 50, UUID and action 1 (declined) |
| ChunkBatchFinished 11, count | ChunkBatchReceived 11, positive Float credit |

ChunkBatchStart (12) and Finish bracket LevelChunkWithLight (46). Chunk headers
begin with x/z Ints; remaining chunk/light bytes stay opaque. The probe verifies
batch boundaries and counts before answering. It chooses credit 1.0 chunk/tick,
a conservative flow-control setting, not a measured processing rate or a claim
of loaded terrain. The full Horizon client currently sends 64.0 in
`remote.rs::handle_packet`; the audit does not claim that 1.0 is vanilla's default.
Bundles (delimiter 0) may contain these packets; success waits for the closing
delimiter as well as batch completion.

The full client's receive/tick code handles chunk packets before
`tick_client_load` sends PlayerLoaded (44). This supports the proposed probe's
receipt/ACK milestone before the world is ready. No movement, client tick,
PlayerLoaded, resource installation or chat session is fabricated.

## Readiness is separate

`remote.rs::tick_client_load` uses the player's section/light status and
`horizon-client-render/src/level_loading.rs::LevelLoadTracker`. Game event 13
(LEVEL_CHUNKS_LOAD_START) advances the tracker to waiting for the player chunk.
For an alive ordinary player inside build height, normal completion waits for
that section. The source also has spectator/outside-height/dead-player
exceptions and a 30-second fallback, then a close delay. Therefore loaded terrain
is not an unconditional prerequisite on every path in the full engine.

This probe implements neither those readiness rules nor chunk/light processing.
It deliberately sends no PlayerLoaded. Stopping short of it is a diagnostic
scope decision, not a claim that the protocol makes every other path impossible.

## Terminal conditions and limits

Respawn (84), StartConfiguration (120) and Transfer (132) stop as unsupported;
the latter's port is a **VarInt**, not an Int. Respawn does not necessarily return
to configuration; this probe simply cannot continue a world reset. Disconnect
(32) carries an opaque NBT component, not a JSON string.

`lan/conn.rs` specifies a 15-second keep-alive interval and a 30-second read
inactivity limit. Its comment says an unanswered keep-alive is dropped at the
next interval, so it is incorrect to promise a fresh 30 seconds after every
request. These are source-side schedules, not a timing guarantee for every
Java server. One received-and-answered keep-alive suffices for the milestone.

Success requires parsed Login, an acknowledged pose, at least one opaque chunk,
a correctly acknowledged complete chunk batch and a keep-alive response,
without an open batch/bundle. The total 30-second post-DNS budget includes login
and configuration. Frame/decoded caps remain 2 MiB/8 MiB, with 4096 post-login
frames and 64 MiB cumulative decoded data. A timeout reports missing progress;
it does not prove a protocol failure. The result establishes bounded transport
behavior, not a loaded world, long-session stability, rendering or gameplay.
