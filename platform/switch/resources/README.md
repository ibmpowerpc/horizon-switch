# Switch render-asset staging

Developer-only, offline helper that prepares a bounded input set for Switch
meshing experiments from the player's own pinned Java 26.3 client jar. It reads
the jar where Minecraft/Prism installed it; the installation is not modified and no Mojang file is committed. Output goes to the ignored
`.local/switch-assets/26.3` folder (or `--output`).

```sh
python3 tools/stage_switch_assets.py \
  --client-jar "/path/to/minecraft-26.3-client.jar" --dry-run
python3 tools/stage_switch_assets.py \
  --client-jar "/path/to/minecraft-26.3-client.jar"
```

## What it selects

- `assets/minecraft/models/**.json` (block models, including every parent model)
  and `assets/minecraft/blockstates/**.json`;
- `assets/minecraft/textures/block/**` and `textures/colormap/**` PNGs with
  their `.png.mcmeta` sidecars;
- every concrete texture bound by a block model's `textures` values, written as
  a plain string or as an object with a `sprite` key, resolved against the
  `minecraft` namespace by default. Referenced PNGs outside `textures/block`
  and `textures/colormap` are pulled in with their optional `.png.mcmeta`, while model JSON is retained for a later parent/slot resolver. A sprite written with a trailing
  `.png` is normalized to the same texture file.

`#variable` references are not treated as texture filenames. The importer
collects concrete material bindings across block model JSON; it does not bake
models or implement the renderer's inheritance/slot resolution. Texture references the jar does not contain are recorded in the
manifest and printed by the report (`missing:` lines); they are never
synthesized, and the script does not claim full closure of every model.

## Pin and safety rails

- The jar must match the built-in pin: SHA1
  `e877b6a07acd633fb3bb475002175cec036e7b87`, root `version.json` id `26.3`,
  `protocol_version` 777. `--research-lock FILE` only *confirms* the same
  constants; it cannot widen them.
- Every ZIP name is checked (no absolute paths, backslashes, `..`, empty
  components), and encrypted entries, symlinks and duplicate or case-colliding
  names (the target SD card is case-insensitive) are rejected.
- Limits checked before anything is written: at most 12000 files, 2 MiB per
  file, 8 MiB of output in total including the manifest. Each entry is read once
  with a bound, its CRC verified by the ZIP reader, and its size compared with
  the central directory.
- Staging happens in a sibling temporary directory that is renamed into place,
  so a failure leaves no output. An existing output is never overwritten: it is
  accepted only if its manifest and every file hash still match, otherwise the
  run refuses until the folder is removed. Only the temporary directory this run
  created is cleaned up.
- `--dry-run` reports the source SHA1, the selected count and declared bytes, the
  texture bindings found and every missing reference, without extracting.

## Manifest

`manifest.json` (schema 1) records the Minecraft version, protocol, source SHA1,
total bytes, the block-model binding summary (`models_scanned`,
`texture_bindings`, `extra_files`, `missing`) and one sorted entry per file with
its relative path, size and SHA256. `python3 -m unittest tests.test_stage_switch_assets`
covers selection, reference closure, pin refusal, unsafe ZIP entries, limits,
atomic cleanup and existing-output protection with synthetic jars only.

This is not a Horizon resource pack and not a semantic block-state mapping: no
models are baked, no atlas is assembled, and no renderer runs. Whether the
staged subset is enough to mesh every Switch-visible block still has to be
answered by the meshing experiment itself.

## Verified local preparation

The parent validated the installed OffPrism 26.3 JAR against Horizon's pin and
staged 6906 files: 2,986,504 data bytes plus 1,287,232 manifest bytes. All files
were independently compared with the JAR and their recorded SHA256 hashes.
The block-model scan covered 2883 models and 5326 concrete texture bindings;
18 additional PNGs were included. One reference, `minecraft:missingno` from
`block/air.json`, has no PNG in the JAR and remains explicitly recorded.
No placeholder was synthesized.

Eight synthetic tests pass, including actual Horizon nested research-lock
shape handling. The copied data is ignored and is not a complete prepared
Horizon bundle, baked atlas, semantic registry or renderer.

Although logical content plus manifest is about 4.1 MiB, APFS allocates about 29 MiB
for these thousands of files. The project budget checker now counts both
`.local/switch-assets` and `.local/switch-state-facts`. Before staging, only
regenerable vendor copies and host build outputs of the completed std probe
were removed; all delivered NRO/ELF files and verification records remain.
`std-probe/preflight.sh` can restage its vendor source from installed rust-src
and the local Cargo cache without downloads when that experiment is needed.
