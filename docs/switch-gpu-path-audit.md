# Switch GPU Path Audit

**Method**: Offline inventory of devkitpro/devkita64:20260219 container. No device run, no full Horizon build, no external fetch.

## Installed GPU Stack

### deko3d (Native Maxwell API)
- **Libraries**: `/opt/devkitpro/libnx/lib/libdeko3d.a` (3.5M), `libdeko3dd.a` (3.7M debug)
- **Headers**: `/opt/devkitpro/libnx/include/deko3d.h`, `deko3d.hpp`
- **Nouveau DRM**: `/opt/devkitpro/portlibs/switch/lib/libdrm_nouveau.a` (164K), headers present
- **Shader compiler**: `uam` 1.1.0 (GLSL→Maxwell bytecode→`.dksh` container)
- **Examples**: `/opt/devkitpro/examples/switch/graphics/deko3d/{deko_basic,deko_console,deko_examples}`
- **Core API verified** (from deko3d.h): `dkDeviceCreate`, `dkQueueCreate`, `dkMemBlockCreate`, `dkImageLayoutInitialize`, `dkSwapchainCreate`, `dkShaderMakerDefaults`
- **Startup sequence** (from deko_basic example): Device → MemBlock → ImageLayout → Swapchain(nwindowGetDefault) → Shader load → CmdBuf → Queue submit → Present
- **Native window**: libnx `NWindow` struct wraps VI service Binder (`/opt/devkitpro/libnx/include/switch/display/native_window.h`)

**Compilation test**: GLSL→dksh successful. Parent repeated the offline compile:
`uam -s vert -o build/test.vert.dksh test.vert.glsl`. The 102-byte original vertex
source produced 512 bytes with DKSH magic; output is ignored under `build/`.
This verifies shader compilation, not shader execution on the GPU.

### Mesa EGL/GLES (Software or Tegra backend unclear)
- **EGL library**: `/opt/devkitpro/portlibs/switch/lib/libEGL.a` (22M)
- **GLES2 library**: `/opt/devkitpro/portlibs/switch/lib/libGLESv2.a` (158K)
- **Symbols confirmed**: `eglGetDisplay`, `eglInitialize`, `eglCreateWindowSurface`, `_mesa_parse_arb_*`, `_mesa_glsl_link_shader`
- **Tegra references**: `/opt/devkitpro/portlibs/switch/include/libavutil/{nvtegra.h,hwcontext_nvtegra.h}` (FFmpeg hwaccel context)
- **SDL2 support**: `SDL_VIDEO_OPENGL_EGL 1` in `/opt/devkitpro/portlibs/switch/include/SDL2/SDL_config.h`, libSDL2.a (9.6M)

**Unknown**: Whether installed Mesa EGL uses Tegra hardware backend or software rasterizer. 22M library size suggests full Mesa stack; Tegra headers exist separately in FFmpeg context. No device confirmation.

## Horizon Renderer Architecture (git inspection, no checkout)

- **Framework**: wgpu-only (`crates/horizon-client-render/src/gfx.rs`: `use wgpu`, `pub struct Renderer { instance: wgpu::Instance, adapter: wgpu::Adapter, surface: Option<Arc<wgpu::Surface>>...`)
- **Third-party wgpu**: `third_party/{wgpu-hal,naga}` excluded from workspace (`Cargo.toml`)
- **No separate direct-GL renderer observed** in the inspected entry/struct. This
  does not rule out wgpu selecting its existing GL backend; it does not establish
  the complete renderer's platform requirements.

## Path Analysis

### Path 1: Native deko3d SDK
**Components available**:
- Complete device/queue/memory/swapchain/shader API (headers+libraries confirmed)
- Working GLSL→Maxwell compiler (verified)
- NWindow integration with VI service (example demonstrates)
- Documented startup sequence in working examples

**Blockers**:
- The inspected setup provides no demonstrated wgpu-to-deko3d bridge. A direct
  deko3d path still needs a separate renderer integration design.
- Horizon's wgpu::Instance/Adapter/Surface must be replaced or bridged.
- Memory model translation: wgpu staging buffers ↔ dkMemBlock CPU/GPU addressing.
- Shader translation: WGSL → GLSL → dksh (two-stage vs. wgpu's WGSL→backend).

**Estimate**: A clear/triangle probe can follow deko_basic. At Switch Lite's
1280×720 target, three RGBA8 buffers contain 11,059,200 bytes (10.55 MiB) before
GPU layout/alignment overhead. Command/code pools add implementation-dependent
memory; this is not a measured minimum. No Minecraft resources are included.
Prefer Rust application logic with a small SDK shim if needed; choose the final
renderer from device performance and compatibility evidence.

### Path 2: EGL/GLES via wgpu-hal GLES backend
**Components available**:
- EGL/GLES libraries and headers installed
- SDL2 with EGL support
- wgpu-hal has existing GLES backend (used for WebGL, Android)

**Blockers**:
- **Hardware acceleration unconfirmed**. Installed Mesa may be software-only; Tegra backend presence unknown without device GL string query (`glGetString(GL_RENDERER)`).
- wgpu-hal GLES backend exists but Switch platform surface integration unverified.
- NWindow → EGLNativeWindowType binding unclear (may require libnx/EGL glue).

**Unknown**:
- Actual GL/GLES renderer/driver selected by the homebrew EGL path. Desktop
  Linux kernel-module assumptions do not establish the native Horizon OS route.
- Performance viability if software rasterizer.
- wgpu surface creation for Switch NWindow (not documented in wgpu or SDL2-Switch).

## Concrete Unknowns Requiring Device

1. **EGL hardware evidence**: `eglInitialize` success + `eglQueryString(EGL_VENDOR/EGL_VERSION)` + `glGetString(GL_RENDERER)`.
2. **deko3d swapchain present**: Does `dkQueuePresentImage` actually output to screen on hardware?
3. **wgpu platform surface**: Can wgpu::Instance::create_surface accept Switch NWindow (needs device build + run)?
4. **Memory pressure**: Actual framebuffer allocation limits under Horizon OS (homebrew vs. applet mode).

## Verification Done

- deko3d API complete in headers: device/queue/memory/image/swapchain/shader types present.
- Startup order correct: example shows Device→MemBlock→ImageLayout→Swapchain(nwindow)→Shader→CmdBuf→Present.
- UAM compiler works: GLSL→dksh successful (512-byte output, valid DKSH header).
- EGL symbols exist: eglGetDisplay/Initialize/CreateWindowSurface in libEGL.a.
- The inspected Horizon renderer entry uses wgpu. Whether its GL backend can
  serve Switch remains unverified; no separate direct-GL renderer was examined.

## Remaining Work

- **Smallest probe**: Rust-owned clear/triangle logic through a small libnx/deko3d
  bridge, using deko_basic to establish SDK setup and teardown. This is still
  separate from a Horizon renderer integration.
- **Device confirmation**: NRO build + run for EGL driver string and deko3d present test.
- **wgpu integration**: Investigate the actual backend/platform seam. Read source
  with git show as needed; no full checkout or new toolchain is required merely
  to research this seam.

**Artifact size**: shader source plus 512-byte ignored build output, below 1 MiB.
The parent ran `platform/switch/check-budget.py`; combined experiments remain
below 512 MiB. No device GPU execution or acceleration/performance claim is made.

**Sources**:
- `/opt/devkitpro/libnx/include/deko3d.h`
- `/opt/devkitpro/examples/switch/graphics/deko3d/deko_basic/source/main.c`
- `/opt/devkitpro/portlibs/switch/lib/libEGL.a` (nm output)
- `git show HEAD:crates/horizon-client-render/src/gfx.rs`
- `git show HEAD:docs/layers.md`
