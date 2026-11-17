# Rust Texture Probe 0.9

Separate static preview after the hardware-verified Model Mesh Probe 0.8. It
renders the same nine baked models with ten real sprites in a 128-by-128 atlas.
It has no server connection, live terrain or gameplay.

Prepare the private scene as described in
[ATLAS.md](../../../tools/switch_model_prep/ATLAS.md), then build:

```sh
bash scripts/build-switch.sh docker texture
```

Uses the existing offline SDK image and Rust compiler. Copy these files to the
SD card's `switch/craftrustnx/` folder:

- `dist/switch/craftrustnx/craftrustnx-texture-probe.nro`
- `dist/switch/craftrustnx/texture-probe-scene.bin`
- `dist/switch/craftrustnx/craftrustnx-texture-notices.txt`

The NRO embeds original shaders only. The private scene includes locally prepared
Minecraft pixels and geometry; do not publish it in GitHub or releases. Preserve
0.8 files for comparison.

Launch from hbmenu and press A. A toggles orbit, B changes background, X resets,
right stick rotates, ZL/ZR zoom, Plus returns to the results screen and Plus again
exits. Check all nine textured shapes, the transparent parts of the flower,
glass against the background and other shapes, depth, and seams while rotating.
Neutral white tint intentionally leaves grass without biome colour. This probe
has no game lighting or animation.

Rust performs complete scene validation, the shared 0.8 camera update and
allocation-free back-to-front sorting of translucent quads. A feature on the
existing standalone Rust crate avoids duplicating its camera or linking two
panic runtimes. The native bridge handles libnx/deko3d resource lifetime and
submission only. The original 0.8 build leaves that feature disabled.

Host checks cover binary layout, invalid-input rollback and sorting. SDK compile
and shader checks establish buildability only. No texture, alpha or colour result
is marked hardware-passed until the user runs this NRO.

Run the independent C ABI fixtures against the host Rust static library:

```sh
bash platform/switch/texture-probe/tests/run-host.sh
```

The runner uses cached dependencies offline, then links a C harness with
ASan/UBSan. It checks synthetic malformed scenes, complete draw ranges,
transactional failures, whole-quad sorting and the real private scene when
present. The C translation unit is instrumented; Rust is not. The verified local
run passed 505 assertions with the 516-vertex scene. These checks do not exercise
deko3d or establish a hardware texture pass.
