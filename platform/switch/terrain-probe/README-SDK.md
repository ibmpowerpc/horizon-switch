# Native deko3d terrain GPU bridge

`include/terrain_gpu.h` is the normative ABI. Rust owns geometry, camera, MVP, and
input. The C boundary creates GPU resources and copies/draws untextured colored
meshes; it has no Minecraft protocol or world logic.

## SDK defaults

The installed deko3d 0.5.0 SDK uses these defaults (DkDeviceMaker defaults,
`deko3d.h` lines 116-119, 137):

- **Depth range**: `DkDeviceFlags_DepthZeroToOne` = 0<<8, i.e., 0.0 (near) to
  1.0 (far). The supplied 64-byte column-major MVP is applied as-is; no
  projection conversion is performed.
- **Origin**: `DkDeviceFlags_OriginUpperLeft` = 0<<9, upper-left.
- **Uniform alignment**: 256 bytes (`DK_UNIFORM_BUF_ALIGNMENT=0x100`)

The parent Rust application provides a right-handed perspective projection with
depth in [0,1]. The native probe remains untextured colored meshes.

## Graphics resources

The bridge creates:

- **Double-buffered 1280×720 RGBA8** color surfaces (DkImageFormat_RGBA8_Unorm)
- **Double-buffered 1280×720 Z24X8** depth surfaces (DkImageFormat_Z24X8)
- **Vertex storage**: Bounded GPU-owned `DkMemBlock` for up to `TP_MAX_VERTICES`
  (262140) vertices of `tp_vertex16bytes` (16 bytes each). Storage is allocated
  at init and reused; upload copies caller memory synchronously.
- **Uniform buffer**: 4096-byte `DkMemBlock` (minimum allocation alignment),
  binding a 256-byte range for the 64-byte MVP matrix (std140 mat4).
- **Code memory**: 128KB for vertex/fragment shaders loaded from RomFS.
- **Command memory**: 32KB for rendering command lists.

`tp_graphics_allocated_bytes()` sums all `DkMemBlock` allocations.

## Initialization

`tp_graphics_initialize()`:

1. Creates `DkDevice` with default flags (DepthZeroToOne, OriginUpperLeft)
2. Calculates framebuffer and depth image layouts, acquires required size/align
3. Allocates and initializes double-buffered color/depth images
4. Creates swapchain bound to `nwindowGetDefault()`
5. Allocates code, command, uniform, and vertex memory blocks
6. Loads shaders from `romfs:/shaders/terrain_vsh.dksh` and `terrain_fsh.dksh`
7. Creates command buffer and graphics queue

Errors -2..-13 identify: device, framebuffer mem, depth mem, swapchain, code mem,
vertex/fragment load, cmdbuf mem, uniform mem, vertex mem, command buffer, queue.

Every partial path calls `tp_graphics_shutdown()`, which checks resource handles
independently and is idempotent.

## Mesh upload

`tp_graphics_upload(vertices, count)`:

- **count == 0**: Clears mesh (vertices may be null). Returns 0.
- **count > 0**: Must satisfy:
  - `count <= TP_MAX_VERTICES` (262140)
  - `count % 3 == 0` (triangle list)
  - `vertices` non-null
  - All position components finite (`isfinite`)
- **Validation failure**: Returns non-zero and retains previous mesh
- **Success**: Waits for GPU queue idle before changing shared vertex storage,
  then synchronously copies `count * sizeof(tp_vertex)` to GPU-owned
  bounded storage. No caller pointers retained.

## Present

`tp_graphics_present(mvp, clear)`:

0. Refuse null or non-finite matrix/clear arguments before touching GPU state.
1. **Serialize**: Wait for queue idle before reusing shared memory (command,
   uniform, mesh storage)
2. **Clear command buffer**: `dkCmdBufClear` for fresh rendering
3. **Copy MVP**: 64-byte column-major matrix to 256-byte aligned uniform buffer
4. **Acquire framebuffer**: `dkQueueAcquireImage` returns slot or error
5. **Bind render targets**: Fresh `DkImageView` for acquired slot + depth
6. **Barrier**: Full barrier with Shader+L2Cache invalidate
7. **Build draw**:
   - Viewport/scissor: 1280×720
   - Color clear: `clear[4]` RGBA float
   - Depth clear: 1.0; explicit depth test/write enabled, comparison Less
   - Vertex attributes: location 0 (float3 pos, offset 0), location 1 (Unorm RGBA8, offset 12)
   - Vertex buffer: stride 16, GPU addr from bounded storage
   - Uniform buffer: 256-byte aligned/range, first 64 bytes contain mat4
   - Draw call: `DkPrimitive_Triangles`, vertex count from upload
8. **Submit**: `dkQueueSubmitCommands` then `dkQueuePresentImage`

Errors: -1 uninitialized, -2 uniform mapping, -3 invalid slot, -4 queue error,
-5 empty command list, -6 invalid matrix/clear input.

## Shaders

Shaders are compiled with UAM from GLSL sources in `shaders/`:

- **terrain_vsh.glsl**: std140 mat4 MVP uniform, float3 position + hardware-normalized
  RGBA8 color attributes (vec4 in the shader).
- **terrain_fsh.glsl**: Pass-through fragment output.

Compile with UAM (installed in devkitA64):

```sh
uam -s vert -o terrain_vsh.dksh terrain_vsh.glsl
uam -s frag -o terrain_fsh.dksh terrain_fsh.glsl
```

Compiled DKSH binaries must be placed at `romfs:/shaders/terrain_vsh.dksh` and
`terrain_fsh.dksh`.

## Cleanup

`tp_graphics_shutdown()`:

- Waits for GPU idle before destroying resources
- Destroys queue, command buffer, swapchain, memory blocks, device
- Handles null pointers safely (idempotent)
- Zeroes global state

## Compile verification

Strict C SDK compile (in the existing offline Docker image):

```sh
make -C platform/switch/terrain-probe/native
```

Shaders compile with installed UAM. The SDK pattern follows
[switchbrew's public-domain examples](https://github.com/switchbrew/switch-examples).
Original CraftRustNX changes use the repository MIT license. Linked deko3d is
Copyright (C) 2018–2020 fincs under the zlib license.

## ABI contract

| Element | Size/alignment | Notes |
|---------|----------------|-------|
| `tp_vertex` | 16 bytes | float3 position (12) + uint32 rgba (4), no padding |
| `rgba` offset | 12 bytes | Little-endian byte order R,G,B,A |
| MVP uniform | 64 bytes | 4×4 mat4, column-major, std140 |
| MVP alignment | 256 bytes | DK_UNIFORM_BUF_ALIGNMENT |

## Implementation status

This is a diagnostic safe baseline:

- Double-buffered 1280×720 RGBA8 + Z24X8 depth
- Bounded GPU-owned vertex storage (262140 vertices max)
- 64-byte MVP uniform (std140 mat4)
- Explicit triangle list topology
- Synchronous upload, no retained caller pointers
- Queue serialization before reuse
- CPU cached-memory ordering and shader/L2 invalidations

This is **not** the final throughput backend. Full Horizon/wgpu surface
integration remains separate.

Parent integration corrected allocation alignment (4 KiB), enabled/bound depth
state, serialized uploads, validated frame inputs and used hardware normalized
colors. The packaged NRO builds and passes structural checks; the user subsequently confirmed nine visible shapes and working controls on
Switch Lite. Depth state is enabled in code; exact occlusion/precision and GPU
performance were not separately measured. No host compile alone is a device pass.
