# Native deko3d boundary

`include/gpu_probe.h` is the standalone ABI. Rust owns triangle geometry, color,
integer animation and button behavior. The C boundary creates GPU resources and
copies/draws a `gp_frame`; it has no Minecraft protocol or world logic.

The installed deko3d 0.5.0 SDK drives two 1280×720 RGBA8 images. Code, command
and uniform memory use explicit blocks; `gp_graphics_allocated_bytes()` sums
these blocks, excluding driver, queue and CPU allocations. Image layout and
alignment determine actual storage, so no measured total is claimed before a
device run. There is no depth buffer, texture, font or game asset.

Initialization loads original UAM-compiled shaders from RomFS. File reads are
bounded to the code arena, check DKSH magic and shader initialization. The loader
expects the bundled trusted build outputs; it is not a validator for arbitrary
third-party shader binaries. Errors -2..-11 identify device, framebuffer,
swapchain, code, vertex/fragment load, command/uniform memory, command buffer and
queue creation respectively. Every partial path calls shutdown, which checks
resource handles independently and is idempotent.

Each present waits for queue idle before overwriting the shared uniform or
command arena. It resets the bounded command buffer and records a fresh target
binding plus draw. CPU-uncached writes are ordered before submission, and GPU
shader/L2 caches are invalidated before drawing. The 96-byte std140 payload is
bound through a 256-byte aligned range. The caller's memory can be reused after
return. Swapchain acquisition alone would not protect shared buffers with two
images. Serialization is deliberate for this smoke test; it is not the final
renderer strategy or a performance benchmark. GPU waits may block.

Present errors: -1 uninitialized/null argument, -2 missing CPU uniform mapping,
-3 invalid image slot, -4 queue error, -5 empty command list. SDK creation and
submission cannot be fully verified on the host. No FPS, hardware acceleration
or wgpu integration is claimed from compiling these calls.

Compile-only SDK check (in the existing offline Docker image):

```sh
make -C platform/switch/gpu-probe/native
```

The production build is `bash scripts/build-switch.sh docker gpu`; its objects
stay in a separate Makefile directory. Both shaders are original simple GLSL
sources, compiled with the installed UAM. The SDK pattern follows
[switchbrew's public-domain examples](https://github.com/switchbrew/switch-examples).
Original CraftRustNX changes use the repository MIT license. Linked deko3d is
Copyright (C) 2018–2020 fincs under the zlib license; see
[SDK notices](SDK-NOTICES.txt).
