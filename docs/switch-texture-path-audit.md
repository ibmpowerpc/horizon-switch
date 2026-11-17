# Switch texture path after Mesh Probe 0.8

## Scope and evidence

The user confirmed nine baked shapes and all controls on Switch Lite in Mesh
Probe 0.8. That proves its untextured rendering path. Textured rendering remains
unverified on hardware. Keep the 16-byte `tp_vertex` and the 0.8 target intact
while adding a separate textured experiment.

This audit uses the installed `devkitpro/devkita64:20260219` image, specifically
`/opt/devkitpro/libnx/include/deko3d.h`, and the installed examples under
`examples/switch/graphics/deko3d/deko_examples/source/`:
`Example04_TexturedCube.cpp`, `texture_fsh.glsl`,
`SampleFramework/CExternalImage.cpp` and `SampleFramework/CDescriptorSet.h`.
An AArch64 compile-only check exercised the symbols below with
`-Wall -Wextra -Werror -fPIE`; it did not execute GPU commands.

## Vertex and memory contracts

The existing vertex is position (`float[3]`) plus normalized RGBA8 at offset 12,
stride 16. A proposed textured vertex adds `float[2]` UV at offset 12 and moves
RGBA8 to offset 20, stride 24. The compile check asserts that layout. Its GPU
fetch and shader interface still need a device test.

Memory blocks require `DK_MEMBLOCK_ALIGNMENT` (4096 bytes). Uniform buffer
addresses require `DK_UNIFORM_BUF_ALIGNMENT` (256 bytes). Image and sampler
descriptor GPU addresses each require 32-byte alignment. Both descriptor records
are 32 bytes; their C alignment alone does not satisfy the GPU address contract.

Use `DkMemBlockFlags_Image` for image backing. There is no
`DkImageFlags_Image` or `DkMemBlockFlags_Buffer`. Initialize a `DkImageLayout`,
query `dkImageLayoutGetSize` and `dkImageLayoutGetAlignment`, then allocate enough
aligned backing. Raw pixel size is not the allocation size of a tiled image.

## Uploading the atlas

The default image layout is block-linear. Copying contiguous RGBA directly into
that backing does not implement the required tiling. Follow `CExternalImage`:
put pixels in staging memory, construct a `DkCopyBuf`, and issue
`dkCmdBufCopyBufferToImage` into the image view. Its tightly packed example uses
zero `rowLength` and `imageHeight`; the destination rectangle specifies width,
height and depth one. The SDK also exposes pitch-linear images with a 32-byte
stride alignment, but this audit does not establish them as the preferred path.

Submit the transfer and wait for completion before freeing staging or command
memory. Keep the image backing alive through every draw that samples it. When
updating resources, account for image, descriptor and L2 cache invalidation using
the SDK's `DkInvalidateFlags_Image`, `Descriptors` and `L2Cache` flags. GPU waits
and barriers serve different purposes; neither replaces the other's lifetime
contract.

## Descriptors, sampler and shader

Create a `DkImageView`, initialize its `DkImageDescriptor` with
`dkImageDescriptorInitialize`, and initialize a `DkSamplerDescriptor` with
`dkSamplerDescriptorInitialize`. For a static nearest-neighbour atlas, the exact
sampler settings are `DkFilter_Nearest`, `DkMipFilter_None`,
`DkWrapMode_ClampToEdge` and a maximum LOD of zero.

The installed descriptor helper uploads records with `dkCmdBufPushData` and binds
GPU tables with `dkCmdBufBindImageDescriptorSet` and
`dkCmdBufBindSamplerDescriptorSet`. Select the pair through
`dkCmdBufBindTexture(cmd, DkStage_Fragment, 0, dkMakeTextureHandle(0, 0))`.
The example shader declares `layout(binding = 0) uniform sampler2D texture0`.
Descriptor storage must remain valid until the submitted commands finish; wait
before overwriting it for another resource.

The header provides `DkImageFormat_RGBA8_Unorm_sRGB`. This establishes API
availability, not correct colour output on the Switch display. The texture
sampling format and render-target format must form a consistent colour pipeline.
The static atlas preserves straight source RGBA bytes without gamma conversion
or premultiplication. Hardware testing must check colour output before claiming
sRGB correctness.

## Alpha and Horizon semantics

Horizon's `horizon-client-render/src/models/catalog.rs` combines explicit
`RenderType`, `material.force_translucent`, sprite alpha and region opacity.
`sprites.rs` distinguishes zero alpha from partial alpha. Pixel alpha alone
therefore cannot choose the correct render layer; glass can require translucent
rendering even without partially transparent texels. Biome tinting is a separate
input to meshing, not a property reconstructed by the atlas.

Opaque and cutout draws need depth testing and writes. Cutout additionally needs
a shader discard with the appropriate verified threshold. Translucent draws need
depth testing, no depth writes and ordering from back to front. Enable blending
with `dkColorStateSetBlendEnable`, then set `DkBlendState` factors with
`dkBlendStateSetFactors`: straight-alpha colour uses `DkBlendFactor_SrcAlpha` and
`DkBlendFactor_InvSrcAlpha`; alpha accumulation uses `One` and `InvSrcAlpha`.
Bind these with `dkCmdBufBindColorState` and `dkCmdBufBindBlendState`.
The blend state has no embedded enable field.

## Size estimates and limits

| Atlas | Raw RGBA8 bytes | Raw size rounded to 4 KiB |
|---|---:|---:|
| 128 by 128 | 65,536 | 64 KiB |
| 256 by 256 | 262,144 | 256 KiB |
| 512 by 512 | 1,048,576 | 1 MiB |
| 1024 by 1024 | 4,194,304 | 4 MiB |

All four raw sizes are already 4-KiB aligned. Image-layout padding, staging,
descriptors, vertices and render targets are additional allocations. A two-pixel
gutter consumes `(width + 4) * (height + 4)` pixels per sprite. A complete mip
chain approaches four thirds of base-level texel storage, but that is not an SDK
allocation promise. The first static experiment has no mip chain.

Refuse invalid or unsupported textures explicitly. Animated/APNG/16-bit resources
must not silently become frame zero or reduced-bit images. Preserve the previous
valid resource if a replacement cannot be built.

## Remaining acceptance work

Build and run the separate textured probe: check the nine shapes, flower holes,
glass ordering and depth, nearest sampling at sprite edges, colour output and
camera controls. Measure memory and frame timing on hardware before making
performance claims. This API audit proves neither a wgpu backend nor EGL support;
it also does not validate live terrain, animation, biome lighting or gameplay.
