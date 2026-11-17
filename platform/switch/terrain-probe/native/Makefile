# Makefile for terrain GPU bridge native implementation
# Build with: make -C platform/switch/terrain-probe/native
# Output: native/build/terrain_native.a

DEVKITPRO ?= /opt/devkitpro
DEVKITA64 ?= $(DEVKITPRO)/devkitA64

BUILD_DIR := build
TARGET_DIR := $(BUILD_DIR)

CROSS ?= aarch64-none-elf
CC := $(DEVKITA64)/bin/$(CROSS)-gcc
AR := $(DEVKITA64)/bin/$(CROSS)-gcc-ar
UAM := $(DEVKITPRO)/tools/bin/uam

CFLAGS := -Wall -Wextra -Werror -Wframe-larger-than=32768
CFLAGS += -O2 -g
CFLAGS += -march=armv8-a+crc+simd -mtune=cortex-a57 -mtp=soft -fPIC
CFLAGS += -ffunction-sections -fdata-sections
CFLAGS += -D__SWITCH__

INCLUDES := -I../include -I$(DEVKITPRO)/libnx/include -I$(DEVKITPRO)/portlibs/switch/include

LDFLAGS := -march=armv8-a+crc+simd -mtune=cortex-a57 -mtp=soft
LDFLAGS += -specs=$(DEVKITPRO)/libnx/switch.specs

LIBS := -lnx -lm

SRC := terrain_native.c
OBJ := $(SRC:.c=.o)
TARGET := $(BUILD_DIR)/terrain_native.a

.PHONY: all clean mkdir

all: mkdir $(TARGET)

mkdir:
	@mkdir -p $(BUILD_DIR)

$(TARGET): $(OBJ)
	$(AR) rcs $@ $^

%.o: %.c
	$(CC) $(CFLAGS) $(INCLUDES) -c $< -o $@

clean:
	rm -rf $(BUILD_DIR)