# Starry OS

## Quick Start

### 1. Install System Dependencies

This step may vary depending on your operating system. Here is an example based on Debian:

```bash
$ sudo apt update
$ sudo apt install -y build-essential cmake clang qemu-system
```

**Note:** Running on LoongArch64 requires QEMU 10. If the QEMU version in your Linux distribution is too old (e.g. Ubuntu), consider installing QEMU from [source](https://www.qemu.org/download/).

### 2. Install Musl Toolchain

1. Download files from https://github.com/arceos-org/setup-musl/releases/tag/prebuilt
2. Extract to some path, for example `/opt/riscv64-linux-musl-cross`
3. Add bin folder to `PATH`, for example:
   ```bash
   $ export PATH=/opt/riscv64-linux-musl-cross/bin:$PATH
   ```

### 3. Clone repo

```bash
$ git clone --recursive https://github.com/Starry-OS/StarryOS.git
$ cd StarryOS
```

Or if you have already cloned it with out `--recursive` option:

```bash
$ cd StarryOS
$ git submodule update --init --recursive
```

### 4. Setup Rust toolchain

```bash
# Install rustup from https://rustup.rs or using your system package manager

# Make sure that you don't have `RUSTUP_DIST_SERVER` set
$ export RUSTUP_DIST_SERVER=

# Automatically download components via rustup
$ cd StarryOS
$ rustup target list --installed
```

### 5. Build

```bash
# Default target: riscv64
$ make build
# Explicit target
$ make ARCH=riscv64 build
$ make ARCH=loongarch64 build
```

This should also download required binary dependencies like [cargo-binutils](https://github.com/rust-embedded/cargo-binutils).

### 6. Prepare rootfs

```bash
$ make img
$ make img ARCH=riscv64
$ make img ARCH=loongarch64
```

This will download rootfs image from [GitHub Releases](https://github.com/Starry-OS/StarryOS/releases) and setup the disk file for running on QEMU.

### 7. Run on QEMU

```bash
$ make run ARCH=riscv64
$ make run ARCH=loongarch64

# Shortcut:
$ make rv
$ make la
```

Note:
1. You don't have to rerun the build step before running. `run` will automatically rebuild it.
2. The disk file will **not** be reset between each run. As a result, if you want to switch to another architecture, you must run `make img` with the new architecture before running `make run`.

## What next?

You can check out the [GUI guide](./docs/gui.md) to set up a graphical environment, or explore other documentation in this folder.

## Other Options

TODO

See [Makefile](./Makefile)


# Qemu 启动

``` bash
git clone -b ajax --recurse-submodules https://github.com/starry-mix-rk3588/StarryOS.git
cd module-local/lwext4_rust
make musl-generic -C c/lwext4 ARCH=aarch64
cd starry-mix
make ARCH=aarch64 LOG=debug run
```

# OrangePi 5 Plus

## eMMC（TODO）

将镜像烧写到 eMMC，或者在 SD 卡中创建两个分区，参考 [eMMC 烧写](https://github.com/starry-mix-rk3588/axplat-opi5p?tab=readme-ov-file#%E7%83%A7%E5%86%99-emmc)

## SDMMC

``` bash
git clone -b ajax --recurse-submodules https://github.com/starry-mix-rk3588/starry-mix.git
cd module-local/lwext4_rust/c
git submodule init && git submodule update
make musl-generic -C c/lwext4 ARCH=aarch64
cd ../../../ # 回到根目录
cd module-local/axplat-opi5p/tools/orangepi5
sudo bash ./make_flash.sh partition rootfs=disk.img # 仅第一次需要, 和写入文件系统的时候需要
make ARCH=aarch64 LOG=error opi5p # 制作 uimg 镜像文件
# 进入Maskrom 模式
make ARCH=aarch64 LOG=error flash # 制作并烧写 boot 镜像
```

``` bash
U-Boot 2025.04 (Apr 02 2024 - 10:58:58 +0000)

Model: Xunlong Orange Pi 5 Plus
SoC:   RK3588
DRAM:  4 GiB
Core:  335 devices, 30 uclasses, devicetree: separate
MMC:   mmc@fe2c0000: 1, mmc@fe2e0000: 0
Loading Environment from nowhere... OK
In:    serial@feb50000
Out:   serial@feb50000
Err:   serial@feb50000
Model: Xunlong Orange Pi 5 Plus
SoC:   RK3588
Net:   No ethernet found.
Hit any key to stop autoboot:  0 
Scanning for bootflows in all bootdevs
Seq  Method       State   Uclass    Part  Name                      Filename
---  -----------  ------  --------  ----  ------------------------  ----------------
Scanning global bootmeth 'efi_mgr':
Cannot persist EFI variables without system partition
  0  efi_mgr      ready   (none)       0  <NULL>                    
** Booting bootflow '<NULL>' with efi_mgr
Loading Boot0000 'mmc 1' failed
Loading Boot0001 'mmc 0' failed
EFI boot manager: Cannot load any image
Boot failed (err=-14)
Scanning bootdev 'mmc@fe2c0000.bootdev':
Scanning bootdev 'mmc@fe2e0000.bootdev':
  1  script       ready   mmc          1  mmc@fe2e0000.bootdev.part /boot.scr
** Booting bootflow 'mmc@fe2e0000.bootdev.part_1' with script
31330496 bytes read in 436 ms (68.5 MiB/s)
## Starting application at 0x00400000 ...
0Hi
   init_early on RK3588

 ____  _                          __  __ _         ⋆˙⟡
/ ___|| |_ __ _ _ __ _ __ _   _  |  \/  (_)_  __  ⋆⭒˚.⋆
\___ \| __/ _` | '__| '__| | | | | |\/| | \ \/ /
 ___) | || (_| | |  | |  | |_| | | |  | | |>  <
|____/ \__\__,_|_|  |_|   \__, | |_|  |_|_/_/\_\
                          |___/

arch = aarch64
platform = aarch64-opi5p
target = aarch64-unknown-none-softfloat
build_mode = release
log_level = error
backtrace = true
smp = 1

Boot at 1970-01-01 00:00:07.050667292 UTC

[  7.052430 0 axbacktrace::dwarf:77] Failed to initialize addr2line context: Hit the end of input before it was expected
/ # 
/ # 
/ # ls
bin         linuxrc     musl        sbin        tmp
dev         lost+found  proc        sys         usr
/ # cd bin
/bin # ls
arch           date           getopt         ln             mv             rmdir          tar
ash            dd             grep           login          netstat        rpm            touch
base32         df             gunzip         ls             nice           run-parts      true
base64         dmesg          gzip           lsattr         pidof          scriptreplay   umount
busybox        dnsdomainname  hostname       lzop           ping           sed            uname
cat            dumpkmap       hush           makemime       ping6          setarch        usleep
chattr         echo           ionice         mkdir          pipe_progress  setpriv        vi
chgrp          ed             iostat         mknod          printenv       setserial      watch
chmod          egrep          ipcalc         mktemp         ps             sh             zcat
chown          false          kbd_mode       more           pwd            sleep
conspy         fatattr        kill           mount          reformime      stat
cp             fdflush        link           mountpoint     resume         stty
cpio           fgrep          linux32        mpstat         rev            su
cttyhack       fsync          linux64        mt             rm             sync
/bin #
```
