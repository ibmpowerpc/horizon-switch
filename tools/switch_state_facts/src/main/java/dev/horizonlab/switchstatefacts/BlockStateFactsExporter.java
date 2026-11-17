package dev.horizonlab.switchstatefacts;

import com.google.gson.Gson;
import com.google.gson.stream.JsonWriter;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.IdMapper;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.level.material.FluidState;

import java.io.BufferedWriter;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardCopyOption;
import java.io.FilterOutputStream;
import java.io.OutputStream;
import java.io.OutputStreamWriter;
import java.nio.file.StandardOpenOption;

/**
 * Minimal exporter of block-state semantic facts for Switch model selection.
 * Reads vanilla registries and emits streaming JSON with state metadata.
 */
public final class BlockStateFactsExporter {
    private static final EmptyBlockGetter EMPTY_LEVEL = EmptyBlockGetter.INSTANCE;

    private BlockStateFactsExporter() {}

    public static void main(String[] args) throws Exception {
        if (args.length != 1) throw new IllegalArgumentException("one output path required");
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();

        Gson gson = new Gson();

        // Get the registry
        IdMapper<BlockState> registry = Block.BLOCK_STATE_REGISTRY;
        int stateCount = registry.size();

        Path target = Paths.get(args[0]);
        Path partial = target.resolveSibling(target.getFileName() + ".partial");

        if (Files.exists(target)) throw new IOException("output already exists");
        try (OutputStream bytes = new FilterOutputStream(Files.newOutputStream(partial,
                StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE)) {
            private long written;
            private void reserve(int n) throws IOException {
                if (n < 0 || written + n > 12L * 1024 * 1024)
                    throw new IOException("state output exceeds 12 MiB");
                written += n;
            }
            @Override public void write(int b) throws IOException { reserve(1); out.write(b); }
            @Override public void write(byte[] b, int off, int len) throws IOException {
                reserve(len); out.write(b, off, len);
            }
        }; JsonWriter out = gson.newJsonWriter(new BufferedWriter(
                new OutputStreamWriter(bytes, StandardCharsets.UTF_8)))) {
            out.beginObject();
            out.name("metadata").beginObject();
            out.name("schema").value(2);
            out.name("version").value(SharedConstants.getCurrentVersion().id());
            out.name("protocol").value(SharedConstants.getProtocolVersion());
            out.name("jar_sha1").value("e877b6a07acd633fb3bb475002175cec036e7b87");
            out.name("runtime").value(System.getProperty("java.version"));
            out.name("state_count").value(stateCount);
            out.endObject();

            out.name("states").beginArray();

            // Iterate through all states by ID
            for (int id = 0; id < stateCount; id++) {
                BlockState state = registry.byId(id);
                if (state == null) {
                    throw new IOException("missing state at ID " + id);
                }

                out.beginObject();
                out.name("id").value(id);

                String blockName = BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString();
                out.name("block").value(blockName);

                out.name("properties").beginObject();
                for (var value : state.getValues().toList())
                    out.name(value.property().getName()).value(value.valueName());
                out.endObject();

                out.name("is_air").value(state.isAir());

                // Check if fluid is empty
                FluidState fluid = state.getFluidState();
                out.name("fluid_empty").value(fluid.isEmpty());

                out.name("render_shape").value(state.getRenderShape().name());
                out.name("solid_render").value(state.isSolidRender());
                out.name("can_occlude").value(state.canOcclude());
                out.name("full_collision").value(state.isCollisionShapeFullBlock(EMPTY_LEVEL, BlockPos.ZERO));
                out.name("map_color").value(state.getMapColor(EMPTY_LEVEL, BlockPos.ZERO).col);

                out.endObject();
            }

            out.endArray();
            out.endObject();
        }

        Files.move(partial, target, StandardCopyOption.ATOMIC_MOVE);
    }
}
