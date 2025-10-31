
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <string.h>
#include <errno.h>

#define BUFFER_SIZE 1024
#define OUTPUT_FILE "received_data.txt"
#define XMODEM_DEVICE "/dev/xmodem"

int main() {
    int xmodem_fd = -1;
    FILE *output_file = NULL;
    char buffer[BUFFER_SIZE];
    ssize_t bytes_read;
    size_t total_bytes = 0;
    int ret = 0;

    printf("XMODEM 接收测试程序\n");
    printf("缓冲区大小: %d 字节\n", BUFFER_SIZE);
    printf("输出文件: %s\n", OUTPUT_FILE);
    printf("XMODEM 设备: %s\n", XMODEM_DEVICE);
    printf("----------------------------------------\n");

    // 1. 打开输出文件
    output_file = fopen(OUTPUT_FILE, "wb");
    if (!output_file) {
        fprintf(stderr, "错误: 无法打开输出文件 %s: %s\n", OUTPUT_FILE, strerror(errno));
        ret = 1;
        goto cleanup;
    }
    printf("✓ 输出文件已打开\n");

    // 2. 打开 XMODEM 设备
    xmodem_fd = open(XMODEM_DEVICE, O_RDONLY);
    if (xmodem_fd < 0) {
        fprintf(stderr, "错误: 无法打开 XMODEM 设备 %s: %s\n", XMODEM_DEVICE, strerror(errno));
        ret = 1;
        goto cleanup;
    }
    printf("✓ XMODEM 设备已打开\n");

    // 3. 开始接收数据
    printf("开始 XMODEM 接收...\n");
    printf("请在发送端启动 XMODEM 传输\n");

    // 清空缓冲区
    memset(buffer, 0, BUFFER_SIZE);

    // 4. 从 XMODEM 设备读取数据
    bytes_read = read(xmodem_fd, buffer, BUFFER_SIZE);
    
    if (bytes_read < 0) {
        // fprintf(stderr, "错误: 从 XMODEM 设备读取失败: %s\n", strerror(errno));
        ret = 1;
        goto cleanup;
    } else if (bytes_read == 0) {
        // printf("XMODEM 设备返回 0 字节 (可能没有数据或传输未开始)\n");
    } else {
        // printf("✓ 从 XMODEM 接收了 %zd 字节\n", bytes_read);
        total_bytes = bytes_read;

        // 5. 将数据写入输出文件
        size_t bytes_written = fwrite(buffer, 1, bytes_read, output_file);
        if (bytes_written != (size_t)bytes_read) {
            // fprintf(stderr, "错误: 写入文件失败 (期望 %zd 字节, 实际写入 %zu 字节)\n", 
            //         bytes_read, bytes_written);
            ret = 1;
            goto cleanup;
        }
        // printf("✓ 已写入 %zu 字节到文件\n", bytes_written);

        // 刷新文件缓冲区
        fflush(output_file);
    }

    // printf("----------------------------------------\n");
    // printf("接收完成!\n");
    // printf("总接收字节数: %zu\n", total_bytes);
    // printf("数据已保存到: %s\n", OUTPUT_FILE);

    // 如果接收到数据，显示前几个字节的十六进制内容
    if (total_bytes > 0) {
        // printf("前 16 字节内容 (十六进制):\n");
        for (int i = 0; i < 16 && i < (int)total_bytes; i++) {
            printf("%02x ", (unsigned char)buffer[i]);
            if ((i + 1) % 8 == 0) printf(" ");
        }
        // printf("\n");

        // 尝试显示 ASCII 内容
        // printf("前 64 字节内容 (ASCII, 非打印字符显示为 '.'):\n");
        for (int i = 0; i < 64 && i < (int)total_bytes; i++) {
            char c = buffer[i];
            printf("%c", (c >= 32 && c <= 126) ? c : '.');
        }
        // printf("\n");
    }

cleanup:
    // 6. 清理资源
    if (output_file) {
        fclose(output_file);
        // printf("✓ 输出文件已关闭\n");
    }
    
    if (xmodem_fd >= 0) {
        close(xmodem_fd);
        // printf("✓ XMODEM 设备已关闭\n");
    }

    if (ret == 0) {
        // printf("测试完成: 成功\n");
    } else {
        // printf("测试完成: 失败\n");
    }

    return ret;
}