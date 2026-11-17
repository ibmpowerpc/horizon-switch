#version 460

layout (location = 0) in vec2 inUV;
layout (location = 1) in vec4 inColor;
layout (location = 2) flat in uint inMode;
layout (location = 0) out vec4 outColor;

layout (binding = 0) uniform sampler2D texture0;

void main()
{
    vec4 texColor = texture(texture0, inUV);
    vec4 finalColor = texColor * inColor;

    // Cutout mode: discard alpha < 0.5
    if (inMode == 1) {
        // Cutout: discard low alpha
        if (finalColor.a < 0.5) discard;
    }

    // Translucent mode (inMode == 2) uses blending, no discard
    // Opaque mode (inMode == 0) uses opaque rendering, no discard

    outColor = finalColor;
}
