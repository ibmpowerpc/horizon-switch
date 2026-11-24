# From Java column receipt to a rendered section

Baseline: Switch Lite has passed the Java status/login/configuration/play,
structural world-data and normalized column probes. Texture Probe 0.9.1 passed
the user's static visual test, including the corrected flower. These are
separate applications. A connected terrain renderer is not implemented yet.

2026-11-24 progress: Column Export 0.10.0 saved and read back on Switch Lite,
then the returned 24-section file passed independent host format/ID validation.
Its explicitly selected section 8 (Y64..79) has been baked into a private static
scene for Captured Section Probe 0.11.0. Host/ABI/cross-build checks pass;
the user confirmed the captured section renders correctly on Switch Lite.
The stages below remain the integration
plan; on-device meshing and live world rendering are still future work.

## First bounded deliverable: retain the actual server column

The column probe already keeps a validated column in caller-owned arrays after
its worker completes. Add a Rust snapshot codec before joining the data and
render paths. No Minecraft packets or authentication behavior change.

`CTCOL001` contract: 48-byte little-endian header, then raw block-state and biome
IDs. Header fields are magic8; protocol777 at8, format1 at12; chunk X/Z/min Y at
16/20/24; sections/block count/biome count at28/32/36; zero words at40/44.
Counts are exactly sections*4096 and sections*64, sections1..256. Section order
is bottom-to-top, cells Y/Z/X. Exact file length is
`48 + sections*(4096+64)*4`; maximum 4,259,888 bytes. Inputs are validated before
outputs change. Failed encode/decode leaves output buffers untouched and clears
only the FFI written/info result. The snapshot carries no account credentials,
chat, server addresses, light or readiness claim.

After the codec passes host and cross checks, integrate an explicit save action
into the column probe's results screen. Save only after the terminal result is
acquired and the worker joined; never read arrays while the worker can reset or
write them. No automatic game launch or unsolicited server connection. Keep the
previous probes intact. Save through a fresh temporary file and publish without
overwriting an existing capture; report any filesystem error. The large output
buffer belongs to caller heap/storage, not the worker stack or Rust allocator.

## Next: prepare a real captured section for the verified renderer

Load a saved snapshot with the same codec on the host, choose one explicit
16-block-high section, and map raw state IDs through the pinned 26.3 state facts
and existing model-selection plan. Do not infer air or opacity from numeric IDs.
Preserve every selected multipart component. A static preview may explicitly
choose the first weighted alternative, as 0.9 does, without claiming RNG parity.

Reuse Horizon's baker and the verified atlas builder. Emit translated original
quads for the actual 4096 server cells, with bounds and counts checked before
publication. Retain outward winding and back-face culling for flat plants.
Enforce the current vertex/atlas/input budgets; an overflow or unsupported
resource must be reported, never replaced with arbitrary cubes or frame-zero
textures. An incomplete diagnostic must enumerate its omitted categories.
Fluids, animation, block entities, biome tint and world lighting require their
own integration; a static block-model preview does not validate those systems.

## Then move meshing to the Switch

Use the host section preview as an integration reference. Prepare compact
verified state-to-model, quad and sprite tables privately from the player's own
resources. Rust on Switch consumes column cells and those tables, generates a
bounded mesh, and calls the existing small native upload/draw bridge. Keep
protocol handling independent of resource preparation and GPU ownership.

Only after this path works should the client keep a bounded multi-column cache,
apply block/light updates and dimension invalidation, and render while the
network worker maintains the session. PlayerLoaded remains unsent until its
actual chunk/light readiness conditions are satisfied. None of these steps
imply movement, collision or gameplay support by themselves.

## Acceptance and execution limit

For the snapshot step: exact independent wire vectors and transactional failure
checks, Rust fmt/clippy/tests, real C ABI link, offline Switch cross-build, then
hardware save/readback when a live world and MTP are available. Generated captures
and Minecraft resources remain ignored; only source and verification records
enter this fork's main. Do not rebuild the full Horizon workspace.

The user currently reports 50% weekly Codex allowance remaining and requested
stopping at 48% remaining. Tools expose no weekly meter and matching local quota
telemetry was absent. Do not estimate an exact percentage from token counts or
claim automatic monitoring. Keep this iteration bounded and honor an explicit
48% notification immediately, including stopping active delegated work.
