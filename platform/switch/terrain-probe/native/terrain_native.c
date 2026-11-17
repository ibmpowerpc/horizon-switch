/* deko3d GPU bridge for Rust-owned untextured 3D meshes.
 * Based on gpu-probe/native/gpu_native.c which is based on
 * devkitpro/examples/switch/graphics/deko3d/deko_basic/source/main.c
 * switchbrew/switch-examples identifies its examples as public domain.
 * Modified for CraftRustNX terrain probe: dynamic Rust MVP, double-buffered
 * 1280x720 RGBA8 + depth, bounded vertex storage, 64-byte column-major MVP.
 * Original CraftRustNX changes follow the repository MIT license.
 * Linked deko3d is Copyright (C) 2018-2020 fincs, zlib license. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <stdatomic.h>
#include <switch.h>
#include <deko3d.h>
#include "../include/terrain_gpu.h"

/* SDK NDC/clip-depth defaults (DkDeviceMaker defaults):
 * - Depth range: 0.0 (near) to 1.0 (far) - DkDeviceFlags_DepthZeroToOne
 * - Origin: upper-left - DkDeviceFlags_OriginUpperLeft
 * The supplied MVP is used as-is; no modification is performed. */

#define FB_NUM 2
#define FB_WIDTH 1280
#define FB_HEIGHT 720
#define CODEMEMSIZE (128*1024)
#define CMDMEMSIZE (32*1024)
#define UNIFORMSIZE DK_MEMBLOCK_ALIGNMENT
#define MAX_VERTICES TP_MAX_VERTICES

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
    /* Bounded GPU-owned storage; Rust owns the source geometry. */
    DkMemBlock vertexMemBlock;
    uint32_t vertexSize;
    uint32_t vertexCount;
    uint32_t vertexMaxSize;
} g_ctx;

/* Load shader from romfs with bounds checking */
static int load_shader(DkShader* pShader, const char* path)
{
    FILE* f = fopen(path, "rb");
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
    uint32_t aligned_size = (size + DK_SHADER_CODE_ALIGNMENT - 1) & ~(DK_SHADER_CODE_ALIGNMENT - 1);
    if (g_ctx.codeMemOffset + aligned_size > CODEMEMSIZE) {
        fclose(f);
        return -1;
    }

    uint32_t codeOffset = g_ctx.codeMemOffset;
    void* dest = (uint8_t*)dkMemBlockGetCpuAddr(g_ctx.codeMemBlock) + codeOffset;

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

/* Validate mesh arguments per terrain_gpu.h spec:
 * - count <= TP_MAX_VERTICES
 * - count == 0 is allowed (clears mesh)
 * - count must be divisible by 3 for triangle list
 * - vertices may be null when count == 0
 * Returns 0 on valid, -1 on invalid (leaves previous mesh) */
static int validate_mesh(const struct tp_vertex *vertices, uint32_t count)
{
    if (count == 0) {
        return 0; /* Empty mesh is valid */
    }
    if (count > TP_MAX_VERTICES) {
        return -1; /* Too many vertices */
    }
    if (count % 3 != 0) {
        return -1; /* Must be triangle list */
    }
    if (!vertices) {
        return -1; /* Non-zero count requires non-null vertices */
    }
    return 0;
}

/* Check for non-finite values in vertex positions */
static int verify_finite_positions(const struct tp_vertex *vertices, uint32_t count)
{
    if (!vertices || count == 0) {
        return 0;
    }
    for (uint32_t i = 0; i < count; i++) {
        const float *pos = vertices[i].position;
        for (int j = 0; j < 3; j++) {
            if (!isfinite(pos[j])) {
                return -1; /* Non-finite position */
            }
        }
    }
    return 0;
}

int32_t tp_graphics_initialize(void)
{
    if (g_ctx.initialized) {
        return 0; /* Already initialized */
    }

    tp_graphics_shutdown();

    /* Create device */
    DkDeviceMaker deviceMaker;
    dkDeviceMakerDefaults(&deviceMaker);
    g_ctx.device = dkDeviceCreate(&deviceMaker);
    if (!g_ctx.device) {
        tp_graphics_shutdown();
        return -2;
    }

    /* Calculate framebuffer layout */
    DkImageLayoutMaker imageLayoutMaker;
    dkImageLayoutMakerDefaults(&imageLayoutMaker, g_ctx.device);
    imageLayoutMaker.flags = DkImageFlags_UsageRender | DkImageFlags_UsagePresent | DkImageFlags_HwCompression;
    imageLayoutMaker.format = DkImageFormat_RGBA8_Unorm;
    imageLayoutMaker.dimensions[0] = FB_WIDTH;
    imageLayoutMaker.dimensions[1] = FB_HEIGHT;

    DkImageLayout framebufferLayout;
    dkImageLayoutInitialize(&framebufferLayout, &imageLayoutMaker);

    uint32_t framebufferSize = (uint32_t)dkImageLayoutGetSize(&framebufferLayout);
    uint32_t framebufferAlign = dkImageLayoutGetAlignment(&framebufferLayout);
    framebufferSize = (framebufferSize + framebufferAlign - 1) & ~(framebufferAlign - 1);

    /* Calculate depth buffer layout */
    DkImageLayoutMaker depthLayoutMaker;
    dkImageLayoutMakerDefaults(&depthLayoutMaker, g_ctx.device);
    depthLayoutMaker.flags = DkImageFlags_UsageRender | DkImageFlags_HwCompression;
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
    dkMemBlockMakerDefaults(&memBlockMaker, g_ctx.device, FB_NUM * framebufferSize);
    memBlockMaker.flags = DkMemBlockFlags_GpuCached | DkMemBlockFlags_Image;
    g_ctx.framebufferMemBlock = dkMemBlockCreate(&memBlockMaker);
    if (!g_ctx.framebufferMemBlock) {
        tp_graphics_shutdown();
        return -3;
    }
    g_ctx.allocatedBytes += FB_NUM * framebufferSize;

    /* Initialize framebuffers */
    DkImage const* swapchainImages[FB_NUM];
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
        tp_graphics_shutdown();
        return -4;
    }
    g_ctx.allocatedBytes += FB_NUM * depthSize;

    /* Initialize depth buffers */
    for (unsigned i = 0; i < FB_NUM; i++) {
        dkImageInitialize(&g_ctx.depths[i], &depthLayout,
                         g_ctx.depthMemBlock, i * depthSize);
    }

    /* Create swapchain */
    DkSwapchainMaker swapchainMaker;
    dkSwapchainMakerDefaults(&swapchainMaker, g_ctx.device, nwindowGetDefault(),
                            swapchainImages, FB_NUM);
    g_ctx.swapchain = dkSwapchainCreate(&swapchainMaker);
    if (!g_ctx.swapchain) {
        tp_graphics_shutdown();
        return -5;
    }

    /* Create code memory block */
    dkMemBlockMakerDefaults(&memBlockMaker, g_ctx.device, CODEMEMSIZE);
    memBlockMaker.flags = DkMemBlockFlags_CpuUncached | DkMemBlockFlags_GpuCached | DkMemBlockFlags_Code;
    g_ctx.codeMemBlock = dkMemBlockCreate(&memBlockMaker);
    if (!g_ctx.codeMemBlock) {
        tp_graphics_shutdown();
        return -6;
    }
    g_ctx.allocatedBytes += CODEMEMSIZE;
    g_ctx.codeMemOffset = 0;

    /* Load shaders */
    if (load_shader(&g_ctx.vertexShader, "romfs:/shaders/terrain_vsh.dksh") != 0) {
        tp_graphics_shutdown();
        return -7;
    }
    if (load_shader(&g_ctx.fragmentShader, "romfs:/shaders/terrain_fsh.dksh") != 0) {
        tp_graphics_shutdown();
        return -8;
    }

    /* Create command buffer memory */
    dkMemBlockMakerDefaults(&memBlockMaker, g_ctx.device, CMDMEMSIZE);
    memBlockMaker.flags = DkMemBlockFlags_CpuUncached | DkMemBlockFlags_GpuCached;
    g_ctx.cmdbufMemBlock = dkMemBlockCreate(&memBlockMaker);
    if (!g_ctx.cmdbufMemBlock) {
        tp_graphics_shutdown();
        return -9;
    }
    g_ctx.allocatedBytes += CMDMEMSIZE;

    /* Create uniform buffer memory (64-byte MVP + alignment) */
    dkMemBlockMakerDefaults(&memBlockMaker, g_ctx.device, UNIFORMSIZE);
    memBlockMaker.flags = DkMemBlockFlags_CpuUncached | DkMemBlockFlags_GpuCached;
    g_ctx.uniformMemBlock = dkMemBlockCreate(&memBlockMaker);
    if (!g_ctx.uniformMemBlock) {
        tp_graphics_shutdown();
        return -10;
    }
    g_ctx.allocatedBytes += UNIFORMSIZE;

    /* Create vertex storage memory (bounded, GPU-owned) */
    g_ctx.vertexMaxSize = (MAX_VERTICES * sizeof(struct tp_vertex)
        + DK_MEMBLOCK_ALIGNMENT - 1) & ~(DK_MEMBLOCK_ALIGNMENT - 1);
    dkMemBlockMakerDefaults(&memBlockMaker, g_ctx.device, g_ctx.vertexMaxSize);
    memBlockMaker.flags = DkMemBlockFlags_CpuUncached | DkMemBlockFlags_GpuCached;
    g_ctx.vertexMemBlock = dkMemBlockCreate(&memBlockMaker);
    if (!g_ctx.vertexMemBlock) {
        tp_graphics_shutdown();
        return -11;
    }
    g_ctx.allocatedBytes += g_ctx.vertexMaxSize;
    g_ctx.vertexSize = 0;
    g_ctx.vertexCount = 0;

    /* Create command buffer */
    DkCmdBufMaker cmdbufMaker;
    dkCmdBufMakerDefaults(&cmdbufMaker, g_ctx.device);
    g_ctx.cmdbuf = dkCmdBufCreate(&cmdbufMaker);
    if (!g_ctx.cmdbuf) {
        tp_graphics_shutdown();
        return -12;
    }

    dkCmdBufAddMemory(g_ctx.cmdbuf, g_ctx.cmdbufMemBlock, 0, CMDMEMSIZE);

    /* Create render queue */
    DkQueueMaker queueMaker;
    dkQueueMakerDefaults(&queueMaker, g_ctx.device);
    queueMaker.flags = DkQueueFlags_Graphics;
    g_ctx.renderQueue = dkQueueCreate(&queueMaker);
    if (!g_ctx.renderQueue) {
        tp_graphics_shutdown();
        return -13;
    }

    g_ctx.initialized = 1;
    return 0;
}

int32_t tp_graphics_upload(const struct tp_vertex *vertices, uint32_t count)
{
    if (!g_ctx.initialized) {
        return -1;
    }

    /* Validate arguments */
    if (validate_mesh(vertices, count) != 0) {
        return -2; /* Invalid arguments - retain old mesh */
    }

    /* Verify finite positions */
    if (verify_finite_positions(vertices, count) != 0) {
        return -3; /* Non-finite positions - retain old mesh */
    }

    dkQueueWaitIdle(g_ctx.renderQueue);
    if (dkQueueIsInErrorState(g_ctx.renderQueue)) return -4;

    /* count == 0 clears mesh */
    if (count == 0) {
        g_ctx.vertexSize = 0;
        g_ctx.vertexCount = 0;
        return 0;
    }

    /* Synchronously copy caller memory to GPU-owned bounded storage */
    void* vertexCpu = dkMemBlockGetCpuAddr(g_ctx.vertexMemBlock);
    if (!vertexCpu) {
        return -4; /* Cannot map vertex memory */
    }

    size_t copySize = count * sizeof(struct tp_vertex);
    memcpy(vertexCpu, vertices, copySize);
    atomic_thread_fence(memory_order_seq_cst);

    g_ctx.vertexSize = (uint32_t)copySize;
    g_ctx.vertexCount = count;

    return 0;
}

int32_t tp_graphics_present(const float mvp[16], const float clear[4])
{
    if (!g_ctx.initialized) {
        return -1;
    }
    if (!mvp || !clear) return -6;
    for (unsigned i = 0; i < 16; ++i) if (!isfinite(mvp[i])) return -6;
    for (unsigned i = 0; i < 4; ++i) if (!isfinite(clear[i])) return -6;

    /* Wait for GPU to finish before reusing shared memory */
    dkQueueWaitIdle(g_ctx.renderQueue);
    if (dkQueueIsInErrorState(g_ctx.renderQueue)) {
        return -4;
    }

    /* Clear command buffer for fresh rendering */
    dkCmdBufClear(g_ctx.cmdbuf);

    /* Copy MVP to CPU-uncached GPU memory (64-byte column-major) */
    void* uniformCpu = dkMemBlockGetCpuAddr(g_ctx.uniformMemBlock);
    if (!uniformCpu) {
        return -2;
    }

    /* Ensure 16-byte alignment for std140 mat4 (dk uniform buffer alignment is 256) */
    memcpy(uniformCpu, mvp, 64);
    atomic_thread_fence(memory_order_seq_cst);

    /* Acquire framebuffer */
    int slot = dkQueueAcquireImage(g_ctx.renderQueue, g_ctx.swapchain);
    if (slot < 0 || slot >= FB_NUM) {
        return -3;
    }

    /* Bind fresh render targets after acquire */
    DkImageView colorView, depthView;
    dkImageViewDefaults(&colorView, &g_ctx.framebuffers[slot]);
    dkImageViewDefaults(&depthView, &g_ctx.depths[slot]);
    dkCmdBufBindRenderTarget(g_ctx.cmdbuf, &colorView, &depthView);
    dkCmdBufBarrier(g_ctx.cmdbuf, DkBarrier_Full,
        DkInvalidateFlags_Shader | DkInvalidateFlags_L2Cache);

    /* Build render command list - per-frame clearing avoids command growth */
    DkViewport viewport = { 0.0f, 0.0f, (float)FB_WIDTH, (float)FB_HEIGHT, 0.0f, 1.0f };
    DkScissor scissor = { 0, 0, FB_WIDTH, FB_HEIGHT };
    DkShader const* shaders[] = { &g_ctx.vertexShader, &g_ctx.fragmentShader };
    DkRasterizerState rasterizerState;
    DkColorState colorState;
    DkColorWriteState colorWriteState;
    DkDepthStencilState depthState;

    dkRasterizerStateDefaults(&rasterizerState);
    rasterizerState.cullMode = DkFace_None;
    dkColorStateDefaults(&colorState);
    dkColorWriteStateDefaults(&colorWriteState);
    dkDepthStencilStateDefaults(&depthState);
    depthState.depthTestEnable = true;
    depthState.depthWriteEnable = true;
    depthState.depthCompareOp = DkCompareOp_Less;

    /* Clear color and depth */
    dkCmdBufClearColorFloat(g_ctx.cmdbuf, 0, DkColorMask_RGBA,
                           clear[0], clear[1], clear[2], clear[3]);
    dkCmdBufClearDepthStencil(g_ctx.cmdbuf, true, 1.0f, 0, 0);

    /* Set up vertex attributes for tp_vertex16bytes layout:
     * - bufferId 0: stride 16, tp_vertex struct
     * - attrib 0: float3 position at offset 0 (DkVtxAttribSize_3x32, DkVtxAttribType_Float)
     * - attrib 1: normalized RGBA8 at offset 12 */
    DkVtxAttribState attribState[2] = {
        { .bufferId=0, .isFixed=0, .offset=0,
          .size=DkVtxAttribSize_3x32, .type=DkVtxAttribType_Float, .isBgra=0 },  /* position */
        { .bufferId=0, .isFixed=0, .offset=12,
          .size=DkVtxAttribSize_4x8, .type=DkVtxAttribType_Unorm, .isBgra=0 }  /* color */
    };

    DkVtxBufferState vbState[1] = {
        { .stride=sizeof(struct tp_vertex) }
    };

    DkBufExtents vbExtents[1] = {
        { .addr=dkMemBlockGetGpuAddr(g_ctx.vertexMemBlock), .size=g_ctx.vertexSize }
    };

    /* Bind state and draw */
    dkCmdBufSetViewports(g_ctx.cmdbuf, 0, &viewport, 1);
    dkCmdBufSetScissors(g_ctx.cmdbuf, 0, &scissor, 1);
    dkCmdBufBindShaders(g_ctx.cmdbuf, DkStageFlag_GraphicsMask, shaders, 2);
    dkCmdBufBindRasterizerState(g_ctx.cmdbuf, &rasterizerState);
    dkCmdBufBindColorState(g_ctx.cmdbuf, &colorState);
    dkCmdBufBindColorWriteState(g_ctx.cmdbuf, &colorWriteState);
    dkCmdBufBindDepthStencilState(g_ctx.cmdbuf, &depthState);
    dkCmdBufBindVtxAttribState(g_ctx.cmdbuf, attribState, 2);
    dkCmdBufBindVtxBufferState(g_ctx.cmdbuf, vbState, 1);
    if (g_ctx.vertexCount > 0)
        dkCmdBufBindVtxBuffer(g_ctx.cmdbuf, 0, vbExtents[0].addr, vbExtents[0].size);

    /* Bind uniform buffer (64-byte MVP, aligned to DK_UNIFORM_BUF_ALIGNMENT=256) */
    dkCmdBufBindUniformBuffer(g_ctx.cmdbuf, DkStage_Vertex, 0,
                             dkMemBlockGetGpuAddr(g_ctx.uniformMemBlock), 256);

    /* Draw triangle list if vertices exist */
    if (g_ctx.vertexCount > 0) {
        dkCmdBufDraw(g_ctx.cmdbuf, DkPrimitive_Triangles,
                    g_ctx.vertexCount, 1, 0, 0);
    }

    DkCmdList renderList = dkCmdBufFinishList(g_ctx.cmdbuf);
    if (!renderList) {
        return -5;
    }
    dkQueueSubmitCommands(g_ctx.renderQueue, renderList);

    /* Present */
    dkQueuePresentImage(g_ctx.renderQueue, g_ctx.swapchain, slot);

    return dkQueueIsInErrorState(g_ctx.renderQueue) ? -4 : 0;
}

void tp_graphics_shutdown(void)
{
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

uint64_t tp_graphics_allocated_bytes(void)
{
    return g_ctx.allocatedBytes;
}
