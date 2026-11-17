/* deko3d GPU bridge for Rust-owned textured 3D meshes.
 * Based on gpu-probe/native/gpu_native.c which is based on
 * devkitpro/examples/switch/graphics/deko3d/deko_basic/source/main.c
 * switchbrew/switch-examples identifies its examples as public domain.
 * Modified for CraftRustNX texture probe: dynamic Rust MVP, double-buffered
 * 1280x720 RGBA8 + depth, bounded vertex storage, 64-byte column-major MVP.
 * Original CraftRustNX changes follow the repository MIT license.
 * Linked deko3d is Copyright (C) 2018-2020 fincs, zlib license. */
#include "../include/texture_gpu.h"
#include <deko3d.h>
#include <math.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <switch.h>

/* SDK NDC/clip-depth defaults (DkDeviceMaker defaults):
 * - Depth range: 0.0 (near) to 1.0 (far) - DkDeviceFlags_DepthZeroToOne
 * - Origin: upper-left - DkDeviceFlags_OriginUpperLeft
 * The supplied MVP is used as-is; no modification is performed. */

#define FB_NUM 2
#define FB_WIDTH 1280
#define FB_HEIGHT 720
#define CODEMEMSIZE (128 * 1024)
#define CMDMEMSIZE (512 * 1024)
#define UNIFORMSIZE DK_MEMBLOCK_ALIGNMENT
#define MAX_VERTICES TX_MAX_VERTICES

/* Global state, single-threaded */
static struct {
  DkDevice device;
  DkMemBlock framebufferMemBlock;
  DkImage framebuffers[FB_NUM];
  DkSwapchain swapchain;
  DkMemBlock depthMemBlock;
  DkImage depths[FB_NUM];
  DkMemBlock codeMemBlock;
  uint32_t codeMemOffset;
  DkShader vertexShader;
  DkShader fragmentShader;
  DkMemBlock cmdbufMemBlock;
  DkMemBlock uniformMemBlock;
  DkCmdBuf cmdbuf;
  DkQueue renderQueue;
  uint64_t allocatedBytes;
  int initialized;
  DkMemBlock descriptorMemBlock, imageMemBlock;
  DkImage image;
  uint32_t imageBytes;
  struct tx_draw draws[TX_MAX_DRAWS];
  uint32_t drawCount;
  /* Bounded GPU-owned storage; Rust owns the source geometry. */
  DkMemBlock vertexMemBlock;
  uint32_t vertexSize;
  uint32_t vertexCount;
  uint32_t vertexMaxSize;
} g_ctx;

/* Load shader from romfs with bounds checking */
static int load_shader(DkShader *pShader, const char *path) {
  FILE *f = fopen(path, "rb");
  if (!f) {
    return -1;
  }

  if (fseek(f, 0, SEEK_END) != 0) {
    fclose(f);
    return -1;
  }

  long size_long = ftell(f);
  if (size_long < 16 || size_long > 65536) {
    fclose(f);
    return -1;
  }
  uint32_t size = (uint32_t)size_long;

  rewind(f);

  /* Check space in code memory block */
  uint32_t aligned_size =
      (size + DK_SHADER_CODE_ALIGNMENT - 1) & ~(DK_SHADER_CODE_ALIGNMENT - 1);
  if (g_ctx.codeMemOffset + aligned_size > CODEMEMSIZE) {
    fclose(f);
    return -1;
  }

  uint32_t codeOffset = g_ctx.codeMemOffset;
  void *dest = (uint8_t *)dkMemBlockGetCpuAddr(g_ctx.codeMemBlock) + codeOffset;

  size_t read = fread(dest, 1, size, f);
  fclose(f);

  if (read != size || memcmp(dest, "DKSH", 4) != 0) {
    return -1;
  }

  g_ctx.codeMemOffset += aligned_size;

  DkShaderMaker shaderMaker;
  dkShaderMakerDefaults(&shaderMaker, g_ctx.codeMemBlock, codeOffset);
  dkShaderInitialize(pShader, &shaderMaker);

  return dkShaderIsValid(pShader) ? 0 : -1;
}

_Static_assert(sizeof(struct tx_vertex) == 24, "vertex ABI");
_Static_assert(offsetof(struct tx_vertex, rgba) == 20, "color ABI");
_Static_assert(sizeof(struct tx_draw) == 16, "draw ABI");
_Static_assert(sizeof(DkImageDescriptor) == 32, "image descriptor");
_Static_assert(sizeof(DkSamplerDescriptor) == 32, "sampler descriptor");
static int validate_mesh(const struct tx_vertex *v, uint32_t count,
                         const struct tx_draw *draws, uint32_t n) {
  if (count > TX_MAX_VERTICES || n > TX_MAX_DRAWS)
    return -1;
  if (count == 0)
    return n == 0 ? 0 : -1;
  if (!v || !draws || n == 0 || count % 3)
    return -1;
  uint32_t end = 0, previous = 0;
  for (uint32_t i = 0; i < n; ++i) {
    const struct tx_draw *d = &draws[i];
    if (d->first != end || d->count == 0 || d->count > count - end ||
        d->count % 3 || d->mode > 2 || d->reserved ||
        (i && d->mode < previous) || (d->mode == 2 && d->count % 6))
      return -1;
    end += d->count;
    previous = d->mode;
  }
  if (end != count)
    return -1;
  for (uint32_t i = 0; i < count; ++i) {
    for (unsigned j = 0; j < 3; ++j)
      if (!isfinite(v[i].position[j]))
        return -1;
    for (unsigned j = 0; j < 2; ++j)
      if (!isfinite(v[i].uv[j]) || v[i].uv[j] < 0 || v[i].uv[j] > 1)
        return -1;
  }
  return 0;
}

int32_t tx_graphics_initialize(void) {
  if (g_ctx.initialized) {
    return 0; /* Already initialized */
  }

  tx_graphics_shutdown();

  /* Create device */
  DkDeviceMaker deviceMaker;
  dkDeviceMakerDefaults(&deviceMaker);
  g_ctx.device = dkDeviceCreate(&deviceMaker);
  if (!g_ctx.device) {
    tx_graphics_shutdown();
    return -2;
  }

  /* Calculate framebuffer layout */
  DkImageLayoutMaker imageLayoutMaker;
  dkImageLayoutMakerDefaults(&imageLayoutMaker, g_ctx.device);
  imageLayoutMaker.flags = DkImageFlags_UsageRender |
                           DkImageFlags_UsagePresent |
                           DkImageFlags_HwCompression;
  imageLayoutMaker.format = DkImageFormat_RGBA8_Unorm;
  imageLayoutMaker.dimensions[0] = FB_WIDTH;
  imageLayoutMaker.dimensions[1] = FB_HEIGHT;

  DkImageLayout framebufferLayout;
  dkImageLayoutInitialize(&framebufferLayout, &imageLayoutMaker);

  uint32_t framebufferSize = (uint32_t)dkImageLayoutGetSize(&framebufferLayout);
  uint32_t framebufferAlign = dkImageLayoutGetAlignment(&framebufferLayout);
  framebufferSize =
      (framebufferSize + framebufferAlign - 1) & ~(framebufferAlign - 1);

  /* Calculate depth buffer layout */
  DkImageLayoutMaker depthLayoutMaker;
  dkImageLayoutMakerDefaults(&depthLayoutMaker, g_ctx.device);
  depthLayoutMaker.flags =
      DkImageFlags_UsageRender | DkImageFlags_HwCompression;
  depthLayoutMaker.format = DkImageFormat_Z24X8;
  depthLayoutMaker.dimensions[0] = FB_WIDTH;
  depthLayoutMaker.dimensions[1] = FB_HEIGHT;

  DkImageLayout depthLayout;
  dkImageLayoutInitialize(&depthLayout, &depthLayoutMaker);

  uint32_t depthSize = (uint32_t)dkImageLayoutGetSize(&depthLayout);
  uint32_t depthAlign = dkImageLayoutGetAlignment(&depthLayout);
  depthSize = (depthSize + depthAlign - 1) & ~(depthAlign - 1);

  /* Create framebuffer memory (double-buffered) */
  DkMemBlockMaker memBlockMaker;
  dkMemBlockMakerDefaults(&memBlockMaker, g_ctx.device,
                          FB_NUM * framebufferSize);
  memBlockMaker.flags = DkMemBlockFlags_GpuCached | DkMemBlockFlags_Image;
  g_ctx.framebufferMemBlock = dkMemBlockCreate(&memBlockMaker);
  if (!g_ctx.framebufferMemBlock) {
    tx_graphics_shutdown();
    return -3;
  }
  g_ctx.allocatedBytes += FB_NUM * framebufferSize;

  /* Initialize framebuffers */
  DkImage const *swapchainImages[FB_NUM];
  for (unsigned i = 0; i < FB_NUM; i++) {
    swapchainImages[i] = &g_ctx.framebuffers[i];
    dkImageInitialize(&g_ctx.framebuffers[i], &framebufferLayout,
                      g_ctx.framebufferMemBlock, i * framebufferSize);
  }

  /* Create depth buffer memory (double-buffered) */
  dkMemBlockMakerDefaults(&memBlockMaker, g_ctx.device, FB_NUM * depthSize);
  memBlockMaker.flags = DkMemBlockFlags_GpuCached | DkMemBlockFlags_Image;
  g_ctx.depthMemBlock = dkMemBlockCreate(&memBlockMaker);
  if (!g_ctx.depthMemBlock) {
    tx_graphics_shutdown();
    return -4;
  }
  g_ctx.allocatedBytes += FB_NUM * depthSize;

  /* Initialize depth buffers */
  for (unsigned i = 0; i < FB_NUM; i++) {
    dkImageInitialize(&g_ctx.depths[i], &depthLayout, g_ctx.depthMemBlock,
                      i * depthSize);
  }

  /* Create swapchain */
  DkSwapchainMaker swapchainMaker;
  dkSwapchainMakerDefaults(&swapchainMaker, g_ctx.device, nwindowGetDefault(),
                           swapchainImages, FB_NUM);
  g_ctx.swapchain = dkSwapchainCreate(&swapchainMaker);
  if (!g_ctx.swapchain) {
    tx_graphics_shutdown();
    return -5;
  }

  /* Create code memory block */
  dkMemBlockMakerDefaults(&memBlockMaker, g_ctx.device, CODEMEMSIZE);
  memBlockMaker.flags = DkMemBlockFlags_CpuUncached |
                        DkMemBlockFlags_GpuCached | DkMemBlockFlags_Code;
  g_ctx.codeMemBlock = dkMemBlockCreate(&memBlockMaker);
  if (!g_ctx.codeMemBlock) {
    tx_graphics_shutdown();
    return -6;
  }
  g_ctx.allocatedBytes += CODEMEMSIZE;
  g_ctx.codeMemOffset = 0;

  /* Load shaders */
  if (load_shader(&g_ctx.vertexShader, "romfs:/shaders/texture_vsh.dksh") !=
      0) {
    tx_graphics_shutdown();
    return -7;
  }
  if (load_shader(&g_ctx.fragmentShader, "romfs:/shaders/texture_fsh.dksh") !=
      0) {
    tx_graphics_shutdown();
    return -8;
  }

  /* Create command buffer memory */
  dkMemBlockMakerDefaults(&memBlockMaker, g_ctx.device, CMDMEMSIZE);
  memBlockMaker.flags = DkMemBlockFlags_CpuUncached | DkMemBlockFlags_GpuCached;
  g_ctx.cmdbufMemBlock = dkMemBlockCreate(&memBlockMaker);
  if (!g_ctx.cmdbufMemBlock) {
    tx_graphics_shutdown();
    return -9;
  }
  g_ctx.allocatedBytes += CMDMEMSIZE;

  /* Create uniform buffer memory (64-byte MVP + alignment) */
  dkMemBlockMakerDefaults(&memBlockMaker, g_ctx.device, UNIFORMSIZE);
  memBlockMaker.flags = DkMemBlockFlags_CpuUncached | DkMemBlockFlags_GpuCached;
  g_ctx.uniformMemBlock = dkMemBlockCreate(&memBlockMaker);
  if (!g_ctx.uniformMemBlock) {
    tx_graphics_shutdown();
    return -10;
  }
  g_ctx.allocatedBytes += UNIFORMSIZE;

  /* Create vertex storage memory (bounded, GPU-owned) */
  g_ctx.vertexMaxSize =
      (MAX_VERTICES * sizeof(struct tx_vertex) + DK_MEMBLOCK_ALIGNMENT - 1) &
      ~(DK_MEMBLOCK_ALIGNMENT - 1);
  dkMemBlockMakerDefaults(&memBlockMaker, g_ctx.device, g_ctx.vertexMaxSize);
  memBlockMaker.flags = DkMemBlockFlags_CpuUncached | DkMemBlockFlags_GpuCached;
  g_ctx.vertexMemBlock = dkMemBlockCreate(&memBlockMaker);
  if (!g_ctx.vertexMemBlock) {
    tx_graphics_shutdown();
    return -11;
  }
  g_ctx.allocatedBytes += g_ctx.vertexMaxSize;
  g_ctx.vertexSize = 0;
  g_ctx.vertexCount = 0;

  dkMemBlockMakerDefaults(&memBlockMaker, g_ctx.device, DK_MEMBLOCK_ALIGNMENT);
  memBlockMaker.flags = DkMemBlockFlags_CpuUncached | DkMemBlockFlags_GpuCached;
  g_ctx.descriptorMemBlock = dkMemBlockCreate(&memBlockMaker);
  if (!g_ctx.descriptorMemBlock) {
    tx_graphics_shutdown();
    return -14;
  }
  g_ctx.allocatedBytes += DK_MEMBLOCK_ALIGNMENT;

  /* Create command buffer */
  DkCmdBufMaker cmdbufMaker;
  dkCmdBufMakerDefaults(&cmdbufMaker, g_ctx.device);
  g_ctx.cmdbuf = dkCmdBufCreate(&cmdbufMaker);
  if (!g_ctx.cmdbuf) {
    tx_graphics_shutdown();
    return -12;
  }

  dkCmdBufAddMemory(g_ctx.cmdbuf, g_ctx.cmdbufMemBlock, 0, CMDMEMSIZE);

  /* Create render queue */
  DkQueueMaker queueMaker;
  dkQueueMakerDefaults(&queueMaker, g_ctx.device);
  queueMaker.flags = DkQueueFlags_Graphics;
  g_ctx.renderQueue = dkQueueCreate(&queueMaker);
  if (!g_ctx.renderQueue) {
    tx_graphics_shutdown();
    return -13;
  }

  g_ctx.initialized = 1;
  return 0;
}

int32_t tx_graphics_atlas(const uint8_t *rgba, uint32_t width,
                          uint32_t height) {
  if (!g_ctx.initialized || !rgba || width == 0 || height == 0 ||
      width > TX_MAX_DIMENSION || height > TX_MAX_DIMENSION ||
      (width & (width - 1)) || (height & (height - 1)))
    return -1;
  dkQueueWaitIdle(g_ctx.renderQueue);
  if (dkQueueIsInErrorState(g_ctx.renderQueue))
    return -2;
  DkImageLayoutMaker lm;
  dkImageLayoutMakerDefaults(&lm, g_ctx.device);
  lm.format = DkImageFormat_RGBA8_Unorm_sRGB;
  lm.dimensions[0] = width;
  lm.dimensions[1] = height;
  lm.dimensions[2] = 1;
  DkImageLayout layout;
  dkImageLayoutInitialize(&layout, &lm);
  const uint64_t logical = dkImageLayoutGetSize(&layout);
  uint32_t alignment = dkImageLayoutGetAlignment(&layout);
  if (alignment < DK_MEMBLOCK_ALIGNMENT)
    alignment = DK_MEMBLOCK_ALIGNMENT;
  if (!logical || logical > UINT32_MAX - alignment || !alignment ||
      (alignment & (alignment - 1)))
    return -3;
  const uint32_t image_bytes =
      ((uint32_t)logical + alignment - 1) & ~(alignment - 1);
  const uint32_t raw_bytes = width * height * 4;
  const uint32_t staging_bytes =
      (raw_bytes + DK_MEMBLOCK_ALIGNMENT - 1) & ~(DK_MEMBLOCK_ALIGNMENT - 1);
  DkMemBlockMaker mm;
  dkMemBlockMakerDefaults(&mm, g_ctx.device, image_bytes);
  mm.flags = DkMemBlockFlags_Image | DkMemBlockFlags_GpuCached;
  DkMemBlock image_mem = dkMemBlockCreate(&mm);
  if (!image_mem)
    return -3;
  dkMemBlockMakerDefaults(&mm, g_ctx.device, staging_bytes);
  mm.flags = DkMemBlockFlags_CpuUncached | DkMemBlockFlags_GpuCached;
  DkMemBlock staging = dkMemBlockCreate(&mm);
  if (!staging) {
    dkMemBlockDestroy(image_mem);
    return -3;
  }
  void *cpu = dkMemBlockGetCpuAddr(staging);
  uint8_t *descriptor_cpu = dkMemBlockGetCpuAddr(g_ctx.descriptorMemBlock);
  if (!cpu || !descriptor_cpu) {
    dkMemBlockDestroy(staging);
    dkMemBlockDestroy(image_mem);
    return -4;
  }
  /* Copy actual source bytes, not the rounded staging allocation size. */
  memcpy(cpu, rgba, raw_bytes);
  atomic_thread_fence(memory_order_seq_cst);
  DkImage image;
  dkImageInitialize(&image, &layout, image_mem, 0);
  DkImageView view;
  dkImageViewDefaults(&view, &image);
  DkCopyBuf copy = {
      .addr = dkMemBlockGetGpuAddr(staging), .rowLength = 0, .imageHeight = 0};
  DkImageRect rect = {.width = width, .height = height, .depth = 1};
  dkCmdBufClear(g_ctx.cmdbuf);
  dkCmdBufBarrier(g_ctx.cmdbuf, DkBarrier_Full, DkInvalidateFlags_L2Cache);
  dkCmdBufCopyBufferToImage(g_ctx.cmdbuf, &copy, &view, &rect, 0);
  dkCmdBufBarrier(g_ctx.cmdbuf, DkBarrier_Full,
                  DkInvalidateFlags_Image | DkInvalidateFlags_L2Cache);
  DkCmdList list = dkCmdBufFinishList(g_ctx.cmdbuf);
  if (!list) {
    dkMemBlockDestroy(staging);
    dkMemBlockDestroy(image_mem);
    return -5;
  }
  dkQueueSubmitCommands(g_ctx.renderQueue, list);
  dkQueueWaitIdle(g_ctx.renderQueue);
  dkMemBlockDestroy(staging);
  if (dkQueueIsInErrorState(g_ctx.renderQueue)) {
    dkMemBlockDestroy(image_mem);
    return -2;
  }
  DkImageDescriptor id;
  dkImageDescriptorInitialize(&id, &view, false, false);
  DkSampler sampler;
  dkSamplerDefaults(&sampler);
  sampler.minFilter = DkFilter_Nearest;
  sampler.magFilter = DkFilter_Nearest;
  sampler.mipFilter = DkMipFilter_None;
  sampler.lodClampMax = 0;
  for (unsigned i = 0; i < 3; ++i)
    sampler.wrapMode[i] = DkWrapMode_ClampToEdge;
  DkSamplerDescriptor sd;
  dkSamplerDescriptorInitialize(&sd, &sampler);
  /* Commit only after the new image transfer succeeds. */
  if (g_ctx.imageMemBlock)
    dkMemBlockDestroy(g_ctx.imageMemBlock);
  g_ctx.allocatedBytes -= g_ctx.imageBytes;
  g_ctx.imageMemBlock = image_mem;
  g_ctx.image = image;
  g_ctx.imageBytes = image_bytes;
  g_ctx.allocatedBytes += image_bytes;
  memcpy(descriptor_cpu, &id, sizeof(id));
  memcpy(descriptor_cpu + 32, &sd, sizeof(sd));
  atomic_thread_fence(memory_order_seq_cst);
  return 0;
}

int32_t tx_graphics_upload(const struct tx_vertex *vertices, uint32_t count,
                           const struct tx_draw *draws, uint32_t draw_count) {
  if (!g_ctx.initialized)
    return -1;
  if (validate_mesh(vertices, count, draws, draw_count) != 0)
    return -2;
  dkQueueWaitIdle(g_ctx.renderQueue);
  if (dkQueueIsInErrorState(g_ctx.renderQueue))
    return -4;
  if (count == 0) {
    g_ctx.vertexCount = 0;
    g_ctx.vertexSize = 0;
    g_ctx.drawCount = 0;
    return 0;
  }
  void *cpu = dkMemBlockGetCpuAddr(g_ctx.vertexMemBlock);
  if (!cpu)
    return -4;
  memcpy(cpu, vertices, count * sizeof(*vertices));
  memcpy(g_ctx.draws, draws, draw_count * sizeof(*draws));
  atomic_thread_fence(memory_order_seq_cst);
  g_ctx.vertexCount = count;
  g_ctx.vertexSize = count * sizeof(*vertices);
  g_ctx.drawCount = draw_count;
  return 0;
}

int32_t tx_graphics_present(const float mvp[16], const float clear[4]) {
  if (!g_ctx.initialized || !g_ctx.imageMemBlock)
    return -1;
  if (!mvp || !clear)
    return -6;
  for (unsigned i = 0; i < 16; ++i)
    if (!isfinite(mvp[i]))
      return -6;
  for (unsigned i = 0; i < 4; ++i)
    if (!isfinite(clear[i]))
      return -6;
  dkQueueWaitIdle(g_ctx.renderQueue);
  if (dkQueueIsInErrorState(g_ctx.renderQueue))
    return -4;
  dkCmdBufClear(g_ctx.cmdbuf);
  uint8_t *uniform = dkMemBlockGetCpuAddr(g_ctx.uniformMemBlock);
  if (!uniform)
    return -2;
  for (uint32_t mode = 0; mode < 3; ++mode) {
    memcpy(uniform + mode * 256, mvp, 64);
    memcpy(uniform + mode * 256 + 64, &mode, 4);
  }
  atomic_thread_fence(memory_order_seq_cst);
  int slot = dkQueueAcquireImage(g_ctx.renderQueue, g_ctx.swapchain);
  if (slot < 0 || slot >= FB_NUM)
    return -3;
  DkImageView color, depth;
  dkImageViewDefaults(&color, &g_ctx.framebuffers[slot]);
  color.format = DkImageFormat_RGBA8_Unorm_sRGB;
  dkImageViewDefaults(&depth, &g_ctx.depths[slot]);
  dkCmdBufBindRenderTarget(g_ctx.cmdbuf, &color, &depth);
  dkCmdBufBarrier(g_ctx.cmdbuf, DkBarrier_Full,
                  DkInvalidateFlags_Shader | DkInvalidateFlags_Image |
                      DkInvalidateFlags_Descriptors |
                      DkInvalidateFlags_L2Cache);
  DkViewport viewport = {0, 0, FB_WIDTH, FB_HEIGHT, 0, 1};
  DkScissor scissor = {0, 0, FB_WIDTH, FB_HEIGHT};
  DkShader const *shaders[] = {&g_ctx.vertexShader, &g_ctx.fragmentShader};
  DkRasterizerState raster;
  dkRasterizerStateDefaults(&raster);
  raster.cullMode = DkFace_None;
  DkColorWriteState write;
  dkColorWriteStateDefaults(&write);
  DkDepthStencilState ds;
  dkDepthStencilStateDefaults(&ds);
  ds.depthTestEnable = true;
  ds.depthCompareOp = DkCompareOp_Less;
  dkCmdBufSetViewports(g_ctx.cmdbuf, 0, &viewport, 1);
  dkCmdBufSetScissors(g_ctx.cmdbuf, 0, &scissor, 1);
  dkCmdBufBindShaders(g_ctx.cmdbuf, DkStageFlag_GraphicsMask, shaders, 2);
  dkCmdBufBindRasterizerState(g_ctx.cmdbuf, &raster);
  dkCmdBufBindColorWriteState(g_ctx.cmdbuf, &write);
  dkCmdBufClearColorFloat(g_ctx.cmdbuf, 0, DkColorMask_RGBA, clear[0], clear[1],
                          clear[2], clear[3]);
  dkCmdBufClearDepthStencil(g_ctx.cmdbuf, true, 1, 0, 0);
  DkVtxAttribState attrs[] = {{.bufferId = 0,
                               .offset = 0,
                               .size = DkVtxAttribSize_3x32,
                               .type = DkVtxAttribType_Float},
                              {.bufferId = 0,
                               .offset = 12,
                               .size = DkVtxAttribSize_2x32,
                               .type = DkVtxAttribType_Float},
                              {.bufferId = 0,
                               .offset = 20,
                               .size = DkVtxAttribSize_4x8,
                               .type = DkVtxAttribType_Unorm}};
  DkVtxBufferState vb = {.stride = sizeof(struct tx_vertex)};
  dkCmdBufBindVtxAttribState(g_ctx.cmdbuf, attrs, 3);
  dkCmdBufBindVtxBufferState(g_ctx.cmdbuf, &vb, 1);
  if (g_ctx.vertexCount)
    dkCmdBufBindVtxBuffer(g_ctx.cmdbuf, 0,
                          dkMemBlockGetGpuAddr(g_ctx.vertexMemBlock),
                          g_ctx.vertexSize);
  DkGpuAddr desc = dkMemBlockGetGpuAddr(g_ctx.descriptorMemBlock);
  dkCmdBufBindImageDescriptorSet(g_ctx.cmdbuf, desc, 1);
  dkCmdBufBindSamplerDescriptorSet(g_ctx.cmdbuf, desc + 32, 1);
  dkCmdBufBindTexture(g_ctx.cmdbuf, DkStage_Fragment, 0,
                      dkMakeTextureHandle(0, 0));
  for (uint32_t i = 0; i < g_ctx.drawCount; ++i) {
    const struct tx_draw *d = &g_ctx.draws[i];
    DkColorState cs;
    dkColorStateDefaults(&cs);
    dkColorStateSetBlendEnable(&cs, 0, d->mode == TX_TRANSLUCENT);
    dkCmdBufBindColorState(g_ctx.cmdbuf, &cs);
    DkBlendState blend;
    dkBlendStateDefaults(&blend);
    dkBlendStateSetFactors(&blend, DkBlendFactor_SrcAlpha,
                           DkBlendFactor_InvSrcAlpha, DkBlendFactor_One,
                           DkBlendFactor_InvSrcAlpha);
    dkCmdBufBindBlendState(g_ctx.cmdbuf, 0, &blend);
    ds.depthWriteEnable = d->mode != TX_TRANSLUCENT;
    dkCmdBufBindDepthStencilState(g_ctx.cmdbuf, &ds);
    DkGpuAddr ubo = dkMemBlockGetGpuAddr(g_ctx.uniformMemBlock) + d->mode * 256;
    dkCmdBufBindUniformBuffer(g_ctx.cmdbuf, DkStage_Vertex, 0, ubo, 256);
    dkCmdBufDraw(g_ctx.cmdbuf, DkPrimitive_Triangles, d->count, 1, d->first, 0);
  }
  DkCmdList list = dkCmdBufFinishList(g_ctx.cmdbuf);
  if (!list)
    return -5;
  dkQueueSubmitCommands(g_ctx.renderQueue, list);
  dkQueuePresentImage(g_ctx.renderQueue, g_ctx.swapchain, slot);
  return dkQueueIsInErrorState(g_ctx.renderQueue) ? -4 : 0;
}

void tx_graphics_shutdown(void) {
  /* Wait for GPU to finish before freeing resources */
  if (g_ctx.renderQueue) {
    dkQueueWaitIdle(g_ctx.renderQueue);
    dkQueueDestroy(g_ctx.renderQueue);
    g_ctx.renderQueue = NULL;
  }

  if (g_ctx.cmdbuf) {
    dkCmdBufDestroy(g_ctx.cmdbuf);
    g_ctx.cmdbuf = NULL;
  }

  if (g_ctx.swapchain) {
    dkSwapchainDestroy(g_ctx.swapchain);
    g_ctx.swapchain = NULL;
  }

  if (g_ctx.imageMemBlock)
    dkMemBlockDestroy(g_ctx.imageMemBlock);
  if (g_ctx.descriptorMemBlock)
    dkMemBlockDestroy(g_ctx.descriptorMemBlock);

  if (g_ctx.vertexMemBlock) {
    dkMemBlockDestroy(g_ctx.vertexMemBlock);
    g_ctx.vertexMemBlock = NULL;
  }

  if (g_ctx.uniformMemBlock) {
    dkMemBlockDestroy(g_ctx.uniformMemBlock);
    g_ctx.uniformMemBlock = NULL;
  }

  if (g_ctx.cmdbufMemBlock) {
    dkMemBlockDestroy(g_ctx.cmdbufMemBlock);
    g_ctx.cmdbufMemBlock = NULL;
  }

  if (g_ctx.depthMemBlock) {
    dkMemBlockDestroy(g_ctx.depthMemBlock);
    g_ctx.depthMemBlock = NULL;
  }

  if (g_ctx.framebufferMemBlock) {
    dkMemBlockDestroy(g_ctx.framebufferMemBlock);
    g_ctx.framebufferMemBlock = NULL;
  }

  if (g_ctx.codeMemBlock) {
    dkMemBlockDestroy(g_ctx.codeMemBlock);
    g_ctx.codeMemBlock = NULL;
  }

  if (g_ctx.device) {
    dkDeviceDestroy(g_ctx.device);
    g_ctx.device = NULL;
  }

  memset(&g_ctx, 0, sizeof(g_ctx));
}

uint64_t tx_graphics_allocated_bytes(void) { return g_ctx.allocatedBytes; }
