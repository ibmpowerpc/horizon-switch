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

There is no model parent/texture resolution, baked quad output, atlas, lighting,
meshing or rendering in this tool yet. The native client does not load this JSON
plan; its future compact runtime resource format remains a separate step.
