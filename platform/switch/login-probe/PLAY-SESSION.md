# Bounded play-session probe

Build with existing Rust sources/dependencies and the existing offline SDK image:

```sh
bash scripts/build-switch.sh docker play-session
```

Output: `dist/switch/craftrustnx/craftrustnx-play-session-probe.nro`.
Homebrew title **CraftRustNX Play Session Probe**, version **0.4.0**.
Prior encrypted-login and configuration NROs are separate and preserved.

The initial host is `192.168.0.12`, editable with X. Y edits the current Java
LAN port, L edits the offline player name, A starts, B cancels, Plus exits.
This is the same protocol 777 / Java 26.3 and no-session-join authentication
attempt as before. The server decides whether to accept that offline profile.

After configuration, the probe parses Login and PlayerPosition, acknowledges
teleports with full coordinates/angles, replies to play keepalive/ping/cookie
requests, counts opaque chunk bodies and acknowledges complete chunk batches.
It also supports subsequent relative pose corrections after an absolute baseline.
It does not install registries or validate chunk/light schemas.

Expected success: **PLAY_SESSION_OBSERVED**, **Error: NONE**, configuration ACK
**yes**, Pose ACKs >=1, keepalive >=1, opaque chunks >=1 and batch ACKs >=1.
The display includes entity ID, dimension, mode and last acknowledged position.
Success waits for any open chunk batch and bundle to close, then disconnects.
The server may take around 15 seconds to send a keepalive; the whole attempt has
a 30-second deadline after DNS. A slow server can time out before all milestones.

No PlayerLoaded, movement/tick packets or gameplay are sent. Resource packs are
declined; respawn, transfer and reconfiguration stop as unsupported. Server
refusal/disconnect is displayed separately from a transport failure. Terrain
is not ready and nothing is rendered, even after successful observation.

Runtime: approximately 10 MiB heap buffers, 128 KiB worker stack, bounded wire/
decoded bodies of 2 MiB/8 MiB, 4096 post-login packets and 64 MiB decoded total.
Blocking DNS and a single bounded crypto/inflate operation cannot be interrupted;
100 ms socket wait slices remain cancellable. All owned resources are freed
before the UI reads a terminal result.

For hardware testing, record Stage/Error, Pose ACKs, keepalive, opaque chunk and
batch counts, and position/dimension. The host fixture
`tools/test_switch_play_session.py` uses synthetic wire bodies and independent
OpenSSL CFB8 contexts; it does not replace hardware or vanilla parity testing.
Source contract: [play-session audit](../../../docs/switch-play-session-audit.md).
Reproduce host checks: [PLAY-SESSION-TESTS.md](tests/PLAY-SESSION-TESTS.md).

The host runner also completed this exchange with the user's live Java 26.3 LAN
world in 16.59 seconds: pose ACK, 49 opaque chunks, 41 batch ACKs and one
keepalive reply. This uses the host OpenSSL backend. A subsequent physical Switch Lite run also
passed: PLAY_SESSION_OBSERVED, Error NONE, one pose ACK, 49 opaque chunks,
41 batch ACKs and one keepalive reply. Chunk schemas and terrain remain unloaded. See [verification](../../../docs/verification.md).
