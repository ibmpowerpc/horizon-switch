# Rust GPU Probe 0.6.0

This separate native `.nro` tests Rust-generated frames through deko3d on the
original Nintendo Switch family. It does not run Horizon's renderer, load a
Minecraft world or change the verified Java networking probes.

```sh
bash scripts/build-switch.sh docker gpu
```

Use the existing SDK image and Rust compiler/cache; no downloads are attempted.
Output: `dist/switch/craftrustnx/craftrustnx-gpu-probe.nro`, with the original
compiled vertex/fragment shaders embedded in RomFS. Copy it and
`craftrustnx-gpu-notices.txt` to `sdmc:/switch/craftrustnx/`.

In hbmenu choose **CraftRustNX Rust GPU Probe 0.6.0**. Press **A** at the initial
instruction screen. Expect a colored triangle moving horizontally on a dark
background. During graphics, **A** pauses/resumes, **B** cycles palettes, **X**
resets animation/palette and **Plus** ends graphics. A summary shows SDK/Rust
status, submitted frame count and allocated GPU block bytes. Press **Plus** again
to return to hbmenu. If the app exits through the system loop, it cleans up
without opening another console. The text console and deko3d never own the
native window simultaneously.

Please report whether the triangle is visible and moves, whether pause/palette/
reset work, and whether both the summary and exit work. Status 0 and a frame
count only mean submission calls completed; visual confirmation is separate.
No screenshot readback or FPS benchmark is implemented.

Rust is `no_std`, without allocation or SDK dependencies. It owns geometry,
colors, animation time and control decisions. The C portion handles libnx input,
lifecycle and the native GPU calls. For this diagnostic it waits for GPU idle
before reusing uniform/command memory, so it must not be used to choose the
final renderer on performance grounds. Full Horizon/wgpu surface integration
and an EGL comparison remain unresolved.

Verification before device use: eleven Rust tests, fmt/clippy, Cortex-A57
cross-build, strict C SDK compile/link, NRO/ELF relocation checks, and independent
RomFS traversal matching both shader files byte-for-byte. The user confirmed visible animation, A/B/X controls and Plus exit on
Switch Lite. No device performance or memory measurements were supplied. Details: [Rust logic](rust/README.md), [SDK bridge](README-SDK.md)
and [project verification](../../../docs/verification.md).
