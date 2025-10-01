#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>
#include <string.h>
#include <stdlib.h>

typedef unsigned long ulong;

struct drm_version
{
    int major;
    int minor;
    int patchlevel;
    ulong name_len;
    char *name;
    ulong date_len;
    char *date;
    ulong desc_len;
    char *desc;
};

#define DRM_IOCTL_VERSION 0xc0406400

int main()
{
    // 打印结构体大小
    printf("sizeof: %lu\n", sizeof(struct drm_version));

    // 打开 /dev/dri/card0
    int fd = open("/dev/dri/card1", O_RDWR);
    if (fd == -1)
    {
        perror("Failed to open /dev/dri/card0");
        return -1;
    }

    // 初始化 drm_version 结构体
    struct drm_version drm_version;

    drm_version.name = malloc(64);
    drm_version.date = malloc(64);
    drm_version.desc = malloc(64);

    printf("drm version name ptr: %p\n", drm_version.name);

    // 传递结构体并执行 ioctl 调用
    int ret = ioctl(fd, DRM_IOCTL_VERSION, &drm_version);
    if (ret == -1)
    {
        perror("Failed to call ioctl");
        close(fd);
        return -1;
    }

    printf("name: (len: %d)\n", drm_version.name_len);
    for (int i = 0; i < drm_version.name_len; i++)
        putchar(drm_version.name[i]);
    printf("\n");

    printf("date: (len: %d)\n", drm_version.date_len);
    for (int i = 0; i < drm_version.date_len; i++)
        printf("%X,", drm_version.date[i]);
    printf("\n");

    printf("desc: (len: %d)\n", drm_version.desc_len);
    for (int i = 0; i < drm_version.desc_len; i++)
        putchar(drm_version.desc[i]);
    printf("\n");

    printf("major: %d, minor: %d, patchlevel: %d\n", drm_version.major, drm_version.minor,
           drm_version.patchlevel);

    free(drm_version.name);
    free(drm_version.desc);
    free(drm_version.date);

    // 关闭设备文件
    close(fd);
    return 0;
}
