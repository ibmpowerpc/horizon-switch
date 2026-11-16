#version 460

// std140 uniform block for triangle vertex data (96 bytes)
layout (std140, binding = 0) uniform VertexData
{
    vec4 positions[3];  // 48 bytes: 3 vec4s at 16 bytes each
    vec4 colors[3];     // 48 bytes: 3 vec4s at 16 bytes each
};

layout (location = 0) out vec4 outColor;

void main()
{
    gl_Position = positions[gl_VertexID];
    outColor = colors[gl_VertexID];
}
