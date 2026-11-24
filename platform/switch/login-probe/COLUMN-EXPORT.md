# Column Export Probe 0.10.0

This separate diagnostic preserves the Java 26.3/protocol 777 login,
configuration and column-receipt path. After `COLUMN_SNAPSHOT_OBSERVED`, it can
save the validated normalized column for the next offline meshing stage.
The existing texture and column probes remain separate applications.

## Build and use

```sh
bash scripts/build-switch.sh docker column-export
```

Copy `dist/switch/craftrustnx/craftrustnx-column-export-probe.nro` into
`sdmc:/switch/craftrustnx/`, then launch **CraftRustNX Column Export Probe**.
Use X for the current Java server host, Y for its current port, L for the offline
player name, A to connect, B to cancel and Plus to exit. LAN ports may change
when the host reopens its world; the editable initial port is 25565.

After `COLUMN_SNAPSHOT_OBSERVED`, `Error: NONE`, snapshot status 0 and
`Snapshot sample agrees: yes`, press **R**. A successful save displays
`Saved and read back:` with a path such as:

```text
sdmc:/switch/craftrustnx/captures/capture-0000/column.ctcol
```

Each save creates another unused numbered directory (0000 through 1023), so
existing captures remain intact. The worker is joined before its arrays are
read. The Rust encoder validates the entire column before serialization. The
native file shim writes a temporary file, flushes/closes it, renames it inside
its newly created directory, and reads every byte back for comparison before
reporting success. This checks filesystem readback, not persistence through an
unexpected power loss. On failure, only that attempt's temporary files and
directory are removed. Another press of R retries; A starts a fresh connection.

The SD directory `switch/craftrustnx` must already exist, as in the copy step.
Save status -1 means invalid arguments/counts, -2 a Rust codec rejection,
-3 a memory/filesystem failure or exhausted capture slots, and -4 readback
mismatch/error. The separate codec value reports the Rust status.

Copy the resulting `.ctcol` back through MTP for the next stage. Captures are
private generated data and must remain outside git. They contain coordinates,
raw numeric block-state IDs and biome IDs; no account token, server address,
chat, light arrays, block entities or resource files are stored.

## Format and verification

`CTCOL001`: a 48-byte little-endian header, followed by exactly
`sections*4096` u32 block-state IDs and `sections*64` u32 biome IDs. Sections
are bottom-to-top and cells are Y/Z/X. The header pins protocol 777 and format 1,
includes chunk X/Z/min Y, section and array counts, and two zero reserved words.
Sections are 1..256, min Y is aligned to 16, and the upper bound must fit i32.
File length is exactly `48 + sections*(4096+64)*4`; a 24-section overworld
column is 399,408 bytes, maximum 4,259,888 bytes. Raw IDs have no intrinsic
air/solid meaning and must use the matching version's prepared state tables.

Host checks (cached dependencies, no downloads):

```sh
bash platform/switch/login-probe/tests/run-snapshot-host.sh
bash platform/switch/login-probe/tests/run-column-store-host.sh
```

The first links independent C format/ABI vectors against the real Rust archive.
The second saves real temporary files, decodes them with Rust, verifies repeated
saves preserve the first capture, and checks invalid input and filesystem
failure. Both compile their C code with ASan/UBSan; the Rust archive itself is
not sanitizer-instrumented. Hardware acceptance still requires saving on Switch
and copying the file back for an independent decode.

This is a static column capture. It does not apply subsequent block/light
updates, construct geometry, render a world or send `PlayerLoaded`.
