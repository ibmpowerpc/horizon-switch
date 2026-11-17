# Allocation-free mesh and camera

Standalone no_std static library. Normative C contracts are
`../include/terrain_app.h` and `../include/terrain_gpu.h` (relative to this directory).
No SDK, std, allocation, filesystem or network is used. Cached libm supplies
camera trigonometry/square roots; panic aborts through the C runtime.

`craftrustnx_mesh_decode` checks protocol/format/size/count, finite bounds and
every vertex before writing any output. The caller owns input/output memory;
invalid input zeroes metadata and leaves vertices untouched. It is a static
diagnostic mesh loader, not the general Minecraft model/world format.

`craftrustnx_terrain_update` produces a column-major right-handed MVP with depth
zero-to-one, matching deko3d defaults. Camera time has a 250 ms delta cap and
monotonic timestamp handling; sticks, pitch and zoom are bounded. Controls have
no simulation or server effects. Caller validity/alignment/non-overlap remain
the unsafe FFI contract.

From the repository root:

```sh
cargo test --offline --locked --jobs 1 \
  --manifest-path platform/switch/terrain-probe/rust/Cargo.toml \
  --target-dir platform/switch/terrain-probe/rust/build/host-target
bash platform/switch/terrain-probe/build.sh
```

The cross-build reuses the existing freestanding core target. Host artifacts
stay separate from it. See the parent README for the packaged probe and C-link
verification; neither host nor cross compilation establishes a device pass.
