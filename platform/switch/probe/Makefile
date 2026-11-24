#---------------------------------------------------------------------------------
.SUFFIXES:
#---------------------------------------------------------------------------------

ifeq ($(strip $(DEVKITPRO)),)
$(error "Please set DEVKITPRO in your environment. export DEVKITPRO=<path to>/devkitpro")
endif

TOPDIR ?= $(CURDIR)
include $(DEVKITPRO)/libnx/switch_rules

#---------------------------------------------------------------------------------
# TARGET is the name of the output
# BUILD is the directory where object files & intermediate files will be placed
# SOURCES is a list of directories containing source code
# DATA is a list of directories containing data files
# INCLUDES is a list of directories containing header files
# ROMFS is the directory containing data to be added to RomFS, relative to the Makefile (Optional)
#
# NO_ICON: if set to anything, do not use icon.
# NO_NACP: if set to anything, no .nacp file is generated.
# APP_TITLE is the name of the app stored in the .nacp file (Optional)
# APP_AUTHOR is the author of the app stored in the .nacp file (Optional)
# APP_VERSION is the version of the app stored in the .nacp file (Optional)
# APP_TITLEID is the titleID of the app stored in the .nacp file (Optional)
# ICON is the filename of the icon (.jpg), relative to the project folder.
#   If not set, it attempts to use one of the following (in this order):
#     - <Project name>.jpg
#     - icon.jpg
#     - <libnx folder>/default_icon.jpg
#
# CONFIG_JSON is the filename of the NPDM config file (.json), relative to the project folder.
#   If not set, it attempts to use one of the following (in this order):
#     - <Project name>.json
#     - config.json
#   If a JSON file is provided or autodetected, an ExeFS PFS0 (.nsp) is built instead
#   of a homebrew executable (.nro). This is intended to be used for sysmodules.
#   NACP building is skipped as well.
#---------------------------------------------------------------------------------
TARGET		:=	craftrustnx-probe
APP_TITLE := CraftRustNX Hardware Probe
APP_AUTHOR := CraftRustNX contributors
APP_VERSION := 0.1.0
BUILD		:=	build
ifneq ($(strip $(RUST_ARCHIVE)),)
TARGET := craftrustnx-rust-probe
BUILD := build-rust
APP_TITLE := CraftRustNX Rust Probe
DEFINES += -DCRAFTRUSTNX_RUST_PROBE
endif
SOURCES		:=	source
ifneq ($(strip $(STD_ARCHIVE)),)
TARGET := craftrustnx-std-probe
BUILD := build-std
APP_TITLE := CraftRustNX std Probe
SOURCES := source-std
endif
DATA		:=	data
INCLUDES	:=	include
ifneq ($(strip $(NET_ARCHIVE)),)
TARGET := craftrustnx-net-probe
BUILD := build-net
APP_TITLE := CraftRustNX Java Status Probe
SOURCES := source-net
INCLUDES += ../net-probe/include
endif
ifneq ($(strip $(LOGIN_ARCHIVE)),)
TARGET := craftrustnx-login-probe
BUILD := build-login
APP_TITLE := CraftRustNX Login Info Probe
APP_VERSION := 0.1.2
SOURCES := source-login
INCLUDES += ../login-probe/include
DEFINES += -Wframe-larger-than=32768
ifneq ($(strip $(ENCRYPTED_LOGIN)),)
TARGET := craftrustnx-encrypted-login-probe
BUILD := build-encrypted-login
APP_TITLE := CraftRustNX Encrypted Login Probe
APP_VERSION := 0.2.0
SOURCES += ../login-probe/crypto
DEFINES += -DCRAFTRUSTNX_ENCRYPTED_LOGIN
ifneq ($(strip $(CONFIGURATION_PROBE)),)
TARGET := craftrustnx-configuration-probe
BUILD := build-configuration
APP_TITLE := CraftRustNX Configuration Probe
APP_VERSION := 0.3.0
DEFINES += -DCRAFTRUSTNX_CONFIGURATION_PROBE
ifneq ($(strip $(PLAY_SESSION)),)
TARGET := craftrustnx-play-session-probe
BUILD := build-play-session
APP_TITLE := CraftRustNX Play Session Probe
APP_VERSION := 0.4.0
DEFINES += -DCRAFTRUSTNX_PLAY_SESSION
ifneq ($(strip $(WORLD_DATA)),)
TARGET := craftrustnx-world-data-probe
BUILD := build-world-data
APP_TITLE := CraftRustNX World Data Probe
APP_VERSION := 0.5.0
DEFINES += -DCRAFTRUSTNX_WORLD_DATA
ifneq ($(strip $(COLUMN_SNAPSHOT)),)
TARGET := craftrustnx-column-snapshot-probe
BUILD := build-column-snapshot
APP_TITLE := CraftRustNX Column Snapshot Probe
APP_VERSION := 0.7.0
DEFINES += -DCRAFTRUSTNX_COLUMN_SNAPSHOT
ifneq ($(strip $(COLUMN_EXPORT)),)
TARGET := craftrustnx-column-export-probe
BUILD := build-column-export
APP_TITLE := CraftRustNX Column Export Probe
APP_VERSION := 0.10.0
DEFINES += -DCRAFTRUSTNX_COLUMN_EXPORT
endif
endif
endif
endif
endif
endif
endif
ifneq ($(strip $(GPU_ARCHIVE)),)
TARGET := craftrustnx-gpu-probe
BUILD := build-gpu
APP_TITLE := CraftRustNX Rust GPU Probe
APP_VERSION := 0.6.0
SOURCES := source-gpu ../gpu-probe/native
INCLUDES += ../gpu-probe/include
ROMFS := build-gpu-romfs
GRAPHICS_SHADER_DIR := ../gpu-probe/shaders
GRAPHICS_SHADER_STEM := probe
DEFINES += -Wframe-larger-than=32768
endif
ifneq ($(strip $(TERRAIN_ARCHIVE)),)
TARGET := craftrustnx-terrain-probe
BUILD := build-terrain
APP_TITLE := CraftRustNX Rust Model Mesh Probe
APP_VERSION := 0.8.0
SOURCES := ../terrain-probe/source ../terrain-probe/native
INCLUDES += ../terrain-probe/include
ROMFS := build-terrain-romfs
GRAPHICS_SHADER_DIR := ../terrain-probe/shaders
GRAPHICS_SHADER_STEM := terrain
DEFINES += -Wframe-larger-than=32768
endif
ifneq ($(strip $(TEXTURE_ARCHIVE)),)
TARGET := craftrustnx-texture-probe
BUILD := build-texture
APP_TITLE := CraftRustNX Rust Texture Probe
APP_VERSION := 0.9.1
SOURCES := ../texture-probe/source ../texture-probe/native
INCLUDES += ../texture-probe/include ../terrain-probe/include
ROMFS := build-texture-romfs
GRAPHICS_SHADER_DIR := ../texture-probe/shaders
GRAPHICS_SHADER_STEM := texture
DEFINES += -Wframe-larger-than=32768
endif

#---------------------------------------------------------------------------------
# options for code generation
#---------------------------------------------------------------------------------
ARCH	:=	-march=armv8-a+crc+crypto -mtune=cortex-a57 -mtp=soft -fPIE

CFLAGS	:=	-g -Wall -Wextra -Werror -O2 -ffunction-sections \
			$(ARCH) $(DEFINES)

CFLAGS	+=	$(INCLUDE) -D__SWITCH__

CXXFLAGS	:= $(CFLAGS) -fno-rtti -fno-exceptions

ASFLAGS	:=	-g $(ARCH)
LDFLAGS	=	-specs=$(DEVKITPRO)/libnx/switch.specs -g $(ARCH) -Wl,-Map,$(notdir $*.map)

LIBS	:= $(RUST_ARCHIVE) $(STD_ARCHIVE) $(NET_ARCHIVE) $(LOGIN_ARCHIVE) -lnx $(if $(strip $(STD_ARCHIVE) $(NET_ARCHIVE) $(LOGIN_ARCHIVE)),-lsysbase,)
ifneq ($(strip $(GPU_ARCHIVE)),)
LIBS := $(GPU_ARCHIVE) -ldeko3d -lnx
endif
ifneq ($(strip $(TERRAIN_ARCHIVE)),)
LIBS := $(TERRAIN_ARCHIVE) -ldeko3d -lnx
endif
ifneq ($(strip $(TEXTURE_ARCHIVE)),)
LIBS := $(TEXTURE_ARCHIVE) -ldeko3d -lnx
endif
ifneq ($(strip $(ENCRYPTED_LOGIN)),)
LIBS += -lmbedcrypto
endif

#---------------------------------------------------------------------------------
# list of directories containing libraries, this must be the top level containing
# include and lib
#---------------------------------------------------------------------------------
LIBDIRS	:= $(PORTLIBS) $(LIBNX)


#---------------------------------------------------------------------------------
# no real need to edit anything past this point unless you need to add additional
# rules for different file extensions
#---------------------------------------------------------------------------------
ifneq ($(BUILD),$(notdir $(CURDIR)))
#---------------------------------------------------------------------------------

export OUTPUT	:=	$(CURDIR)/$(TARGET)
export TOPDIR	:=	$(CURDIR)

export VPATH	:=	$(foreach dir,$(SOURCES),$(CURDIR)/$(dir)) \
			$(foreach dir,$(DATA),$(CURDIR)/$(dir))

export DEPSDIR	:=	$(CURDIR)/$(BUILD)

CFILES		:=	$(foreach dir,$(SOURCES),$(notdir $(wildcard $(dir)/*.c)))
CPPFILES	:=	$(foreach dir,$(SOURCES),$(notdir $(wildcard $(dir)/*.cpp)))
SFILES		:=	$(foreach dir,$(SOURCES),$(notdir $(wildcard $(dir)/*.s)))
BINFILES	:=	$(foreach dir,$(DATA),$(notdir $(wildcard $(dir)/*.*)))

#---------------------------------------------------------------------------------
# use CXX for linking C++ projects, CC for standard C
#---------------------------------------------------------------------------------
ifeq ($(strip $(CPPFILES)),)
#---------------------------------------------------------------------------------
	export LD	:=	$(CC)
#---------------------------------------------------------------------------------
else
#---------------------------------------------------------------------------------
	export LD	:=	$(CXX)
#---------------------------------------------------------------------------------
endif
#---------------------------------------------------------------------------------

export OFILES_BIN	:=	$(addsuffix .o,$(BINFILES))
export OFILES_SRC	:=	$(CPPFILES:.cpp=.o) $(CFILES:.c=.o) $(SFILES:.s=.o)
export OFILES 	:=	$(OFILES_BIN) $(OFILES_SRC)
export HFILES_BIN	:=	$(addsuffix .h,$(subst .,_,$(BINFILES)))

export INCLUDE	:=	$(foreach dir,$(INCLUDES),-I$(CURDIR)/$(dir)) \
			$(foreach dir,$(LIBDIRS),-I$(dir)/include) \
			-I$(CURDIR)/$(BUILD)

export LIBPATHS	:=	$(foreach dir,$(LIBDIRS),-L$(dir)/lib)

ifeq ($(strip $(CONFIG_JSON)),)
	jsons := $(wildcard *.json)
	ifneq (,$(findstring $(TARGET).json,$(jsons)))
		export APP_JSON := $(TOPDIR)/$(TARGET).json
	else
		ifneq (,$(findstring config.json,$(jsons)))
			export APP_JSON := $(TOPDIR)/config.json
		endif
	endif
else
	export APP_JSON := $(TOPDIR)/$(CONFIG_JSON)
endif

ifeq ($(strip $(ICON)),)
	icons := $(wildcard *.jpg)
	ifneq (,$(findstring $(TARGET).jpg,$(icons)))
		export APP_ICON := $(TOPDIR)/$(TARGET).jpg
	else
		ifneq (,$(findstring icon.jpg,$(icons)))
			export APP_ICON := $(TOPDIR)/icon.jpg
		endif
	endif
else
	export APP_ICON := $(TOPDIR)/$(ICON)
endif

ifeq ($(strip $(NO_ICON)),)
	export NROFLAGS += --icon=$(APP_ICON)
endif

ifeq ($(strip $(NO_NACP)),)
	export NROFLAGS += --nacp=$(CURDIR)/$(TARGET).nacp
endif

ifneq ($(APP_TITLEID),)
	export NACPFLAGS += --titleid=$(APP_TITLEID)
endif

ifneq ($(ROMFS),)
	export NROFLAGS += --romfsdir=$(CURDIR)/$(ROMFS)
endif

.PHONY: $(BUILD) clean all

#---------------------------------------------------------------------------------
ifneq ($(strip $(GPU_ARCHIVE) $(TERRAIN_ARCHIVE) $(TEXTURE_ARCHIVE)),)
GPU_SHADERS := $(ROMFS)/shaders/$(GRAPHICS_SHADER_STEM)_vsh.dksh $(ROMFS)/shaders/$(GRAPHICS_SHADER_STEM)_fsh.dksh
export ROMFS_DEPS := $(addprefix $(CURDIR)/,$(GPU_SHADERS))
all: $(GPU_SHADERS) $(BUILD)

$(ROMFS)/shaders/$(GRAPHICS_SHADER_STEM)_vsh.dksh: $(GRAPHICS_SHADER_DIR)/$(GRAPHICS_SHADER_STEM)_vsh.glsl
	@mkdir -p $(dir $@)
	uam -s vert -o $@ $<

$(ROMFS)/shaders/$(GRAPHICS_SHADER_STEM)_fsh.dksh: $(GRAPHICS_SHADER_DIR)/$(GRAPHICS_SHADER_STEM)_fsh.glsl
	@mkdir -p $(dir $@)
	uam -s frag -o $@ $<

$(BUILD): $(GPU_SHADERS)
else
all: $(BUILD)
endif

$(BUILD):
	@[ -d $@ ] || mkdir -p $@
	@$(MAKE) --no-print-directory -C $(BUILD) -f $(CURDIR)/Makefile

#---------------------------------------------------------------------------------
clean:
	@echo clean ...
ifeq ($(strip $(APP_JSON)),)
	@rm -fr $(BUILD) $(TARGET).nro $(TARGET).nacp $(TARGET).elf
else
	@rm -fr $(BUILD) $(TARGET).nsp $(TARGET).nso $(TARGET).npdm $(TARGET).elf
endif


#---------------------------------------------------------------------------------
else
.PHONY:	all

DEPENDS	:=	$(OFILES:.o=.d)

#---------------------------------------------------------------------------------
# main targets
#---------------------------------------------------------------------------------
ifeq ($(strip $(APP_JSON)),)

all	:	$(OUTPUT).nro

ifeq ($(strip $(NO_NACP)),)
$(OUTPUT).nro	:	$(OUTPUT).elf $(OUTPUT).nacp $(ROMFS_DEPS)
$(OUTPUT).nacp : $(TOPDIR)/Makefile
else
$(OUTPUT).nro	:	$(OUTPUT).elf
endif

else

all	:	$(OUTPUT).nsp

$(OUTPUT).nsp	:	$(OUTPUT).nso $(OUTPUT).npdm

$(OUTPUT).nso	:	$(OUTPUT).elf

endif

$(OUTPUT).elf	:	$(OFILES) $(RUST_ARCHIVE) $(STD_ARCHIVE) $(NET_ARCHIVE) $(LOGIN_ARCHIVE) $(GPU_ARCHIVE) $(TERRAIN_ARCHIVE) $(TEXTURE_ARCHIVE)

$(OFILES_SRC)	: $(HFILES_BIN)

#---------------------------------------------------------------------------------
# you need a rule like this for each extension you use as binary data
#---------------------------------------------------------------------------------
%.bin.o	%_bin.h :	%.bin
#---------------------------------------------------------------------------------
	@echo $(notdir $<)
	@$(bin2o)

-include $(DEPENDS)

#---------------------------------------------------------------------------------------
endif
#---------------------------------------------------------------------------------------
