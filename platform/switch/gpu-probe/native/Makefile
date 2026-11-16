# Makefile for gpu-probe native SDK verification
# Compiles object file and shaders, no linking

BUILD_DIR := ../build/sdk-check
NATIVE_DIR := .
SHADERS_DIR := ../shaders
INCLUDE_DIR := ../include

DEVKITPRO ?= /opt/devkitpro

CC := /opt/devkitpro/devkitA64/bin/aarch64-none-elf-gcc
UAMSH := /opt/devkitpro/tools/bin/uam

CFLAGS := -g -Wall -Wextra -Werror -Wframe-larger-than=32768 \
          -fPIE -std=gnu11 -D__SWITCH__ \
          -I$(INCLUDE_DIR) \
          -I$(DEVKITPRO)/libnx/include \
          -I$(DEVKITPRO)/portlibs/switch/include

SOURCES := gpu_native.c
OBJECTS := $(BUILD_DIR)/gpu_native.o

VERTEX_SHADER := $(SHADERS_DIR)/probe_vsh.glsl
FRAGMENT_SHADER := $(SHADERS_DIR)/probe_fsh.glsl
VERTEX_DKSH := $(BUILD_DIR)/probe_vsh.dksh
FRAGMENT_DKSH := $(BUILD_DIR)/probe_fsh.dksh

.PHONY: all clean size

all: $(BUILD_DIR) $(OBJECTS) $(VERTEX_DKSH) $(FRAGMENT_DKSH) size

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

$(OBJECTS) $(VERTEX_DKSH) $(FRAGMENT_DKSH): | $(BUILD_DIR)

$(BUILD_DIR)/gpu_native.o: $(NATIVE_DIR)/gpu_native.c $(INCLUDE_DIR)/gpu_probe.h
	$(CC) $(CFLAGS) -c -o $@ $<

$(VERTEX_DKSH): $(VERTEX_SHADER)
	$(UAMSH) -s vert -o $@ $<

$(FRAGMENT_DKSH): $(FRAGMENT_SHADER)
	$(UAMSH) -s frag -o $@ $<

size:
	@echo "=== Artifact sizes ==="
	@du -h $(BUILD_DIR)
	@echo "=== Total build output ==="
	@du -sh $(BUILD_DIR)

clean:
	rm -rf $(BUILD_DIR)
