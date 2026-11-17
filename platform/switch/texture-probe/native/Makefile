# Compile-only SDK check. The common probe Makefile links the actual NRO.
DEVKITPRO ?= /opt/devkitpro
CC := $(DEVKITPRO)/devkitA64/bin/aarch64-none-elf-gcc
CFLAGS := -O2 -fPIE -march=armv8-a+crc+crypto -mtune=cortex-a57 -mtp=soft -D__SWITCH__ -Wall -Wextra -Werror -Wframe-larger-than=32768
.PHONY: all
all: build/texture_native.o ../shaders/texture_vsh.dksh ../shaders/texture_fsh.dksh
build/texture_native.o: texture_native.c ../include/texture_gpu.h
	@mkdir -p build
	$(CC) $(CFLAGS) -I../include -I$(DEVKITPRO)/libnx/include -c $< -o $@
../shaders/texture_vsh.dksh: ../shaders/texture_vsh.glsl
	uam -s vert -o $@ $<
../shaders/texture_fsh.dksh: ../shaders/texture_fsh.glsl
	uam -s frag -o $@ $<
