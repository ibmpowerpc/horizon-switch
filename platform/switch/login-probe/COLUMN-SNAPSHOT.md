# Column Snapshot Probe 0.7.0

```sh
bash scripts/build-switch.sh docker column-snapshot
```

Output: `dist/switch/craftrustnx/craftrustnx-column-snapshot-probe.nro`.
Open **CraftRustNX Column Snapshot Probe 0.7.0** in hbmenu. X edits the host
(default 192.168.0.12), Y the current Java LAN port (default 25565), L the offline
nickname, A connects, B cancels and Plus exits. Server acceptance and the same
RSA/AES/no-session-join policy are unchanged.

Expected success is **COLUMN_SNAPSHOT_OBSERVED**, Error NONE, snapshot status0,
nonzero section/block/biome counts and **Snapshot sample agrees: yes**. The probe
also requires parsed Login/pose ACK, a completed chunk batch and a keepalive
response. It retains the column containing the block below the acknowledged
pose. A pose correction invalidates this selection until a matching chunk is
received again. The server may take roughly 15 seconds to send keepalive;
30 seconds after DNS bounds the exchange.

Rust validates the full packet before copying raw normalized state/biome IDs
into caller-owned static buffers. The arrays remain after the worker closes the
connection, until the application exits or a new attempt replaces the logical
snapshot. They have capacity for 256 sections (4.0625 MiB total). A 384-high column
uses 390 KiB: 98,304 states plus 1,536 biome IDs, each u32. The UI reads only after
acquiring terminal state and joining the worker. Invalidated/unused contents are
not exposed as a current snapshot. BSS holds the large buffers without embedding
zero-filled arrays into the NRO. This is a snapshot at packet receipt; later
block/light changes are not applied. No live cache, block names, air/shape
mapping, retained light, terrain rendering or PlayerLoaded is implemented.

Host checks: 109 Rust tests; 14 independent world-data TCP scenarios; 14 snapshot
scenarios with ASan/UBSan on the C code. Every successful snapshot is checked by
independent digests over all normalized state and biome cells, not only one
sample. Bad input, missing registries and stale/no-pose samples cannot falsely
succeed. The 65 previous login/encryption/configuration/play scenarios also pass.
Rust and installed OpenSSL are not instrumented by the C sanitizer build.

To reproduce snapshot fixtures, use the world runner compile command in
[WORLD-DATA-TESTS.md](tests/WORLD-DATA-TESTS.md), add
`-DCRAFTRUSTNX_COLUMN_SNAPSHOT`, choose a separate executable, and run:

```sh
python3 tools/test_switch_world_transport.py path/to/column-snapshot-runner --snapshot
```

A live-host attempt to the previous LAN endpoint timed out before login; a
current reachable endpoint is needed. Hardware 0.7.0 validation is pending.
World Data 0.5.0 and GPU 0.6.0 hardware passes do not replace this new check.
