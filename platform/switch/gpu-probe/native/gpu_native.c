/* deko3d GPU bridge for Rust smoke test.
 * Based on devkitpro/examples/switch/graphics/deko3d/deko_basic/source/main.c
 * switchbrew/switch-examples identifies its examples as public domain.
 * Modified for CraftRustNX: dynamic Rust uniforms, checked setup and teardown.
 * Original CraftRustNX changes follow the repository MIT license.
 * Linked deko3d is Copyright (C) 2018-2020 fincs, zlib license.
 */
#include "../include/gpu_probe.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdatomic.h>
#include <stddef.h>
#include <switch.h>
#include <deko3d.h>

#define FB_NUM 2
#define FB_WIDTH 1280
#define FB_HEIGHT 720
#define CODEMEMSIZE (64*1024)
#define CMDMEMSIZE (16*1024)
#define UNIFORMSIZE (16*1024)

/* Global state, single-threaded */
static struct {
    DkDevice device;
    DkMemBlock framebufferMemBlock;
    DkImage framebuffers[FB_NUM];
    DkSwapchain swapchain;
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

int32_t gp_graphics_initialize(void)
{
    if (g_ctx.initialized) {
        return 0; /* Already initialized */
    }

    gp_graphics_shutdown();

    /* Create device */
    DkDeviceMaker deviceMaker;
    dkDeviceMakerDefaults(&deviceMaker);
    g_ctx.device = dkDeviceCreate(&deviceMaker);
    if (!g_ctx.device) {
        gp_graphics_shutdown();
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

    uint32_t framebufferSize = dkImageLayoutGetSize(&framebufferLayout);
    uint32_t framebufferAlign = dkImageLayoutGetAlignment(&framebufferLayout);
    framebufferSize = (framebufferSize + framebufferAlign - 1) & ~(framebufferAlign - 1);

    /* Create framebuffer memory */
    DkMemBlockMaker memBlockMaker;
    dkMemBlockMakerDefaults(&memBlockMaker, g_ctx.device, FB_NUM * framebufferSize);
    memBlockMaker.flags = DkMemBlockFlags_GpuCached | DkMemBlockFlags_Image;
    g_ctx.framebufferMemBlock = dkMemBlockCreate(&memBlockMaker);
    if (!g_ctx.framebufferMemBlock) {
        gp_graphics_shutdown();
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

    /* Create swapchain */
    DkSwapchainMaker swapchainMaker;
    dkSwapchainMakerDefaults(&swapchainMaker, g_ctx.device, nwindowGetDefault(),
                            swapchainImages, FB_NUM);
    g_ctx.swapchain = dkSwapchainCreate(&swapchainMaker);
    if (!g_ctx.swapchain) {
        gp_graphics_shutdown();
        return -4;
    }

    /* Create code memory block */
    dkMemBlockMakerDefaults(&memBlockMaker, g_ctx.device, CODEMEMSIZE);
    memBlockMaker.flags = DkMemBlockFlags_CpuUncached | DkMemBlockFlags_GpuCached | DkMemBlockFlags_Code;
    g_ctx.codeMemBlock = dkMemBlockCreate(&memBlockMaker);
    if (!g_ctx.codeMemBlock) {
        gp_graphics_shutdown();
        return -5;
    }
    g_ctx.allocatedBytes += CODEMEMSIZE;
    g_ctx.codeMemOffset = 0;

    /* Load shaders */
    if (load_shader(&g_ctx.vertexShader, "romfs:/shaders/probe_vsh.dksh") != 0) {
        gp_graphics_shutdown();
        return -6;
    }
    if (load_shader(&g_ctx.fragmentShader, "romfs:/shaders/probe_fsh.dksh") != 0) {
        gp_graphics_shutdown();
        return -7;
    }

    /* Create command buffer memory */
    dkMemBlockMakerDefaults(&memBlockMaker, g_ctx.device, CMDMEMSIZE);
    memBlockMaker.flags = DkMemBlockFlags_CpuUncached | DkMemBlockFlags_GpuCached;
    g_ctx.cmdbufMemBlock = dkMemBlockCreate(&memBlockMaker);
    if (!g_ctx.cmdbufMemBlock) {
        gp_graphics_shutdown();
        return -8;
    }
    g_ctx.allocatedBytes += CMDMEMSIZE;

    /* Create uniform buffer memory */
    dkMemBlockMakerDefaults(&memBlockMaker, g_ctx.device, UNIFORMSIZE);
    memBlockMaker.flags = DkMemBlockFlags_CpuUncached | DkMemBlockFlags_GpuCached;
    g_ctx.uniformMemBlock = dkMemBlockCreate(&memBlockMaker);
    if (!g_ctx.uniformMemBlock) {
        gp_graphics_shutdown();
        return -9;
    }
    g_ctx.allocatedBytes += UNIFORMSIZE;

    /* Create command buffer */
    DkCmdBufMaker cmdbufMaker;
    dkCmdBufMakerDefaults(&cmdbufMaker, g_ctx.device);
    g_ctx.cmdbuf = dkCmdBufCreate(&cmdbufMaker);
    if (!g_ctx.cmdbuf) {
        gp_graphics_shutdown();
        return -10;
    }

    dkCmdBufAddMemory(g_ctx.cmdbuf, g_ctx.cmdbufMemBlock, 0, CMDMEMSIZE);

    /* Create render queue */
    DkQueueMaker queueMaker;
    dkQueueMakerDefaults(&queueMaker, g_ctx.device);
    queueMaker.flags = DkQueueFlags_Graphics;
    g_ctx.renderQueue = dkQueueCreate(&queueMaker);
    if (!g_ctx.renderQueue) {
        gp_graphics_shutdown();
        return -11;
    }

    g_ctx.initialized = 1;
    return 0;
}

int32_t gp_graphics_present(const struct gp_frame *frame)
{
    if (!g_ctx.initialized || !frame) {
        return -1;
    }

    /* Diagnostic serialization: acquire alone only fences one swapchain image;
     * it does not protect shared uniforms/command memory. Wait before every
     * overwrite, then recycle the bounded command arena. Not an FPS benchmark. */
    dkQueueWaitIdle(g_ctx.renderQueue);
    if (dkQueueIsInErrorState(g_ctx.renderQueue)) return -4;
    dkCmdBufClear(g_ctx.cmdbuf);

    /* Copy uniform data to CPU-uncached GPU memory. */
    void* uniformCpu = dkMemBlockGetCpuAddr(g_ctx.uniformMemBlock);
    if (!uniformCpu) {
        return -2;
    }

    /* Copy 96 bytes of uniform data (positions + colors) */
    memset(uniformCpu, 0, DK_UNIFORM_BUF_ALIGNMENT);
    memcpy(uniformCpu, (const unsigned char*)frame + offsetof(struct gp_frame, positions), 96);
    atomic_thread_fence(memory_order_seq_cst);

    /* Acquire framebuffer */
    int slot = dkQueueAcquireImage(g_ctx.renderQueue, g_ctx.swapchain);
    if (slot < 0 || slot >= FB_NUM) {
        return -3;
    }

    /* Build render command list */
    DkViewport viewport = { 0.0f, 0.0f, (float)FB_WIDTH, (float)FB_HEIGHT, 0.0f, 1.0f };
    DkScissor scissor = { 0, 0, FB_WIDTH, FB_HEIGHT };
    DkShader const* shaders[] = { &g_ctx.vertexShader, &g_ctx.fragmentShader };
    DkRasterizerState rasterizerState;
    DkColorState colorState;
    DkColorWriteState colorWriteState;

    dkRasterizerStateDefaults(&rasterizerState);
    rasterizerState.cullMode = DkFace_None;
    dkColorStateDefaults(&colorState);
    dkColorWriteStateDefaults(&colorWriteState);

    /* Bind framebuffer */
    DkImageView imageView;
    dkImageViewDefaults(&imageView, &g_ctx.framebuffers[slot]);
    dkCmdBufBindRenderTarget(g_ctx.cmdbuf, &imageView, NULL);
    dkCmdBufBarrier(g_ctx.cmdbuf, DkBarrier_Full,
        DkInvalidateFlags_Shader | DkInvalidateFlags_L2Cache);

    /* Render */
    dkCmdBufSetViewports(g_ctx.cmdbuf, 0, &viewport, 1);
    dkCmdBufSetScissors(g_ctx.cmdbuf, 0, &scissor, 1);
    dkCmdBufClearColorFloat(g_ctx.cmdbuf, 0, DkColorMask_RGBA,
                           frame->clear[0], frame->clear[1],
                           frame->clear[2], frame->clear[3]);
    dkCmdBufBindUniformBuffer(g_ctx.cmdbuf, DkStage_Vertex, 0,
                             dkMemBlockGetGpuAddr(g_ctx.uniformMemBlock), DK_UNIFORM_BUF_ALIGNMENT);
    dkCmdBufBindShaders(g_ctx.cmdbuf, DkStageFlag_GraphicsMask, shaders, 2);
    dkCmdBufBindRasterizerState(g_ctx.cmdbuf, &rasterizerState);
    dkCmdBufBindColorState(g_ctx.cmdbuf, &colorState);
    dkCmdBufBindColorWriteState(g_ctx.cmdbuf, &colorWriteState);
    dkCmdBufDraw(g_ctx.cmdbuf, DkPrimitive_Triangles, 3, 1, 0, 0);

    DkCmdList renderList = dkCmdBufFinishList(g_ctx.cmdbuf);
    if (!renderList) return -5;
    dkQueueSubmitCommands(g_ctx.renderQueue, renderList);

    /* Present */
    dkQueuePresentImage(g_ctx.renderQueue, g_ctx.swapchain, slot);

    return dkQueueIsInErrorState(g_ctx.renderQueue) ? -4 : 0;
}

void gp_graphics_shutdown(void)
{
    /* Wait for GPU to finish */
    if (g_ctx.renderQueue) {
        dkQueueWaitIdle(g_ctx.renderQueue);
        dkQueueDestroy(g_ctx.renderQueue);
        g_ctx.renderQueue = NULL;
    }

    if (g_ctx.cmdbuf) {
        dkCmdBufDestroy(g_ctx.cmdbuf);
        g_ctx.cmdbuf = NULL;
    }

    if (g_ctx.uniformMemBlock) {
        dkMemBlockDestroy(g_ctx.uniformMemBlock);
        g_ctx.uniformMemBlock = NULL;
    }

    if (g_ctx.cmdbufMemBlock) {
        dkMemBlockDestroy(g_ctx.cmdbufMemBlock);
        g_ctx.cmdbufMemBlock = NULL;
    }

    if (g_ctx.codeMemBlock) {
        dkMemBlockDestroy(g_ctx.codeMemBlock);
        g_ctx.codeMemBlock = NULL;
    }

    if (g_ctx.swapchain) {
        dkSwapchainDestroy(g_ctx.swapchain);
        g_ctx.swapchain = NULL;
    }

    if (g_ctx.framebufferMemBlock) {
        dkMemBlockDestroy(g_ctx.framebufferMemBlock);
        g_ctx.framebufferMemBlock = NULL;
    }

    if (g_ctx.device) {
        dkDeviceDestroy(g_ctx.device);
        g_ctx.device = NULL;
    }

    memset(&g_ctx, 0, sizeof(g_ctx));
}

uint64_t gp_graphics_allocated_bytes(void)
{
    return g_ctx.allocatedBytes;
}
