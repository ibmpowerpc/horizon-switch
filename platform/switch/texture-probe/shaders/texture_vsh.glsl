#version 460

// std140 uniform block for MVP matrix (64 bytes) + mode (4 bytes)
layout (std140, binding = 0) uniform Ubo
{
    mat4 mvp;
    uint mode;
};

// Vertex attribute: 24 bytes (float3 position + float2 uv + uint32 RGBA8)
layout (location = 0) in vec3 inPosition;
layout (location = 1) in vec2 inUV;
layout (location = 2) in vec4 inColor;

layout (location = 0) out vec2 outUV;
layout (location = 1) out vec4 outColor;
layout (location = 2) flat out uint outMode;

void main()
{
    gl_Position = mvp * vec4(inPosition, 1.0);
    outUV = inUV;
    outColor = inColor;
    outMode = mode;
}
