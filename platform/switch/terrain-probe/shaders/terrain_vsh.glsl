#version 460

// std140 uniform block for MVP matrix (64 bytes)
layout (std140, binding = 0) uniform MvpMatrix
{
    mat4 mvp;
};

// Vertex attribute: 16 bytes (float3 position + uint32 RGBA8)
layout (location = 0) in vec3 inPosition;
layout (location = 1) in vec4 inColor;

layout (location = 0) out vec4 outColor;

void main()
{
    gl_Position = mvp * vec4(inPosition, 1.0);
    // Vertex fetch normalizes the packed little-endian R,G,B,A bytes.
    outColor = inColor;
}
