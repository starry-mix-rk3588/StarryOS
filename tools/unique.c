#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>
#include <string.h>
#include <stdlib.h>

typedef unsigned long ulong;

struct unique_t
{
    ulong unique_len;
    char *unique;
};

#define DRM_IOCTL_GET_UNIQUE 0xc0106401

int main()
{
    // 打印结构体大小
    printf("sizeof: %lu\n", sizeof(struct unique_t));

    // 打开 /dev/dri/card0
    int fd = open("/dev/dri/card1", O_RDWR);
    if (fd == -1)
    {
        perror("Failed to open /dev/dri/card0");
        return -1;
    }

    struct unique_t unique;

    unique.unique = malloc(512);
    printf("drm version name ptr: %p\n", unique.unique);

    int ret = ioctl(fd, DRM_IOCTL_GET_UNIQUE, &unique);
    if (ret == -1)
    {
        perror("Failed to call ioctl");
        close(fd);
        return -1;
    }

    printf("name: (len: %d)\n", unique.unique_len);
    for (int i = 0; i < unique.unique_len; i++)
        putchar(unique.unique[i]);
    printf("\n");

    free(unique.unique);

    // 关闭设备文件
    close(fd);
    return 0;
}