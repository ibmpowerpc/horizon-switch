# Structured World Data Probe 0.5.0

This standalone diagnostic extends the verified play-session probe for Java 26.3
(protocol 777). Rust owns the allocation-free NBT, dimension registry, chunk,
palette and light decoding. The existing C worker and libnx UI retain bounded
results; this is not yet Horizon's world cache or renderer.

```sh
bash scripts/build-switch.sh docker world-data
```

Output: `dist/switch/craftrustnx/craftrustnx-world-data-probe.nro`.
In hbmenu select **CraftRustNX World Data Probe 0.5.0**.
X edits the host (default `192.168.0.12`), Y the port (default `25565`),
L the offline nickname, A connects, B cancels, Plus exits. A Java LAN world's
port may change each time it is opened; use the current port shown by Java.
The server decides whether the offline profile is accepted. RSA/AES negotiation
continues without a session join, as in the previous diagnostic.

Expected success is **WORLD_DATA_OBSERVED**, **Error: NONE**, checked chunks and
sections greater than zero, and a block-below-pose sample with raw state/biome IDs.
The predicate also requires Login, a full pose ACK, a completed chunk batch and a
keepalive reply. It waits for the current batch/bundle to close, then disconnects.
A server can take about 15 seconds to send keepalive; the attempt is bounded by
30 seconds after blocking DNS. Previous probe variants keep their behavior.

The probe retains up to 32 dimension shapes and the biome registry count. It
walks network NBT with explicit byte/depth/tag budgets; validates heightmap sizes
and values, the two section counts, prefix-less paletted containers, block-entity
NBT and coordinates, light masks and 2048-byte layers. Java 26.3 light BitSets
are length-prefixed byte arrays in low-byte-first order, not long arrays.
Only a matching chunk can supply the sample below the acknowledged player pose;
a subsequent pose correction invalidates the previous sample.

These checks establish packet structure, not a loaded world. Global block IDs
use the wire bit width without a built-in block-registry width cross-check.
Biome IDs are checked against the received count, but biome fields are not
installed. NBT strings remain borrowed modified-UTF-8 bytes. There is no block
name mapping, terrain cache, mesh, light propagation, movement or renderer.
PlayerLoaded is not sent. Required resource packs, transfer, respawn and
reconfiguration retain the previous diagnostic's unsupported/declined behavior.

Host verification includes 103 Rust tests, 14 independent world-data TCP
scenarios and a successful connection to the user's real Java 26.3 LAN world:
49 chunks / 1176 sections checked, a valid sample and one keepalive response.
This uses OpenSSL on the host; execution of 0.5.0 on Switch still needs a separate
hardware result. See [verification](../../../docs/verification.md) and
[reproducible host checks](tests/WORLD-DATA-TESTS.md).
