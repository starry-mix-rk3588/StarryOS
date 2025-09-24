# Build Options
export ARCH := riscv64
export LOG := warn
export BACKTRACE := y
export MEMTRACK := n

# QEMU Options
export BLK := y
export NET := y
export MEM := 1G
export ICOUNT := n

# Generated Options
export A := $(PWD)
export NO_AXSTD := y
export AX_LIB := axfeat
export APP_FEATURES := qemu

TARGET_DIR := $(PWD)/target/aarch64-unknown-none-softfloat/release/starry
TOOL_PATH = $(PWD)/crates/axplat-opi5p/tools/orangepi5

ifeq ($(MEMTRACK), y)
	APP_FEATURES += starry-api/memtrack
endif

IMG_URL = https://github.com/Starry-OS/rootfs/releases/download/20250917
IMG = rootfs-$(ARCH).img

img:
	@if [ ! -f $(IMG) ]; then \
		echo "Image not found, downloading..."; \
		curl -f -L $(IMG_URL)/$(IMG).xz -O; \
		xz -d $(IMG).xz; \
	fi
	@cp $(IMG) arceos/disk.img

defconfig justrun clean:
	@make -C arceos $@

build run debug disasm: defconfig
	@make -C arceos $@

# Aliases
rv:
	$(MAKE) ARCH=riscv64 run

la:
	$(MAKE) ARCH=loongarch64 run

vf2:
	$(MAKE) ARCH=riscv64 APP_FEATURES=vf2 MYPLAT=axplat-riscv64-visionfive2 BUS=dummy build

2k1000la:
	$(MAKE) ARCH=loongarch64 APP_FEATURES=2k1000la MYPLAT=axplat-loongarch64-2k1000la BUS=dummy build

opi5p:
	$(MAKE) ARCH=aarch64 APP_FEATURES=opi5p MYPLAT=axplat-aarch64-opi5p BUS=dummy MODE=release UIMAGE=y build
	rust-objdump -d --print-imm-hex $(TARGET_DIR) > $(TARGET_DIR)_opi5p.disasm

upload: 
	bash $(TOOL_PATH)/upload_flash.sh $(TARGET_DIR).img

flash:
	sudo bash $(TOOL_PATH)/make_flash.sh

.PHONY: build run justrun debug disasm clean
