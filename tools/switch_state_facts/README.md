# Offline semantic-state export

Developer tooling only. `BlockStateFactsExporter.java` reads registry accessors
from the user's pinned Java 26.3 client; no game/decompiled source is copied.
The exporter bootstraps registries, not the game client, server or a world.
The native Switch application remains Rust plus its SDK shim.

```sh
python3 tools/switch_state_facts/exporter.py \
  --client-jar "/path/to/minecraft-26.3-client.jar" \
  --libraries-dir "/path/to/launcher/libraries" \
  --java-home "/path/to/jdk" \
  --output-dir .local/switch-state-facts/26.3 --preflight
```

Remove `--preflight` to export into a new output directory. The client JAR must
match SHA1 `e877b6a07acd633fb3bb475002175cec036e7b87`, root id 26.3/protocol 777.
By default launcher metadata is `libraries/../meta/net.minecraft/26.3.json`;
`--launcher-metadata` overrides the path. The runner selects its exact Maven
versions and checks cached dependency SHA1s. It rejects gaps or conflicts,
never downloads or selects a random/latest version from the cache. Native
classifier JARs, client ObjC bridge and native tracing are excluded from this
headless registry export; this is not a general launcher classpath resolver.

The verified local run used the installed JDK 27.0.1.0.3, 43 classpath JARs and
`javac --release 25`, with exporter bytecode major 69 checked before execution.
A Java 25-compatibility target does not mean the run used HotSpot 25 or establish
its numeric parity. Actual runtime comes from the Java-produced metadata.

Compilation and execution use fresh scratch cwd, user.home and isolated Horizon/
XDG homes. Java option injection variables are removed; the runner never starts
the launcher/client main, touches real player saves or requests account tokens.
Java heap is capped at 512 MiB, compile/run timeouts 60s. A bounded writer stops
state output at 12 MiB; Python also checks the total directory bytes. Missing
inputs, invalid rows or failures do not publish output: the sibling staging
directory is removed, and existing output is refused rather than overwritten.

Output (local/ignored): `state_facts.json`, `metadata.json`, `export.log`.
Schema 2 is an object with metadata and a states array. Each row has contiguous
numeric id, block resource name, a **canonical property-name/value map**, is_air,
fluid_empty, render_shape, solid_render, can_occlude, full_collision and integer
map_color. Properties use the actual Property.Value.valueName accessor, not an
opaque comma-delimited string. Numeric air IDs are not assumed. Validation checks
IDs/count, unique block+property tuples, flag/value types, metadata pin and the
three vanilla air names. Metadata also records a SHA256 of the complete output.
Full-collision and map-color facts use EmptyBlockGetter at BlockPos.ZERO, like
the inspected upstream exporter; they are not a replacement for baked geometry
or all neighbour-dependent rendering.

Parent verification: the corrected export contains 35,723 states, 3 air states,
13,389 nonempty-fluid states; 9,946,032 JSON bytes, SHA256
`131d7939c39658ac227c711740dc0313e438ba260e93984bee592e07297cfbbd`.
Every MODEL state's blockstate file and selected model references exist in the
staged assets. A selector probe found 64 MODEL states selecting no multipart
parts (wall states with no post/arms); these must remain empty model selections,
with fluids handled separately, not be filled with fallback solid cubes. This
is linkage verification, not a complete texture/UV/lighting bake or visual parity.

Three synthetic runner tests cover semantic validation, source-pin refusal and
cleanup/isolation after a failed export. Run:

```sh
python3 -m unittest discover -s tools/switch_state_facts -p 'test_*.py'
```

No generated facts, logs, hashes manifest or game files may be committed.
