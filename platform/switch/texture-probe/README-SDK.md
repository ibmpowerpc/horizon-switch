# Native texture bridge

`include/texture_gpu.h` defines the ABI. Rust owns scene validation, camera and
translucent-quad sorting; this C shim owns SDK resources and submission only.
The parent integrated this from the verified 0.8 lifecycle, replacing draft SDK
calls with the installed deko3d interfaces. See
[the SDK audit](../../../docs/switch-texture-path-audit.md) for sources.

Resources: two 1280-by-720 UNORM swapchain images with sRGB render-target views,
two Z24X8 depth images, a bounded vertex buffer (262,140 vertices of 24 bytes),
128 KiB shader code, 512 KiB command memory, a 4-KiB uniform block and a 4-KiB
descriptor block. Three 256-byte uniform slots hold the same MVP plus distinct
mode words. Draw ranges are copied into a fixed CPU array; they need no GPU
allocation. Image layouts supply required size/alignment. Every memory block is
rounded to at least 4-KiB alignment. Persistent block allocations are counted;
staging and SDK internal allocations are not included in that counter.

Atlas upload validates power-of-two dimensions 1..1024, creates a new sRGB
block-linear image and staging buffer, copies exactly `width*height*4` source
bytes, and issues `dkCmdBufCopyBufferToImage`. Rounded staging size is never used
as the source read length. The queue is waited before freeing staging. Only
successful completion commits the replacement and frees old image backing;
allocation/validation failures retain the old texture. A device/queue error is
terminal for the session even though old resource handles remain retained.

Image/sampler descriptors are initialized with the installed SDK, then copied
into CPU-uncached descriptor storage at 32-byte GPU-aligned offsets. A CPU fence
and descriptor/image/L2 invalidation precede use. The sampler uses nearest,
clamp-to-edge, no mip filter and maximum LOD zero. Binding uses separate image
and sampler descriptor sets plus `dkMakeTextureHandle(0,0)`.

Mesh upload validates finite positions, UV0..1, bounded counts and complete,
ordered, non-overlapping draw ranges covering the vertex list. Ordinary ranges
contain triangles; translucent ranges contain six-vertex quads. Invalid arguments
leave the old mesh unchanged. Source vertices and draws are copied synchronously;
no caller pointers are retained.

Each present waits for the previous submission before reusing shared storage,
clears the command list, fills the three uniform slots, acquires an image and
binds viewport, scissor, shaders, vertex attributes and texture descriptors.
Vertex attributes are position3 at0, UV2 at12 and normalized RGBA8 at20, stride24.
The full GPU address type is preserved for uniform bindings.

All modes test depth with Less. Opaque and cutout write depth, with cutout
alpha<0.5 discarded by the fragment shader. Translucent draws disable depth writes
and enable straight-alpha blending (SrcAlpha/InvSrcAlpha for colour,
One/InvSrcAlpha for alpha). Rust provides their order. The vertex shader passes
mode to the fragment shader as a flat integer. Camera MVP uses SDK zero-to-one
depth and upper-left framebuffer origin without another projection conversion.

Shutdown waits for GPU idle, destroys queue, command buffer and swapchain before
backing memory, then releases the device. It is safe after partial initialization.
`DkImage` values themselves do not have a separate destruction call.

Build the complete app with `bash scripts/build-switch.sh docker texture`.
For a compile-only C/shader check, run `make` in `native/` inside the existing SDK
image. Strict C flags include `-Wall -Wextra -Werror -Wframe-larger-than=32768`.
Neither compile checks nor host tests prove physical texture sampling, alpha,
colour output or frame timing. The original 0.8 hardware pass applies only to its
untextured target. This serialized diagnostic bridge is not a throughput result
or a final wgpu integration.

Original CraftRustNX changes use the repository MIT license. Linked deko3d uses
the zlib license; SDK notices ship beside the NRO. The lifecycle follows the
Switch examples identified as public domain by switchbrew/switch-examples.
