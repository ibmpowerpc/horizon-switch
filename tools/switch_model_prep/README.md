# Offline Rust blockstate selection

Host developer tool for the Switch resource pipeline. It maps the user's
canonical state properties to selected block-model references. This is a
standalone Rust workspace, using cached serde_json and sha2; no game/client/JVM
is launched and no dependencies are downloaded.

From the repository root:

```sh
bash tools/switch_model_prep/run.sh \
  --facts .local/switch-state-facts/26.3/state_facts.json \
  --assets .local/switch-assets/26.3 \
  --output .local/switch-models/26.3/model-selection.json
```

Output must be new: an existing file is refused, not replaced. Preparation reads
only bounded regular files, verifies the asset manifest pin (26.3/protocol777,
client SHA1), every listed file's size/SHA256 and the aggregate byte count. Unsafe
paths, case collisions and symlink components are refused. Facts require schema2,
the same pin, contiguous IDs, canonical string properties and unique state tuples.
The original exporter is responsible for obtaining those facts from the pinned
registry; a manifest's source hash alone does not authenticate arbitrary edited
facts. The plan records both input hashes.

The schema1 JSON plan interns model names and records one row per state, with
ordered parts and their model-index/x/y/uvlock/weight options. `weighted` records
whether the original application was an array: even a singleton array must stay
distinct from a plain model object for later vanilla random consumption. Every
alternative is retained; no RNG choice occurs here. The complete bounded output
is staged in a sibling file and published with an atomic no-replace hard link.
A filesystem without hard-link support returns an error instead of overwriting.
Generated plans/resources belong under ignored `.local`, never in Git.

## Selector API and supported subset

`selector::select(&serde_json::Value, &BTreeMap<String,String>) -> Result<Vec<Part>,Error>`
returns `Part { weighted, options }`; `ModelOption` exposes normalized model id,
x/y quarter-turn rotation in degrees, uvlock and weight.

- Variant keys are comma-separated `property=value` conjunctions. The empty key
  matches universally. Exactly one key must match; overlapping/missing matches
  fail. `|` is not a variant-key operator.
- Multipart conditions support property-value `|` alternatives, per-term `!`
  negation, nested AND/OR arrays and conjunctions of property clauses. Absent
  `when` is unconditional. Empty AND is true; empty OR is false. Empty property
  objects and mixed operator/property shapes are refused.
- All definition entries are validated, including unmatched ones. Zero matching
  multipart entries is valid empty geometry; no cube fallback is substituted.
- Names are structurally checked resource identifiers with explicit/default
  namespace and safe relative paths. Model references must exist in the verified
  asset set and parse as JSON objects.
- Rotations currently accept x/y in 0/90/180/270; uvlock is boolean. Weighted
  arrays preserve order and require positive signed32 weights and total.

This intentionally strict subset is verified against the pinned local vanilla
resources, not a general resource-pack implementation. It does not reproduce
upstream overlap recovery, duplicate-key ordering/coercions, schema binding,
z rotation or all modded fields. Source API references and the next baking step
are in [the baking audit](../../docs/switch-model-baking-audit.md).

## Geometry baking

`geometry::bake_requested(&BTreeMap<String,Vec<u8>>, &[BakeRequest])` reuses the
existing `horizon-assets` decoder, parent/texture resolver and block baker; it
adds no geometry math of its own. `BakeRequest { model, x, y, uvlock }` matches
one blockstate option (`x`/`y` in 0/90/180/270) and derives `Ord`, so callers can
deduplicate alternatives; `BakedModel { request, quads }` returns the upstream
`bake::ModelQuad` unchanged, keeping material, tint index, cull direction, light,
emission, colour/AO overrides, shading and render type. A returned request carries
the canonical `namespace:path` model id.

- Only the requested models and their parent chains are decoded from the
  caller's manifest-verified `assets/...` bytes; other listed files are ignored.
  Model ids use the selector's strict lowercase relative-path subset, including
  parents named inside model JSON.
- A missing parent, a cyclic chain, a chain deeper than 64 parent links, a
  non-UTF-8 or invalid model, an upstream built-in name (`builtin/missing`,
  `builtin/generated`) or a custom loader (`neoforge:composite`,
  `neoforge:obj`, anything unknown) is refused by name. The upstream
  missing-model fallback is never used and no cube is invented.
- Each reachable face must resolve its texture slot; an unresolved alias is an
  error, not `minecraft:missingno`. Baked positions and UVs must be finite.
- `bake_block_model` is used so zero-extent elements keep vanilla's block-specific
  face selection; the generic item/entity model entry point is not substituted.
- Two requests with the same canonical model and rotation are rejected, not
  silently merged. Returned models follow request order, and an explicitly empty
  element list or an empty request batch yields valid empty geometry.
- No weighted alternative is chosen and no RNG is consumed: the caller passes
  every option its selection retained. Texture atlas coordinates, lighting values,
  biome/ambient sampling and world meshing remain the runtime's job.

Bounds per batch: 16384 requests, 4096 reachable models, 64 parent links, 2 MiB
per model JSON, 16 MiB of decoded model sources, and 1,000,000 emitted quads.
Exceeding any of them, or any model/geometry error, fails the whole batch: no
partial list is returned. These are host input limits for offline preparation,
not a measured Switch memory or performance result; the quads live in host
`Vec`s.

The synthetic tests below cover an independent cuboid's six face planes and
corners, a child overriding a parent's texture, a quarter turn and UV locking,
flat-element face selection, empty geometry, missing/cyclic/too-deep parents,
oversized sources, duplicate/invalid requests, the reachable-model cap,
unresolved materials, custom loaders and the absence of a partial result.

`examples/bake_check.rs` reads the local selection plan and asset manifest,
verifies every listed asset's size/SHA256, collects the unique
`model`/`x`/`y`/`uvlock` signatures from all options (including weighted
alternatives) and bakes them. It writes nothing and prints aggregate counts only
(no model names or quads); a refusal prints the error and exits non-zero.

```sh
cargo run --offline --locked --jobs 1 \
  --manifest-path tools/switch_model_prep/Cargo.toml \
  --target-dir tools/switch_model_prep/build/host-target --example bake_check
```

A local run verified all 6906 listed assets (2,986,504 bytes), deduplicated
77,425 options over 35,723 states into 7856 requests, and baked 79,851 quads
from 7856 models: 35 empty model results, 1247 distinct material sprites,
573 tinted, 580 translucent, 38,944 culled, and no unsupported loader or
material. This cross-checks reuse of the Horizon baker on the pinned resources;
it is not a vanilla JVM rendering oracle and does not prove Switch-side memory
use.

## Bounds and verification

Input facts/output: 12 MiB each; manifest plus listed assets: 8 MiB; individual
asset: 2 MiB; 12000 asset entries, 65536 states, 512 variants/multipart entries,
64 alternatives per part, condition depth32, property strings64 bytes.
Across all states: 131072 parts and 262144 options. These are input/output
limits, not a cap on compiler caches or a measured Switch memory budget.

```sh
cargo fmt --manifest-path tools/switch_model_prep/Cargo.toml --check
cargo clippy --offline --locked --jobs 1 --all-targets \
  --manifest-path tools/switch_model_prep/Cargo.toml \
  --target-dir tools/switch_model_prep/build/host-target -- -D warnings
cargo test --offline --locked --jobs 1 \
  --manifest-path tools/switch_model_prep/Cargo.toml \
  --target-dir tools/switch_model_prep/build/host-target
```

Synthetic tests cover selectors, malformed/oversized definitions, negated
alternatives, weighted singleton distinction/overflow, manifest corruption,
state-plan output and no-overwrite publication. A local run checked all 6906
asset hashes and all 35723 states: 35637 MODEL states, 72572 parts, 77425 options,
2575 unique model references and 64 empty wall selections. An independent Python
evaluation agreed with every state's parts and weighted alternatives. This is
cross-checking resource selection, not a vanilla JVM rendering oracle.

The separate [scene exporter](SCENE.md) now creates a small probe-only binary
mesh of nine named states from these quads. There is no atlas, live-world
meshing, terrain lighting or general game resource format yet. The probe loads
its own static diagnostic mesh; it does not load the JSON selection plan.
