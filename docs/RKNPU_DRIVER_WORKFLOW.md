# RK3588 RKNPU 驱动工作流程详解

> 本文档详细说明 RK3588 NPU 内核驱动的完整工作流程,包括上电、时钟配置、内存管理、任务提交和作业执行等各个环节。

## 目录

1. [驱动架构概览](#1-驱动架构概览)
2. [驱动初始化与上电](#2-驱动初始化与上电)
3. [时钟和电源管理](#3-时钟和电源管理)
4. [内存分配与管理](#4-内存分配与管理)
5. [任务(Task)的创建与提交](#5-任务task的创建与提交)
6. [作业(Job)的生成与调度](#6-作业job的生成与调度)
7. [硬件执行与中断处理](#7-硬件执行与中断处理)
8. [用户态接口与交互](#8-用户态接口与交互)
9. [完整执行流程示例](#9-完整执行流程示例)

---

## 1. 驱动架构概览

### 1.1 代码目录结构

RKNPU 驱动代码分布在三个主要位置:

```
StarryOS-qemu/
├── crates/
│   ├── rknpu/                    # 内核驱动(C 语言)
│   │   ├── rknpu_drv.c          # 驱动主文件
│   │   ├── rknpu_job.c          # 作业调度
│   │   ├── rknpu_mem.c          # 内存管理
│   │   ├── rknpu_reset.c        # 复位控制
│   │   └── include/
│   │       ├── rknpu_drv.h      # 驱动头文件
│   │       ├── rknpu_ioctl.h    # IOCTL 定义
│   │       └── rknpu_job.h      # 作业结构
│   │
│   └── rknpu2-rslab/            # 用户态库(Rust)
│       ├── rknpu2/              # 高层应用示例
│       │   └── src/
│       │       ├── main.rs      # 测试程序入口
│       │       └── matmul.rs    # 矩阵乘法示例
│       │
│       └── rk3588-rs/           # 硬件抽象层
│           └── src/
│               ├── hw.rs        # 硬件寄存器定义
│               ├── ioctl.rs     # IOCTL 封装
│               ├── interface.rs # 设备接口
│               ├── cna.rs       # CNA 单元
│               ├── dpu.rs       # DPU 单元
│               └── matmul.rs    # 矩阵乘法算法
```

### 1.2 核心数据结构

#### 内核态核心结构

```c
// 位置: crates/rknpu/include/rknpu_drv.h

// NPU 设备结构
struct rknpu_device {
    void __iomem *base[RKNPU_MAX_CORES];      // 各核心寄存器基地址
    struct device *dev;                        // Linux 设备
    const struct rknpu_config *config;         // 芯片配置
    
    // 电源与时钟
    struct clk_bulk_data *clks;                // 时钟列表
    int num_clks;                              // 时钟数量
    struct regulator *vdd;                     // 电源调节器
    atomic_t power_refcount;                   // 电源引用计数
    
    // 作业管理
    struct rknpu_subcore_data subcore_datas[RKNPU_MAX_CORES];
    spinlock_t irq_lock;                       // 中断锁
    
    // 内存管理
    bool iommu_en;                             // IOMMU 使能
    struct rk_dma_heap *heap;                  // DMA 堆
};

// 子核心数据(每个 NPU 核心一个)
struct rknpu_subcore_data {
    struct list_head todo_list;                // 待处理作业队列
    wait_queue_head_t job_done_wq;            // 作业完成等待队列
    struct rknpu_job *job;                     // 当前执行的作业
    int64_t task_num;                          // 任务数量
};
```

### 1.3 硬件架构

RK3588 NPU 采用三核心架构:

```
┌─────────────────────────────────────────┐
│         RK3588 NPU (6 TOPS)             │
├─────────────────────────────────────────┤
│  NPU0 (2T)  │  NPU1 (2T)  │  NPU2 (2T) │
├─────────────┼─────────────┼─────────────┤
│     CNA     │     CNA     │     CNA     │  卷积神经加速器
│     DPU     │     DPU     │     DPU     │  数据处理单元
│    CORE     │    CORE     │    CORE     │  核心控制
│     PC      │     PC      │     PC      │  程序计数器
└─────────────┴─────────────┴─────────────┘
        │             │             │
        └─────────────┴─────────────┘
                   │
            ┌──────▼──────┐
            │   AXI BUS   │
            └──────┬──────┘
                   │
        ┌──────────┴──────────┐
        │                     │
    ┌───▼────┐          ┌────▼────┐
    │  DDR   │          │  IOMMU  │
    └────────┘          └─────────┘
```

**核心组件说明:**

- **CNA (Convolution Neural Accelerator)**: 卷积神经网络加速器,处理卷积运算
- **DPU (Data Processing Unit)**: 数据处理单元,处理激活函数、池化等
- **CORE**: 核心控制模块,协调各单元工作
- **PC (Program Counter)**: 程序计数器模式,支持批量任务提交

### 1.4 工作模式

RKNPU 支持两种工作模式:

#### PC 模式 (Program Counter Mode)
- 批量任务提交
- 硬件自动遍历任务列表
- 高吞吐量
- **本驱动使用的模式**

#### Slave 模式
- 单任务提交
- 软件控制每个任务
- 灵活性高

---

## 2. 驱动初始化与上电

### 2.1 驱动注册与探测

驱动初始化从 `rknpu_probe` 函数开始:

```c
// 位置: crates/rknpu/rknpu_drv.c:1169

static int rknpu_probe(struct platform_device *pdev)
{
    struct rknpu_device *rknpu_dev = NULL;
    struct device *dev = &pdev->dev;
    const struct rknpu_config *config = NULL;
    int ret = -EINVAL;

    // 1. 检查设备树节点
    if (!pdev->dev.of_node) {
        LOG_DEV_ERROR(dev, "rknpu device-tree data is missing!\n");
        return -ENODEV;
    }

    // 2. 匹配设备树配置
    match = of_match_device(rknpu_of_match, dev);
    if (!match) {
        LOG_DEV_ERROR(dev, "rknpu device-tree entry is missing!\n");
        return -ENODEV;
    }

    // 3. 分配设备结构
    rknpu_dev = devm_kzalloc(dev, sizeof(*rknpu_dev), GFP_KERNEL);
    if (!rknpu_dev) {
        LOG_DEV_ERROR(dev, "failed to allocate rknpu device!\n");
        return -ENOMEM;
    }

    // 4. 获取芯片配置
    config = of_device_get_match_data(dev);
    rknpu_dev->config = config;  // rk3588_rknpu_config
    rknpu_dev->dev = dev;

    // ... 继续初始化
}
```

**RK3588 配置定义:**

```c
// 位置: crates/rknpu/rknpu_drv.c:89

static const struct rknpu_config rk3588_rknpu_config = {
    .dma_mask = DMA_BIT_MASK(40),              // 40位地址空间
    .pc_data_amount_scale = 2,                 // PC数据缩放
    .pc_task_number_bits = 12,                 // 任务编号位数
    .pc_task_number_mask = 0xfff,              // 任务编号掩码
    .pc_task_status_offset = 0x3c,             // 任务状态偏移
    .irqs = rk3588_npu_irqs,                   // 三个中断
    .resets = rk3588_npu_resets,               // 三组复位
    .num_irqs = 3,                             // NPU0, NPU1, NPU2
    .num_resets = 3,
    .max_submit_number = (1 << 12) - 1,        // 最大4095个任务
    .core_mask = 0x7,                          // 三核心(111b)
};
```

### 2.2 硬件资源映射

```c
// 位置: crates/rknpu/rknpu_drv.c:1239-1258

// 映射寄存器地址
for (i = 0; i < config->num_irqs; i++) {
    res = platform_get_resource(pdev, IORESOURCE_MEM, i);
    if (!res) {
        LOG_DEV_ERROR(dev, "failed to get memory resource for rknpu\n");
        return -ENXIO;
    }

    // 映射各核心寄存器基地址
    rknpu_dev->base[i] = devm_ioremap_resource(dev, res);
    if (IS_ERR(rknpu_dev->base[i])) {
        LOG_DEV_ERROR(dev, "failed to remap register for rknpu\n");
        return PTR_ERR(rknpu_dev->base[i]);
    }
}
```

**寄存器地址 (来自设备树):**
- NPU0: 0xfdb0_0000
- NPU1: 0xfdb1_0000  
- NPU2: 0xfdb2_0000

### 2.3 中断注册

```c
// 位置: crates/rknpu/rknpu_drv.c:923-947

static int rknpu_register_irq(struct platform_device *pdev,
                              struct rknpu_device *rknpu_dev)
{
    const struct rknpu_config *config = rknpu_dev->config;
    
    // 注册三个核心的中断
    for (i = 0; i < config->num_irqs; i++) {
        irq = platform_get_irq_byname(pdev, config->irqs[i].name);
        if (irq < 0) {
            LOG_DEV_ERROR(dev, "no npu %s in dts\n",
                         config->irqs[i].name);
            return irq;
        }

        // 注册中断处理函数
        ret = devm_request_irq(dev, irq,
                              config->irqs[i].irq_hdl,  // rknpu_core0/1/2_irq_handler
                              IRQF_SHARED, dev_name(dev),
                              rknpu_dev);
        if (ret < 0) {
            LOG_DEV_ERROR(dev, "request %s failed: %d\n",
                         config->irqs[i].name, ret);
            return ret;
        }
    }
    return 0;
}
```

**中断处理函数:**
```c
// 位置: crates/rknpu/rknpu_job.c:771-779

irqreturn_t rknpu_core0_irq_handler(int irq, void *data)
{
    return rknpu_irq_handler(irq, data, 0);  // 核心0
}

irqreturn_t rknpu_core1_irq_handler(int irq, void *data)
{
    return rknpu_irq_handler(irq, data, 1);  // 核心1
}

irqreturn_t rknpu_core2_irq_handler(int irq, void *data)
{
    return rknpu_irq_handler(irq, data, 2);  // 核心2
}
```

### 2.4 初始化作业队列

```c
// 位置: crates/rknpu/rknpu_drv.c:1228-1234

for (i = 0; i < config->num_irqs; i++) {
    // 初始化每个核心的作业队列
    INIT_LIST_HEAD(&rknpu_dev->subcore_datas[i].todo_list);
    
    // 初始化等待队列
    init_waitqueue_head(&rknpu_dev->subcore_datas[i].job_done_wq);
    
    // 初始化任务计数
    rknpu_dev->subcore_datas[i].task_num = 0;
}
```

---

## 3. 时钟和电源管理

### 3.1 时钟初始化

```c
// 位置: crates/rknpu/rknpu_drv.c:1207-1213

// 获取所有时钟
rknpu_dev->num_clks = devm_clk_bulk_get_all(dev, &rknpu_dev->clks);
if (rknpu_dev->num_clks < 1) {
    LOG_DEV_ERROR(dev, "failed to get clk source for rknpu\n");
    return -ENODEV;
}
```

**RK3588 NPU 时钟列表 (来自设备树):**
- `clk_npu`: NPU 工作时钟
- `aclk_npu`: AXI 总线时钟
- `hclk_npu`: AHB 总线时钟
- `pclk_npu`: APB 总线时钟

### 3.2 电源调节器获取

```c
// 位置: crates/rknpu/rknpu_drv.c:1217-1233

// 获取 VDD 调节器
rknpu_dev->vdd = devm_regulator_get_optional(dev, "rknpu");
if (IS_ERR(rknpu_dev->vdd)) {
    if (PTR_ERR(rknpu_dev->vdd) != -ENODEV) {
        ret = PTR_ERR(rknpu_dev->vdd);
        LOG_DEV_ERROR(dev, "failed to get vdd regulator for rknpu: %d\n", ret);
        return ret;
    }
    rknpu_dev->vdd = NULL;
}

// 获取 MEM 调节器
rknpu_dev->mem = devm_regulator_get_optional(dev, "mem");
if (IS_ERR(rknpu_dev->mem)) {
    if (PTR_ERR(rknpu_dev->mem) != -ENODEV) {
        ret = PTR_ERR(rknpu_dev->mem);
        LOG_DEV_ERROR(dev, "failed to get mem regulator for rknpu: %d\n", ret);
        return ret;
    }
    rknpu_dev->mem = NULL;
}
```

### 3.3 电源域初始化

```c
// 位置: crates/rknpu/rknpu_drv.c:1339-1353

// 检查是否有多个电源域
if (of_count_phandle_with_args(dev->of_node, "power-domains",
                               "#power-domain-cells") > 1) {
    // 获取各核心电源域
    virt_dev = dev_pm_domain_attach_by_name(dev, "npu0");
    if (!IS_ERR(virt_dev))
        rknpu_dev->genpd_dev_npu0 = virt_dev;
        
    virt_dev = dev_pm_domain_attach_by_name(dev, "npu1");
    if (!IS_ERR(virt_dev))
        rknpu_dev->genpd_dev_npu1 = virt_dev;
        
    virt_dev = dev_pm_domain_attach_by_name(dev, "npu2");
    if (!IS_ERR(virt_dev))
        rknpu_dev->genpd_dev_npu2 = virt_dev;
        
    rknpu_dev->multiple_domains = true;
}
```

### 3.4 上电流程

```c
// 位置: crates/rknpu/rknpu_drv.c:658-722

static int rknpu_power_on(struct rknpu_device *rknpu_dev)
{
    struct device *dev = rknpu_dev->dev;
    int ret = -EINVAL;

    // 1. 使能电压调节器
    if (rknpu_dev->vdd) {
        ret = regulator_enable(rknpu_dev->vdd);
        if (ret) {
            LOG_DEV_ERROR(dev, "failed to enable vdd reg for rknpu, ret: %d\n", ret);
            return ret;
        }
    }

    if (rknpu_dev->mem) {
        ret = regulator_enable(rknpu_dev->mem);
        if (ret) {
            LOG_DEV_ERROR(dev, "failed to enable mem reg for rknpu, ret: %d\n", ret);
            return ret;
        }
    }

    // 2. 使能所有时钟
    ret = clk_bulk_prepare_enable(rknpu_dev->num_clks, rknpu_dev->clks);
    if (ret) {
        LOG_DEV_ERROR(dev, "failed to enable clk for rknpu, ret: %d\n", ret);
        return ret;
    }

    // 3. 上电各个电源域
    if (rknpu_dev->multiple_domains) {
        if (rknpu_dev->genpd_dev_npu0) {
            ret = pm_runtime_resume_and_get(rknpu_dev->genpd_dev_npu0);
            if (ret < 0) {
                LOG_DEV_ERROR(dev, "failed to get pm runtime for npu0, ret: %d\n", ret);
                goto out;
            }
        }
        if (rknpu_dev->genpd_dev_npu1) {
            ret = pm_runtime_resume_and_get(rknpu_dev->genpd_dev_npu1);
            if (ret < 0) {
                LOG_DEV_ERROR(dev, "failed to get pm runtime for npu1, ret: %d\n", ret);
                goto out;
            }
        }
        if (rknpu_dev->genpd_dev_npu2) {
            ret = pm_runtime_resume_and_get(rknpu_dev->genpd_dev_npu2);
            if (ret < 0) {
                LOG_DEV_ERROR(dev, "failed to get pm runtime for npu2, ret: %d\n", ret);
                goto out;
            }
        }
    }
    
    // 4. 运行时电源管理
    ret = pm_runtime_get_sync(dev);
    if (ret < 0) {
        LOG_DEV_ERROR(dev, "failed to get pm runtime for rknpu, ret: %d\n", ret);
    }

out:
    return ret;
}
```

### 3.5 下电流程

```c
// 位置: crates/rknpu/rknpu_drv.c:724-771

static int rknpu_power_off(struct rknpu_device *rknpu_dev)
{
    struct device *dev = rknpu_dev->dev;

    // 1. 释放运行时电源
    pm_runtime_put_sync(dev);

    // 2. 下电各个电源域 (等待 IOMMU 禁用)
    if (rknpu_dev->multiple_domains) {
        // 等待 IOMMU 禁用以避免访问错误
        ret = readx_poll_timeout(rockchip_iommu_is_enabled, dev, val,
                                !val, NPU_MMU_DISABLED_POLL_PERIOD_US,
                                NPU_MMU_DISABLED_POLL_TIMEOUT_US);
        if (ret) {
            LOG_DEV_ERROR(dev, "iommu still enabled\n");
            pm_runtime_get_sync(dev);
            return ret;
        }
        
        if (rknpu_dev->genpd_dev_npu2)
            pm_runtime_put_sync(rknpu_dev->genpd_dev_npu2);
        if (rknpu_dev->genpd_dev_npu1)
            pm_runtime_put_sync(rknpu_dev->genpd_dev_npu1);
        if (rknpu_dev->genpd_dev_npu0)
            pm_runtime_put_sync(rknpu_dev->genpd_dev_npu0);
    }

    // 3. 禁用时钟
    clk_bulk_disable_unprepare(rknpu_dev->num_clks, rknpu_dev->clks);

    // 4. 禁用电压调节器
    if (rknpu_dev->vdd)
        regulator_disable(rknpu_dev->vdd);

    if (rknpu_dev->mem)
        regulator_disable(rknpu_dev->mem);

    return 0;
}
```

### 3.6 电源引用计数管理

```c
// 位置: crates/rknpu/rknpu_drv.c:253-268

// 获取电源 (增加引用计数)
int rknpu_power_get(struct rknpu_device *rknpu_dev)
{
    int ret = 0;
    
    mutex_lock(&rknpu_dev->power_lock);
    // 引用计数从 0 变为 1 时才真正上电
    if (atomic_inc_return(&rknpu_dev->power_refcount) == 1)
        ret = rknpu_power_on(rknpu_dev);
    mutex_unlock(&rknpu_dev->power_lock);
    
    return ret;
}

// 释放电源 (减少引用计数)
int rknpu_power_put(struct rknpu_device *rknpu_dev)
{
    int ret = 0;
    
    mutex_lock(&rknpu_dev->power_lock);
    // 引用计数变为 0 时才真正下电
    if (atomic_dec_if_positive(&rknpu_dev->power_refcount) == 0)
        ret = rknpu_power_off(rknpu_dev);
    mutex_unlock(&rknpu_dev->power_lock);
    
    return ret;
}
```

---

## 4. 内存分配与管理

### 4.1 内存对象结构

```c
// 位置: crates/rknpu/include/rknpu_mem.h (推断)

struct rknpu_mem_object {
    struct dma_buf *dmabuf;          // DMA 缓冲区
    dma_addr_t dma_addr;             // DMA 物理地址
    void *kv_addr;                   // 内核虚拟地址
    size_t size;                     // 大小
    struct sg_table *sgt;            // scatter-gather 表
    int owner;                       // 是否是所有者
    struct list_head head;           // 链表节点
};
```

### 4.2 内存分配 IOCTL

```c
// 位置: crates/rknpu/rknpu_mem.c:23-112

int rknpu_mem_create_ioctl(struct rknpu_device *rknpu_dev, unsigned long data,
                           struct file *file)
{
    struct rknpu_mem_create args;
    struct dma_buf *dmabuf;
    struct rknpu_mem_object *rknpu_obj = NULL;
    int ret = -EINVAL;

    // 1. 从用户空间复制参数
    if (unlikely(copy_from_user(&args, (struct rknpu_mem_create *)data,
                                sizeof(struct rknpu_mem_create)))) {
        LOG_ERROR("%s: copy_from_user failed\n", __func__);
        return -EFAULT;
    }

    // 2. 分配内存对象
    rknpu_obj = kzalloc(sizeof(*rknpu_obj), GFP_KERNEL);
    if (!rknpu_obj)
        return -ENOMEM;

    // 3. 分配或导入 DMA 缓冲区
    if (args.handle > 0) {
        // 导入已有的 DMA 缓冲区
        fd = args.handle;
        dmabuf = dma_buf_get(fd);
        if (IS_ERR(dmabuf)) {
            ret = PTR_ERR(dmabuf);
            goto err_free_obj;
        }
        rknpu_obj->dmabuf = dmabuf;
        rknpu_obj->owner = 0;  // 不是所有者
    } else {
        // 分配新的 DMA 缓冲区
        dmabuf = rk_dma_heap_buffer_alloc(rknpu_dev->heap, args.size,
                                         O_CLOEXEC | O_RDWR, 0x0,
                                         dev_name(rknpu_dev->dev));
        if (IS_ERR(dmabuf)) {
            LOG_ERROR("dmabuf alloc failed, args.size = %llu\n", args.size);
            ret = PTR_ERR(dmabuf);
            goto err_free_obj;
        }
        
        rknpu_obj->dmabuf = dmabuf;
        rknpu_obj->owner = 1;  // 是所有者

        // 获取文件描述符
        fd = dma_buf_fd(dmabuf, O_CLOEXEC | O_RDWR);
        if (fd < 0) {
            LOG_ERROR("dmabuf fd get failed\n");
            ret = -EFAULT;
            goto err_free_dma_buf;
        }
    }

    // 4. 附加到设备
    attachment = dma_buf_attach(dmabuf, rknpu_dev->dev);
    if (IS_ERR(attachment)) {
        LOG_ERROR("dma_buf_attach failed\n");
        ret = PTR_ERR(attachment);
        goto err_free_dma_buf;
    }

    // 5. 映射 DMA 地址
    table = dma_buf_map_attachment(attachment, DMA_BIDIRECTIONAL);
    if (IS_ERR(table)) {
        LOG_ERROR("dma_buf_map_attachment failed\n");
        dma_buf_detach(dmabuf, attachment);
        ret = PTR_ERR(table);
        goto err_free_dma_buf;
    }

    // 6. 获取物理地址
    for_each_sgtable_sg(table, sgl, i) {
        phys = sg_dma_address(sgl);
        page = sg_page(sgl);
        length = sg_dma_len(sgl);
    }

    // 7. 创建内核映射 (如果需要)
    if (args.flags & RKNPU_MEM_KERNEL_MAPPING) {
        page_count = length >> PAGE_SHIFT;
        pages = vmalloc(page_count * sizeof(struct page));
        if (!pages) {
            LOG_ERROR("alloc pages failed\n");
            ret = -ENOMEM;
            goto err_detach_dma_buf;
        }

        for (i = 0; i < page_count; i++)
            pages[i] = &page[i];

        rknpu_obj->kv_addr = vmap(pages, page_count, VM_MAP, PAGE_KERNEL);
        if (!rknpu_obj->kv_addr) {
            LOG_ERROR("vmap pages addr failed\n");
            ret = -ENOMEM;
            goto err_free_pages;
        }
        vfree(pages);
    }

    // 8. 填充返回参数
    rknpu_obj->size = PAGE_ALIGN(args.size);
    rknpu_obj->dma_addr = phys;
    rknpu_obj->sgt = table;

    args.size = rknpu_obj->size;
    args.obj_addr = (__u64)(uintptr_t)rknpu_obj;
    args.dma_addr = rknpu_obj->dma_addr;
    args.handle = fd;

    // 9. 返回给用户空间
    if (unlikely(copy_to_user((struct rknpu_mem_create *)data, &args,
                              sizeof(struct rknpu_mem_create)))) {
        LOG_ERROR("%s: copy_to_user failed\n", __func__);
        ret = -EFAULT;
        goto err_unmap_kv_addr;
    }

    // 10. 清理临时资源
    dma_buf_unmap_attachment(attachment, table, DMA_BIDIRECTIONAL);
    dma_buf_detach(dmabuf, attachment);

    // 11. 添加到会话列表
    spin_lock(&rknpu_dev->lock);
    session = file->private_data;
    list_add_tail(&rknpu_obj->head, &session->list);
    spin_unlock(&rknpu_dev->lock);

    return 0;
    
    // ... 错误处理代码 ...
}
```

### 4.3 内存同步

```c
// 位置: crates/rknpu/rknpu_mem.c:176-211

int rknpu_mem_sync_ioctl(struct rknpu_device *rknpu_dev, unsigned long data)
{
    struct rknpu_mem_object *rknpu_obj = NULL;
    struct rknpu_mem_sync args;
    struct dma_buf *dmabuf;

    // 1. 获取参数
    if (unlikely(copy_from_user(&args, (struct rknpu_mem_sync *)data,
                                sizeof(struct rknpu_mem_sync)))) {
        LOG_ERROR("%s: copy_from_user failed\n", __func__);
        return -EFAULT;
    }

    rknpu_obj = (struct rknpu_mem_object *)(uintptr_t)args.obj_addr;
    dmabuf = rknpu_obj->dmabuf;

    // 2. 同步到设备 (CPU -> Device)
    if (args.flags & RKNPU_MEM_SYNC_TO_DEVICE) {
        dmabuf->ops->end_cpu_access_partial(dmabuf, DMA_TO_DEVICE,
                                           args.offset, args.size);
    }
    
    // 3. 同步到 CPU (Device -> CPU)
    if (args.flags & RKNPU_MEM_SYNC_FROM_DEVICE) {
        dmabuf->ops->begin_cpu_access_partial(dmabuf, DMA_FROM_DEVICE,
                                             args.offset, args.size);
    }

    return 0;
}
```

### 4.4 内存释放

```c
// 位置: crates/rknpu/rknpu_mem.c:114-174

int rknpu_mem_destroy_ioctl(struct rknpu_device *rknpu_dev, unsigned long data,
                            struct file *file)
{
    struct rknpu_mem_object *rknpu_obj, *entry, *q;
    struct rknpu_session *session = NULL;
    struct rknpu_mem_destroy args;

    // 1. 获取参数
    if (unlikely(copy_from_user(&args, (struct rknpu_mem_destroy *)data,
                                sizeof(struct rknpu_mem_destroy)))) {
        LOG_ERROR("%s: copy_from_user failed\n", __func__);
        return -EFAULT;
    }

    rknpu_obj = (struct rknpu_mem_object *)(uintptr_t)args.obj_addr;

    // 2. 从会话列表中移除
    spin_lock(&rknpu_dev->lock);
    session = file->private_data;
    list_for_each_entry_safe(entry, q, &session->list, head) {
        if (entry == rknpu_obj) {
            list_del(&entry->head);
            break;
        }
    }
    spin_unlock(&rknpu_dev->lock);

    // 3. 释放资源
    if (rknpu_obj == entry) {
        // 取消内核映射
        vunmap(rknpu_obj->kv_addr);
        rknpu_obj->kv_addr = NULL;

        // 释放 DMA 缓冲区
        if (!rknpu_obj->owner)
            dma_buf_put(rknpu_obj->dmabuf);

        // 释放对象
        kfree(rknpu_obj);
    }

    return 0;
}
```

### 4.5 用户态内存分配示例

```rust
// 位置: crates/rknpu2-rslab/rk3588-rs/src/interface.rs

impl NpuDevice {
    pub fn mem_allocate(&self, size: usize, flags: u32) -> io::Result<NpuMemory> {
        let mut create = RknpuMemCreate {
            handle: 0,                    // 0 表示分配新内存
            flags,                        // 内存标志
            size: size as u64,
            obj_addr: 0,
            dma_addr: 0,
            sram_size: 0,
        };

        // 调用 IOCTL
        unsafe {
            drm_ioctl_rknpu_mem_create(self.fd, &mut create)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, 
                    format!("mem_create ioctl failed: {}", e)))?;
        }

        Ok(NpuMemory {
            fd: self.fd,
            handle: create.handle as i32,
            obj_addr: create.obj_addr,
            dma_addr: create.dma_addr,
            size: create.size as usize,
            virt_addr: create.obj_addr as *mut u8,
            _phantom: PhantomData,
        })
    }
}
```

---

## 5. 任务(Task)的创建与提交

### 5.1 任务结构定义

```c
// 位置: crates/rknpu/include/rknpu_ioctl.h:170-186

struct rknpu_task {
    __u32 flags;                // 任务标志
    __u32 op_idx;               // 操作索引
    __u32 enable_mask;          // 使能掩码 (哪些模块启用)
    __u32 int_mask;             // 中断掩码 (等待哪些中断)
    __u32 int_clear;            // 中断清除
    __u32 int_status;           // 中断状态 (返回值)
    __u32 regcfg_amount;        // 寄存器配置数量
    __u32 regcfg_offset;        // 寄存器配置偏移
    __u64 regcmd_addr;          // 寄存器命令地址 (DMA地址)
} __packed;
```

**各字段说明:**

- **flags**: 任务标志位
- **op_idx**: 操作索引,用于标识任务
- **enable_mask**: 指示哪些硬件模块需要启用
  - `0x1`: PC (Program Counter)
  - `0x4`: CNA (Convolution Neural Accelerator)
  - `0x8`: DPU (Data Processing Unit)
  - 示例: `0xd` = `0x1 | 0x4 | 0x8` (PC + CNA + DPU)
- **int_mask**: 等待的中断类型
  - `0x300`: 等待 DPU 完成
  - `0xc`: 等待 CNA 完成
- **regcfg_amount**: 寄存器命令的数量
- **regcmd_addr**: 寄存器命令数组的 DMA 物理地址

### 5.2 用户态任务创建示例

```rust
// 位置: crates/rknpu2-rslab/rknpu2/src/matmul.rs:75-110

// 1. 分配任务内存
let tasks_mem = npu.mem_allocate(1024, RKNPU_MEM_KERNEL_MAPPING)?;

// 2. 获取任务结构指针
let tasks_ptr = tasks_mem.as_ptr();
let tasks = unsafe { &mut *(tasks_ptr as *mut RknpuTask) };

// 3. 填充任务结构
tasks.flags = 0;
tasks.op_idx = 0;
tasks.enable_mask = 0xd;           // PC + CNA + DPU
tasks.int_mask = 0x300;            // 等待 DPU 完成
tasks.int_clear = 0x1ffff;         // 清除所有中断
tasks.int_status = 0;
tasks.regcfg_amount = (npu_regs.len() as u32) - (RKNPU_PC_DATA_EXTRA_AMOUNT + 4);
tasks.regcfg_offset = 0;
tasks.regcmd_addr = regcmd_mem.dma_addr();  // 寄存器命令的DMA地址
```

### 5.3 寄存器命令生成

```rust
// 位置: crates/rknpu2-rslab/rk3588-rs/src/matmul.rs:1-100

// 寄存器命令使用 64 位打包格式
pub const fn npuop(op: u16, value: u32, reg: u16) -> u64 {
    ((op as u64 & 0xffff) << 48) |      // [63:48] 操作码
    ((value as u64 & 0xffffffff) << 16) | // [47:16] 值
    (reg as u64 & 0xffff)                 // [15:0]  寄存器偏移
}

// 示例: 配置 CNA 模块
let mut npu_regs: [u64; 112] = [0; 112];
let mut idx = 0;

// 使能 CNA 模块
npu_regs[idx] = npuop(OP_REG_CNA, 0xe, CNA_S_POINTER);
idx += 1;

// 配置卷积参数
npu_regs[idx] = npuop(OP_REG_CNA, conv_config, CNA_CONV_CON1);
idx += 1;

// 配置输入特征图地址
npu_regs[idx] = npuop(OP_REG_CNA, input_dma_addr, CNA_FEATURE_DATA_ADDR);
idx += 1;

// ... 更多寄存器配置 ...
```

**寄存器命令格式:**

```
┌────────────────┬────────────────────────────┬────────────────┐
│   操作码(16)   │       值(32)               │  寄存器(16)    │
│   [63:48]      │       [47:16]              │   [15:0]       │
└────────────────┴────────────────────────────┴────────────────┘

操作码类型:
- OP_REG_PC   (0x0101): PC 模块寄存器
- OP_REG_CNA  (0x0201): CNA 模块寄存器
- OP_REG_CORE (0x0801): CORE 模块寄存器
- OP_REG_DPU  (0x1001): DPU 模块寄存器
- OP_ENABLE   (0x0081): 使能操作
```

### 5.4 提交任务结构

```c
// 位置: crates/rknpu/include/rknpu_ioctl.h:207-227

struct rknpu_submit {
    __u32 flags;                // 提交标志 (PC模式、阻塞等)
    __u32 timeout;              // 超时时间 (ms)
    __u32 task_start;           // 起始任务索引
    __u32 task_number;          // 任务数量
    __u32 task_counter;         // 完成的任务数 (返回值)
    __s32 priority;             // 优先级
    __u64 task_obj_addr;        // 任务对象地址
    __u64 regcfg_obj_addr;      // 寄存器配置对象地址
    __u64 task_base_addr;       // 任务基地址 (可选)
    __s64 hw_elapse_time;       // 硬件执行时间 (返回值)
    __u32 core_mask;            // 核心掩码
    __s32 fence_fd;             // 栅栏文件描述符
    struct rknpu_subcore_task subcore_task[5];  // 多核任务配置
};
```

**flags 标志位:**

- `RKNPU_JOB_PC` (0x1): 使用 PC 模式
- `RKNPU_JOB_BLOCK` (0x0): 阻塞模式
- `RKNPU_JOB_NONBLOCK` (0x2): 非阻塞模式
- `RKNPU_JOB_PINGPONG` (0x4): 乒乓模式

**core_mask 核心掩码:**

- `0x1`: 使用核心 0
- `0x2`: 使用核心 1
- `0x4`: 使用核心 2
- `0x7`: 使用所有三个核心

### 5.5 用户态提交示例

```rust
// 位置: crates/rknpu2-rslab/rknpu2/src/matmul.rs:135-158

let mut submit = RknpuSubmit {
    flags: RKNPU_JOB_PC | RKNPU_JOB_BLOCK | RKNPU_JOB_PINGPONG,
    timeout: 6000,                  // 6 秒超时
    task_start: 0,                  // 从第 0 个任务开始
    task_number: 1,                 // 提交 1 个任务
    task_counter: 0,                // 输出: 完成的任务数
    priority: 0,
    task_obj_addr: tasks_mem.obj_addr(),  // 任务对象地址
    regcfg_obj_addr: 0,
    task_base_addr: 0,              // PC 模式使用 DMA 地址
    user_data: 0,
    core_mask: 1,                   // 使用核心 0
    fence_fd: -1,
    subcore_task: [
        RknpuSubcoreTask { task_start: 0, task_number: 1 },
        RknpuSubcoreTask { task_start: 1, task_number: 0 },
        RknpuSubcoreTask { task_start: 2, task_number: 0 },
        RknpuSubcoreTask { task_start: 0, task_number: 0 },
        RknpuSubcoreTask { task_start: 0, task_number: 0 },
    ],
};

// 提交任务
npu.submit(&mut submit)?;

println!("Completed {} tasks", submit.task_counter);
```

---

## 6. 作业(Job)的生成与调度

### 6.1 作业结构定义

```c
// 位置: crates/rknpu/include/rknpu_job.h

struct rknpu_job {
    struct rknpu_device *rknpu_dev;          // 设备指针
    struct rknpu_submit *args;               // 提交参数
    struct list_head head[RKNPU_MAX_CORES];  // 队列链表节点
    
    // 任务信息
    struct rknpu_task *first_task;           // 第一个任务
    struct rknpu_task *last_task;            // 最后一个任务
    
    // 时间统计
    ktime_t timestamp;                       // 创建时间戳
    ktime_t hw_commit_time;                  // 硬件提交时间
    ktime_t hw_recoder_time;                 // 硬件记录时间
    ktime_t hw_elapse_time;                  // 硬件执行时间
    
    // 中断状态
    __u32 int_mask[RKNPU_MAX_CORES];         // 中断掩码
    __u32 int_status[RKNPU_MAX_CORES];       // 中断状态
    bool irq_entry[RKNPU_MAX_CORES];         // 中断进入标志
    
    // 多核支持
    int use_core_num;                        // 使用的核心数
    atomic_t run_count;                      // 运行计数
    atomic_t interrupt_count;                // 中断计数
    atomic_t submit_count[RKNPU_MAX_CORES];  // 提交计数
    
    // 标志与同步
    __u32 flags;                             // 作业标志
    int ret;                                 // 返回值
    bool args_owner;                         // 是否拥有参数
    struct dma_fence *fence;                 // 栅栏
    struct work_struct cleanup_work;         // 清理工作
};
```

### 6.2 提交入口 (rknpu_submit)

```c
// 位置: crates/rknpu/rknpu_job.c:814-892

static int rknpu_submit(struct rknpu_device *rknpu_dev,
                       struct rknpu_submit *args)
{
    struct rknpu_job *job = NULL;
    int ret = -EINVAL;

    // 1. 参数验证
    if (args->task_number == 0) {
        LOG_ERROR("invalid rknpu task number!\n");
        return -EINVAL;
    }

    if (args->core_mask > rknpu_dev->config->core_mask) {
        LOG_ERROR("invalid rknpu core mask: %#x", args->core_mask);
        return -EINVAL;
    }

    // 2. 分配作业结构
    job = rknpu_job_alloc(rknpu_dev, args);
    if (!job) {
        LOG_ERROR("failed to allocate rknpu job!\n");
        return -ENOMEM;
    }

    // 3. 处理输入栅栏 (可选)
    if (args->flags & RKNPU_JOB_FENCE_IN) {
        struct dma_fence *in_fence;
        in_fence = sync_file_get_fence(args->fence_fd);
        if (!in_fence) {
            LOG_ERROR("invalid fence in fd, fd: %d\n", args->fence_fd);
            return -EINVAL;
        }
        
        // 等待栅栏信号
        ret = dma_fence_wait_timeout(in_fence, true, args->timeout);
        dma_fence_put(in_fence);
        if (ret < 0) {
            if (ret != -ERESTARTSYS)
                LOG_ERROR("Error (%d) waiting for fence!\n", ret);
            return ret;
        }
    }

    // 4. 创建输出栅栏 (可选)
    if (args->flags & RKNPU_JOB_FENCE_OUT) {
        ret = rknpu_fence_alloc(job);
        if (ret) {
            rknpu_job_free(job);
            return ret;
        }
        job->args->fence_fd = rknpu_fence_get_fd(job);
        args->fence_fd = job->args->fence_fd;
    }

    // 5. 调度作业
    if (args->flags & RKNPU_JOB_NONBLOCK) {
        // 非阻塞模式
        job->flags |= RKNPU_JOB_ASYNC;
        rknpu_job_timeout_clean(rknpu_dev, job->args->core_mask);
        rknpu_job_schedule(job);
        ret = job->ret;
        if (ret) {
            rknpu_job_abort(job);
            return ret;
        }
    } else {
        // 阻塞模式
        rknpu_job_schedule(job);
        if (args->flags & RKNPU_JOB_PC)
            job->ret = rknpu_job_wait(job);

        args->task_counter = job->args->task_counter;
        ret = job->ret;
        if (!ret)
            rknpu_job_cleanup(job);
        else
            rknpu_job_abort(job);
    }

    return ret;
}
```

### 6.3 作业分配 (rknpu_job_alloc)

```c
// 位置: crates/rknpu/rknpu_job.c:110-153

static inline struct rknpu_job *rknpu_job_alloc(struct rknpu_device *rknpu_dev,
                                               struct rknpu_submit *args)
{
    struct rknpu_job *job = NULL;

    // 1. 分配作业结构
    job = kzalloc(sizeof(*job), GFP_KERNEL);
    if (!job)
        return NULL;

    // 2. 初始化基本信息
    job->timestamp = ktime_get();
    job->rknpu_dev = rknpu_dev;
    
    // 3. 计算使用的核心数
    job->use_core_num = (args->core_mask & RKNPU_CORE0_MASK) +
                       ((args->core_mask & RKNPU_CORE1_MASK) >> 1) +
                       ((args->core_mask & RKNPU_CORE2_MASK) >> 2);
    
    // 4. 初始化计数器
    atomic_set(&job->run_count, job->use_core_num);
    atomic_set(&job->interrupt_count, job->use_core_num);

    // 5. 处理参数
    if (!(args->flags & RKNPU_JOB_NONBLOCK)) {
        // 阻塞模式: 直接使用用户参数
        job->args = args;
        job->args_owner = false;
        return job;
    }

    // 非阻塞模式: 复制参数
    job->args = kzalloc(sizeof(*args), GFP_KERNEL);
    if (!job->args) {
        kfree(job);
        return NULL;
    }
    *job->args = *args;
    job->args_owner = true;

    // 6. 初始化清理工作
    INIT_WORK(&job->cleanup_work, rknpu_job_cleanup_work);

    return job;
}
```

### 6.4 作业调度 (rknpu_job_schedule)

```c
// 位置: crates/rknpu/rknpu_job.c:609-648

static void rknpu_job_schedule(struct rknpu_job *job)
{
    struct rknpu_device *rknpu_dev = job->rknpu_dev;
    struct rknpu_subcore_data *subcore_data = NULL;
    int i = 0, core_index = 0;
    unsigned long flags;

    // 1. 自动选择核心 (负载均衡)
    if (job->args->core_mask == RKNPU_CORE_AUTO_MASK) {
        core_index = rknpu_schedule_core_index(rknpu_dev);
        job->args->core_mask = rknpu_core_mask(core_index);
        job->use_core_num = 1;
        atomic_set(&job->run_count, job->use_core_num);
        atomic_set(&job->interrupt_count, job->use_core_num);
    }

    // 2. 将作业加入各核心的队列
    spin_lock_irqsave(&rknpu_dev->irq_lock, flags);
    for (i = 0; i < rknpu_dev->config->num_irqs; i++) {
        if (job->args->core_mask & rknpu_core_mask(i)) {
            subcore_data = &rknpu_dev->subcore_datas[i];
            list_add_tail(&job->head[i], &subcore_data->todo_list);
            subcore_data->task_num += rknpu_get_task_number(job, i);
        }
    }
    spin_unlock_irqrestore(&rknpu_dev->irq_lock, flags);

    // 3. 尝试启动各核心的下一个作业
    for (i = 0; i < rknpu_dev->config->num_irqs; i++) {
        if (job->args->core_mask & rknpu_core_mask(i))
            rknpu_job_next(rknpu_dev, i);
    }
}
```

### 6.5 启动下一个作业 (rknpu_job_next)

```c
// 位置: crates/rknpu/rknpu_job.c:483-511

static void rknpu_job_next(struct rknpu_device *rknpu_dev, int core_index)
{
    struct rknpu_job *job = NULL;
    struct rknpu_subcore_data *subcore_data = NULL;
    unsigned long flags;

    // 1. 检查是否正在复位
    if (rknpu_dev->soft_reseting)
        return;

    subcore_data = &rknpu_dev->subcore_datas[core_index];

    spin_lock_irqsave(&rknpu_dev->irq_lock, flags);

    // 2. 检查核心是否空闲且有待处理作业
    if (subcore_data->job || list_empty(&subcore_data->todo_list)) {
        spin_unlock_irqrestore(&rknpu_dev->irq_lock, flags);
        return;
    }

    // 3. 取出队列中的第一个作业
    job = list_first_entry(&subcore_data->todo_list, struct rknpu_job,
                          head[core_index]);

    list_del_init(&job->head[core_index]);
    subcore_data->job = job;
    
    // 4. 记录硬件提交时间
    job->hw_commit_time = ktime_get();
    job->hw_recoder_time = job->hw_commit_time;
    
    spin_unlock_irqrestore(&rknpu_dev->irq_lock, flags);

    // 5. 多核同步: 等待所有核心都取出作业
    if (atomic_dec_and_test(&job->run_count)) {
        // 所有核心都准备好,提交到硬件
        rknpu_job_commit(job);
    }
}
```

### 6.6 负载均衡调度

```c
// 位置: crates/rknpu/rknpu_job.c:585-598

static int rknpu_schedule_core_index(struct rknpu_device *rknpu_dev)
{
    int core_num = rknpu_dev->config->num_irqs;
    int task_num = rknpu_dev->subcore_datas[0].task_num;
    int core_index = 0;
    int i = 0;

    // 选择任务数最少的核心
    for (i = 1; i < core_num; i++) {
        if (task_num > rknpu_dev->subcore_datas[i].task_num) {
            core_index = i;
            task_num = rknpu_dev->subcore_datas[i].task_num;
        }
    }

    return core_index;
}
```

---

## 7. 硬件执行与中断处理

### 7.1 作业提交到硬件 (rknpu_job_commit)

```c
// 位置: crates/rknpu/rknpu_job.c:465-481

static void rknpu_job_commit(struct rknpu_job *job)
{
    // 根据核心掩码,向对应核心提交作业
    switch (job->args->core_mask) {
    case RKNPU_CORE0_MASK:
        rknpu_job_subcore_commit(job, 0);
        break;
    case RKNPU_CORE1_MASK:
        rknpu_job_subcore_commit(job, 1);
        break;
    case RKNPU_CORE2_MASK:
        rknpu_job_subcore_commit(job, 2);
        break;
    case RKNPU_CORE0_MASK | RKNPU_CORE1_MASK:
        rknpu_job_subcore_commit(job, 0);
        rknpu_job_subcore_commit(job, 1);
        break;
    case RKNPU_CORE0_MASK | RKNPU_CORE1_MASK | RKNPU_CORE2_MASK:
        rknpu_job_subcore_commit(job, 0);
        rknpu_job_subcore_commit(job, 1);
        rknpu_job_subcore_commit(job, 2);
        break;
    default:
        LOG_ERROR("Unknown core mask: %d\n", job->args->core_mask);
        break;
    }
}
```

### 7.2 PC 模式硬件提交 (rknpu_job_subcore_commit_pc)

这是最核心的硬件操作函数:

```c
// 位置: crates/rknpu/rknpu_job.c:249-357

static inline int rknpu_job_subcore_commit_pc(struct rknpu_job *job,
                                              int core_index)
{
    struct rknpu_device *rknpu_dev = job->rknpu_dev;
    struct rknpu_submit *args = job->args;
    struct rknpu_mem_object *task_obj =
        (struct rknpu_mem_object *)(uintptr_t)args->task_obj_addr;
    struct rknpu_task *task_base = NULL;
    struct rknpu_task *first_task = NULL;
    struct rknpu_task *last_task = NULL;
    void __iomem *rknpu_core_base = rknpu_dev->base[core_index];
    int task_start = args->task_start;
    int task_end;
    int task_number = args->task_number;
    int task_pp_en = args->flags & RKNPU_JOB_PINGPONG ? 1 : 0;
    int pc_data_amount_scale = rknpu_dev->config->pc_data_amount_scale;
    int pc_task_number_bits = rknpu_dev->config->pc_task_number_bits;
    int i = 0;
    int submit_index = atomic_read(&job->submit_count[core_index]);
    int max_submit_number = rknpu_dev->config->max_submit_number;
    unsigned long flags;

    if (!task_obj) {
        job->ret = -EINVAL;
        return job->ret;
    }

    // 1. 多核任务分配
    if (rknpu_dev->config->num_irqs > 1) {
        for (i = 0; i < rknpu_dev->config->num_irqs; i++) {
            if (i == core_index) {
                // 使能对应核心
                REG_WRITE((0xe + 0x10000000 * i), 0x1004);
                REG_WRITE((0xe + 0x10000000 * i), 0x3004);
            }
        }

        // 根据使用的核心数调整任务范围
        switch (job->use_core_num) {
        case 1:
        case 2:
            task_start = args->subcore_task[core_index].task_start;
            task_number = args->subcore_task[core_index].task_number;
            break;
        case 3:
            task_start = args->subcore_task[core_index + 2].task_start;
            task_number = args->subcore_task[core_index + 2].task_number;
            break;
        default:
            LOG_ERROR("Unknown use core num %d\n", job->use_core_num);
            break;
        }
    }

    // 2. 计算任务范围 (支持分批提交)
    task_start = task_start + submit_index * max_submit_number;
    task_number = task_number - submit_index * max_submit_number;
    task_number = task_number > max_submit_number ? max_submit_number : task_number;
    task_end = task_start + task_number - 1;

    task_base = task_obj->kv_addr;
    first_task = &task_base[task_start];
    last_task = &task_base[task_end];

    // 3. 写入 NPU 寄存器 (关键硬件操作)
    
    // 3.1 设置寄存器命令地址
    if (rknpu_dev->config->pc_dma_ctrl) {
        spin_lock_irqsave(&rknpu_dev->irq_lock, flags);
        REG_WRITE(first_task->regcmd_addr, RKNPU_OFFSET_PC_DATA_ADDR);
        spin_unlock_irqrestore(&rknpu_dev->irq_lock, flags);
    } else {
        REG_WRITE(first_task->regcmd_addr, RKNPU_OFFSET_PC_DATA_ADDR);
    }

    // 3.2 设置数据量
    REG_WRITE((first_task->regcfg_amount + RKNPU_PC_DATA_EXTRA_AMOUNT +
               pc_data_amount_scale - 1) / pc_data_amount_scale - 1,
              RKNPU_OFFSET_PC_DATA_AMOUNT);

    // 3.3 设置中断掩码
    REG_WRITE(last_task->int_mask, RKNPU_OFFSET_INT_MASK);

    // 3.4 清除中断
    REG_WRITE(first_task->int_mask, RKNPU_OFFSET_INT_CLEAR);

    // 3.5 设置任务控制
    //     [11:0]  任务数量
    //     [31:12] 控制字 (0x6 | task_pp_en)
    REG_WRITE(((0x6 | task_pp_en) << pc_task_number_bits) | task_number,
              RKNPU_OFFSET_PC_TASK_CONTROL);

    // 3.6 设置 DMA 基地址 (任务数组的物理地址)
    REG_WRITE(args->task_base_addr, RKNPU_OFFSET_PC_DMA_BASE_ADDR);

    // 4. 保存作业信息
    job->first_task = first_task;
    job->last_task = last_task;
    job->int_mask[core_index] = last_task->int_mask;

    // 5. 启动 NPU
    REG_WRITE(0x1, RKNPU_OFFSET_PC_OP_EN);  // 启动
    REG_WRITE(0x0, RKNPU_OFFSET_PC_OP_EN);  // 复位启动信号

    return 0;
}
```

**寄存器写入顺序说明:**

```
1. PC_DATA_ADDR (0x10)     ← 寄存器命令 DMA 地址
2. PC_DATA_AMOUNT (0x14)   ← 寄存器命令数量
3. INT_MASK (0x20)         ← 中断掩码
4. INT_CLEAR (0x24)        ← 清除中断
5. PC_TASK_CONTROL (0x30)  ← 任务控制 (模式 | 任务数)
6. PC_DMA_BASE_ADDR (0x34) ← 任务数组 DMA 地址
7. PC_OP_EN (0x08)         ← 启动 NPU (1 → 0)
```

### 7.3 中断处理 (rknpu_irq_handler)

```c
// 位置: crates/rknpu/rknpu_job.c:698-752

static inline irqreturn_t rknpu_irq_handler(int irq, void *data, int core_index)
{
    struct rknpu_device *rknpu_dev = data;
    void __iomem *rknpu_core_base = rknpu_dev->base[core_index];
    struct rknpu_subcore_data *subcore_data = NULL;
    struct rknpu_job *job = NULL;
    uint32_t status = 0;
    unsigned long flags;

    subcore_data = &rknpu_dev->subcore_datas[core_index];

    spin_lock_irqsave(&rknpu_dev->irq_lock, flags);
    
    // 1. 获取当前作业
    job = subcore_data->job;
    if (!job) {
        spin_unlock_irqrestore(&rknpu_dev->irq_lock, flags);
        REG_WRITE(RKNPU_INT_CLEAR, RKNPU_OFFSET_INT_CLEAR);
        rknpu_job_next(rknpu_dev, core_index);
        return IRQ_HANDLED;
    }
    job->irq_entry[core_index] = true;
    
    spin_unlock_irqrestore(&rknpu_dev->irq_lock, flags);

    // 2. 读取中断状态
    status = REG_READ(RKNPU_OFFSET_INT_STATUS);
    job->int_status[core_index] = status;

    // 3. 验证中断状态
    if (rknpu_fuzz_status(status) != job->int_mask[core_index]) {
        LOG_ERROR("invalid irq status: %#x, raw status: %#x, "
                 "require mask: %#x, task counter: %#x\n",
                 status, REG_READ(RKNPU_OFFSET_INT_RAW_STATUS),
                 job->int_mask[core_index],
                 (REG_READ(rknpu_dev->config->pc_task_status_offset) &
                  rknpu_dev->config->pc_task_number_mask));
        REG_WRITE(RKNPU_INT_CLEAR, RKNPU_OFFSET_INT_CLEAR);
        return IRQ_HANDLED;
    }

    // 4. 清除中断
    REG_WRITE(RKNPU_INT_CLEAR, RKNPU_OFFSET_INT_CLEAR);

    // 5. 完成作业
    rknpu_job_done(job, 0, core_index);

    return IRQ_HANDLED;
}
```

### 7.4 作业完成处理 (rknpu_job_done)

```c
// 位置: crates/rknpu/rknpu_job.c:513-562

static void rknpu_job_done(struct rknpu_job *job, int ret, int core_index)
{
    struct rknpu_device *rknpu_dev = job->rknpu_dev;
    struct rknpu_subcore_data *subcore_data = NULL;
    ktime_t now;
    unsigned long flags;
    int max_submit_number = rknpu_dev->config->max_submit_number;

    // 1. 检查是否需要分批提交下一批任务
    if (atomic_inc_return(&job->submit_count[core_index]) <
        (rknpu_get_task_number(job, core_index) + max_submit_number - 1) /
            max_submit_number) {
        // 还有更多任务,继续提交
        rknpu_job_subcore_commit(job, core_index);
        return;
    }

    subcore_data = &rknpu_dev->subcore_datas[core_index];

    spin_lock_irqsave(&rknpu_dev->irq_lock, flags);
    
    // 2. 清理核心状态
    subcore_data->job = NULL;
    subcore_data->task_num -= rknpu_get_task_number(job, core_index);
    
    // 3. 统计执行时间
    now = ktime_get();
    job->hw_elapse_time = ktime_sub(now, job->hw_commit_time);
    subcore_data->timer.busy_time += ktime_sub(now, job->hw_recoder_time);
    
    spin_unlock_irqrestore(&rknpu_dev->irq_lock, flags);

    // 4. 多核同步: 等待所有核心完成
    if (atomic_dec_and_test(&job->interrupt_count)) {
        int use_core_num = job->use_core_num;

        // 5. 标记作业完成
        job->flags |= RKNPU_JOB_DONE;
        job->ret = ret;

        // 6. 触发栅栏
        if (job->fence)
            dma_fence_signal(job->fence);

        // 7. 异步清理或唤醒等待
        if (job->flags & RKNPU_JOB_ASYNC)
            schedule_work(&job->cleanup_work);

        // 8. 唤醒等待的进程
        if (use_core_num > 1)
            wake_up(&(&rknpu_dev->subcore_datas[0])->job_done_wq);
        else
            wake_up(&subcore_data->job_done_wq);
    }

    // 9. 启动下一个作业
    rknpu_job_next(rknpu_dev, core_index);
}
```

### 7.5 阻塞等待 (rknpu_job_wait)

```c
// 位置: crates/rknpu/rknpu_job.c:156-221

static inline int rknpu_job_wait(struct rknpu_job *job)
{
    struct rknpu_device *rknpu_dev = job->rknpu_dev;
    struct rknpu_submit *args = job->args;
    struct rknpu_task *last_task = NULL;
    struct rknpu_subcore_data *subcore_data = NULL;
    void __iomem *rknpu_core_base = NULL;
    int core_index = rknpu_wait_core_index(job->args->core_mask);
    int wait_count = 0;
    bool continue_wait = false;
    int ret = -EINVAL;

    subcore_data = &rknpu_dev->subcore_datas[core_index];

    // 循环等待,最多3次
    do {
        // 等待作业完成信号
        ret = wait_event_timeout(subcore_data->job_done_wq,
                                 job->flags & RKNPU_JOB_DONE ||
                                 rknpu_dev->soft_reseting,
                                 msecs_to_jiffies(args->timeout));

        if (++wait_count >= 3)
            break;

        // 超时检查
        if (ret == 0) {
            int64_t elapse_time_us = 0;
            spin_lock_irqsave(&rknpu_dev->irq_lock, flags);
            elapse_time_us = ktime_us_delta(ktime_get(), job->hw_commit_time);
            continue_wait = job->hw_commit_time == 0 ? true :
                          (elapse_time_us < args->timeout * 1000);
            spin_unlock_irqrestore(&rknpu_dev->irq_lock, flags);
            
            LOG_ERROR("job: %p, wait_count: %d, continue wait: %d, "
                     "commit elapse time: %lldus, wait time: %lldus, "
                     "timeout: %uus\n",
                     job, wait_count, continue_wait,
                     (job->hw_commit_time == 0 ? 0 : elapse_time_us),
                     ktime_us_delta(ktime_get(), job->timestamp),
                     args->timeout * 1000);
        }
    } while (ret == 0 && continue_wait);

    last_task = job->last_task;
    if (!last_task) {
        LOG_ERROR("job commit failed\n");
        return ret < 0 ? ret : -EINVAL;
    }

    // 更新任务状态
    last_task->int_status = job->int_status[core_index];

    if (ret <= 0) {
        // 超时处理
        args->task_counter = 0;
        rknpu_core_base = rknpu_dev->base[core_index];
        if (args->flags & RKNPU_JOB_PC) {
            uint32_t task_status = REG_READ(
                rknpu_dev->config->pc_task_status_offset);
            args->task_counter = (task_status &
                                 rknpu_dev->config->pc_task_number_mask);
        }

        LOG_ERROR("failed to wait job, task counter: %d, flags: %#x, "
                 "ret = %d, elapsed time: %lldus\n",
                 args->task_counter, args->flags, ret,
                 ktime_us_delta(ktime_get(), job->timestamp));

        return ret < 0 ? ret : -ETIMEDOUT;
    }

    if (!(job->flags & RKNPU_JOB_DONE))
        return -EINVAL;

    // 成功完成
    args->task_counter = args->task_number;
    args->hw_elapse_time = job->hw_elapse_time;

    return 0;
}
```

---

## 8. 用户态接口与交互

### 8.1 设备打开

```rust
// 位置: crates/rknpu2-rslab/rk3588-rs/src/interface.rs

pub struct NpuDevice {
    fd: RawFd,
}

impl NpuDevice {
    pub fn open() -> io::Result<Self> {
        // 打开 DRM 设备节点
        let path = CString::new("/dev/dri/card1")?;
        let fd = unsafe {
            libc::open(path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC)
        };

        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(NpuDevice { fd })
    }
}
```

### 8.2 IOCTL 封装

```rust
// 位置: crates/rknpu2-rslab/rk3588-rs/src/ioctl.rs

// 使用 nix 库定义 IOCTL
nix::ioctl_readwrite!(
    drm_ioctl_rknpu_submit,
    DRM_IOCTL_BASE,
    DRM_COMMAND_BASE + RKNPU_SUBMIT,
    RknpuSubmit
);

nix::ioctl_readwrite!(
    drm_ioctl_rknpu_mem_create,
    DRM_IOCTL_BASE,
    DRM_COMMAND_BASE + RKNPU_MEM_CREATE,
    RknpuMemCreate
);
```

### 8.3 内存管理封装

```rust
// 位置: crates/rknpu2-rslab/rk3588-rs/src/interface.rs:135-180

pub struct NpuMemory {
    fd: RawFd,
    handle: i32,
    obj_addr: u64,
    dma_addr: u64,
    size: usize,
    virt_addr: *mut u8,
    _phantom: PhantomData<()>,
}

impl NpuMemory {
    // 获取 DMA 地址
    pub fn dma_addr(&self) -> u64 {
        self.dma_addr
    }

    // 获取对象地址
    pub fn obj_addr(&self) -> u64 {
        self.obj_addr
    }

    // 获取指针
    pub fn as_ptr(&self) -> *mut u8 {
        self.virt_addr
    }

    // 获取可变切片
    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        unsafe {
            std::slice::from_raw_parts_mut(self.virt_addr, self.size)
        }
    }
}

// 自动释放内存
impl Drop for NpuMemory {
    fn drop(&mut self) {
        let mut destroy = RknpuMemDestroy {
            handle: self.handle as u32,
            reserved: 0,
            obj_addr: self.obj_addr,
        };

        unsafe {
            let _ = drm_ioctl_rknpu_mem_destroy(self.fd, &mut destroy);
        }
    }
}
```

### 8.4 任务提交封装

```rust
// 位置: crates/rknpu2-rslab/rk3588-rs/src/interface.rs:70-85

impl NpuDevice {
    pub fn submit(&self, submit: &mut RknpuSubmit) -> io::Result<()> {
        unsafe {
            drm_ioctl_rknpu_submit(self.fd, submit).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("submit ioctl failed: {}", e),
                )
            })?;
        }
        Ok(())
    }

    pub fn reset(&self) -> io::Result<()> {
        let mut action = RknpuActionStruct {
            flags: RknpuAction::ActReset as u32,
            value: 0,
        };

        unsafe {
            drm_ioctl_rknpu_action(self.fd, &mut action).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("reset action failed: {}", e),
                )
            })?;
        }
        Ok(())
    }
}
```

### 8.5 IOCTL 处理入口

```c
// 位置: crates/rknpu/rknpu_drv.c:497-517

static long rknpu_ioctl(struct file *file, uint32_t cmd, unsigned long arg)
{
    long ret = -EINVAL;
    struct rknpu_device *rknpu_dev = NULL;

    if (!file->private_data)
        return -EINVAL;

    rknpu_dev = ((struct rknpu_session *)file->private_data)->rknpu_dev;

    // 获取电源
    rknpu_power_get(rknpu_dev);

    // 分发 IOCTL 命令
    switch (cmd) {
    case IOCTL_RKNPU_ACTION:
        ret = rknpu_action_ioctl(rknpu_dev, arg);
        break;
    case IOCTL_RKNPU_SUBMIT:
        ret = rknpu_submit_ioctl(rknpu_dev, arg);
        break;
    case IOCTL_RKNPU_MEM_CREATE:
        ret = rknpu_mem_create_ioctl(rknpu_dev, arg, file);
        break;
    case IOCTL_RKNPU_MEM_DESTROY:
        ret = rknpu_mem_destroy_ioctl(rknpu_dev, arg, file);
        break;
    case IOCTL_RKNPU_MEM_SYNC:
        ret = rknpu_mem_sync_ioctl(rknpu_dev, arg);
        break;
    default:
        break;
    }

    // 释放电源 (延迟释放)
    rknpu_power_put_delay(rknpu_dev);

    return ret;
}
```

### 8.6 DRM 接口集成

```c
// 位置: crates/rknpu/rknpu_drv.c:551-575

// 定义 DRM IOCTL 命令
static const struct drm_ioctl_desc rknpu_ioctls[] = {
    DRM_IOCTL_DEF_DRV(RKNPU_ACTION, __rknpu_action_ioctl, DRM_RENDER_ALLOW),
    DRM_IOCTL_DEF_DRV(RKNPU_SUBMIT, __rknpu_submit_ioctl, DRM_RENDER_ALLOW),
    DRM_IOCTL_DEF_DRV(RKNPU_MEM_CREATE, __rknpu_gem_create_ioctl,
                      DRM_RENDER_ALLOW),
    DRM_IOCTL_DEF_DRV(RKNPU_MEM_MAP, __rknpu_gem_map_ioctl,
                      DRM_RENDER_ALLOW),
    DRM_IOCTL_DEF_DRV(RKNPU_MEM_DESTROY, __rknpu_gem_destroy_ioctl,
                      DRM_RENDER_ALLOW),
    DRM_IOCTL_DEF_DRV(RKNPU_MEM_SYNC, __rknpu_gem_sync_ioctl,
                      DRM_RENDER_ALLOW),
};

// 定义 DRM 驱动
static struct drm_driver rknpu_drm_driver = {
    .driver_features = DRIVER_GEM | DRIVER_RENDER,
    .ioctls = rknpu_ioctls,
    .num_ioctls = ARRAY_SIZE(rknpu_ioctls),
    .fops = &rknpu_drm_driver_fops,
    .name = DRIVER_NAME,
    .desc = DRIVER_DESC,
    .date = DRIVER_DATE,
    .major = DRIVER_MAJOR,
    .minor = DRIVER_MINOR,
    .patchlevel = DRIVER_PATCHLEVEL,
};
```

---

## 9. 完整执行流程示例

### 9.1 矩阵乘法完整流程

这里展示一个 4x36 * 36x16 矩阵乘法的完整执行流程:

```rust
// 位置: crates/rknpu2-rslab/rknpu2/src/matmul.rs

pub fn run_matmul_test() -> io::Result<()> {
    const M: usize = 4;
    const K: usize = 36;
    const N: usize = 16;

    // ========== 步骤 1: 打开 NPU 设备 ==========
    let npu = rk3588_rs::NpuDevice::open()?;
    // 内核: rknpu_open() 创建会话
    
    // ========== 步骤 2: 分配内存 ==========
    let mut regcmd_mem = npu.mem_allocate(1024, 0)?;
    let tasks_mem = npu.mem_allocate(1024, RKNPU_MEM_KERNEL_MAPPING)?;
    let mut input_mem = npu.mem_allocate(4096, 0)?;
    let mut weights_mem = npu.mem_allocate(4096, 0)?;
    let mut output_mem = npu.mem_allocate(4096, 0)?;
    // 内核: rknpu_mem_create_ioctl() 分配 DMA 缓冲区

    println!("Memory allocated: input_dma=0x{:x}, output_dma=0x{:x}",
             input_mem.dma_addr(), output_mem.dma_addr());

    // ========== 步骤 3: 复位 NPU ==========
    npu.reset()?;
    // 内核: rknpu_soft_reset() 软复位

    // ========== 步骤 4: 生成寄存器命令 ==========
    let mut npu_regs = [0u64; 112];
    let mut params = MatmulParams {
        m: M as u16,
        k: 64,                          // 对齐到 64
        n: N as u16,
        input_dma: input_mem.dma_addr() as u32,
        weights_dma: weights_mem.dma_addr() as u32,
        output_dma: output_mem.dma_addr() as u32,
        tasks: npu_regs.as_mut_ptr(),
        fp32tofp16: 0,
    };

    gen_matmul_fp16(&mut params)?;
    // 生成 NPU 寄存器配置命令

    // ========== 步骤 5: 复制寄存器命令到内存 ==========
    let regcmd_slice = regcmd_mem.as_slice_mut();
    unsafe {
        std::ptr::copy_nonoverlapping(
            npu_regs.as_ptr() as *const u8,
            regcmd_slice.as_mut_ptr(),
            std::mem::size_of_val(&npu_regs),
        );
    }

    // ========== 步骤 6: 设置任务结构 ==========
    let tasks_ptr = tasks_mem.as_ptr();
    let tasks = unsafe { &mut *(tasks_ptr as *mut RknpuTask) };
    
    tasks.flags = 0;
    tasks.op_idx = 0;
    tasks.enable_mask = 0xd;        // PC + CNA + DPU
    tasks.int_mask = 0x300;         // 等待 DPU 完成
    tasks.int_clear = 0x1ffff;
    tasks.int_status = 0;
    tasks.regcfg_amount = (npu_regs.len() as u32) - 
                         (RKNPU_PC_DATA_EXTRA_AMOUNT + 4);
    tasks.regcfg_offset = 0;
    tasks.regcmd_addr = regcmd_mem.dma_addr();  // DMA 地址

    // ========== 步骤 7: 准备输入数据 (FP16) ==========
    let weights_ptr = weights_mem.as_ptr();
    let weights_fp16 = unsafe { 
        std::slice::from_raw_parts_mut(weights_ptr as *mut f16, 64 * N) 
    };
    
    for n in 1..=N {
        for k in 1..=K {
            let idx = weight_fp16(64, n as i32, k as i32) as usize;
            let value = MATRIX_B[(n - 1) * K + (k - 1)];
            weights_fp16[idx] = f16::from_f32(value);
        }
    }

    let input_ptr = input_mem.as_ptr();
    let input_fp16 = unsafe { 
        std::slice::from_raw_parts_mut(input_ptr as *mut f16, M * 64) 
    };
    
    for m in 1..=M {
        for k in 1..=K {
            let idx = feature_data(64, 4, 1, 8, k as i32, m as i32, 1) as usize;
            let value = MATRIX_A[(m - 1) * K + (k - 1)];
            input_fp16[idx] = f16::from_f32(value);
        }
    }

    // ========== 步骤 8: 提交任务 ==========
    let mut submit = RknpuSubmit {
        flags: RKNPU_JOB_PC | RKNPU_JOB_BLOCK | RKNPU_JOB_PINGPONG,
        timeout: 6000,
        task_start: 0,
        task_number: 1,
        task_counter: 0,
        priority: 0,
        task_obj_addr: tasks_mem.obj_addr(),
        regcfg_obj_addr: 0,
        task_base_addr: 0,
        user_data: 0,
        core_mask: 1,               // 使用核心 0
        fence_fd: -1,
        subcore_task: [
            RknpuSubcoreTask { task_start: 0, task_number: 1 },
            RknpuSubcoreTask { task_start: 1, task_number: 0 },
            RknpuSubcoreTask { task_start: 2, task_number: 0 },
            RknpuSubcoreTask { task_start: 0, task_number: 0 },
            RknpuSubcoreTask { task_start: 0, task_number: 0 },
        ],
    };

    npu.submit(&mut submit)?;
    // 内核流程:
    // 1. rknpu_submit_ioctl()
    // 2. rknpu_submit()
    // 3. rknpu_job_alloc()
    // 4. rknpu_job_schedule()
    // 5. rknpu_job_next()
    // 6. rknpu_job_commit()
    // 7. rknpu_job_subcore_commit_pc()  ← 写入 NPU 寄存器
    // 8. 启动 NPU 硬件执行
    // 9. rknpu_irq_handler()  ← 中断处理
    // 10. rknpu_job_done()
    // 11. 唤醒等待进程
    
    println!("Task submitted successfully, completed {} tasks", 
             submit.task_counter);

    // ========== 步骤 9: 验证结果 ==========
    let output_ptr = output_mem.as_ptr();
    let output_data = unsafe { 
        std::slice::from_raw_parts(output_ptr as *const f32, M * N) 
    };
    
    let mut all_match = true;
    for m in 1..=M {
        for n in 1..(N) {
            let idx = feature_data(N as i32, 4, 1, 4, 
                                  n as i32, m as i32, 1) as usize;
            let actual = output_data[idx];
            let expected = EXPECTED_RESULT[(m - 1) * N + (n - 1)];

            if (actual - expected).abs() > 0.1 {
                println!("mismatch m:{}  n:{}  expected:{:6.1}  actual:{:6.1}",
                        m, n, expected, actual);
                all_match = false;
            }
        }
    }

    // ========== 步骤 10: 清理资源 ==========
    // Drop 自动调用 rknpu_mem_destroy_ioctl()

    if all_match {
        println!("✓ All results match expected values!");
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::Other, 
            "Results do not match expected values"))
    }
}
```

### 9.2 内核执行时序图

```
用户态                     内核态                         硬件
  │                         │                             │
  ├─ open("/dev/dri/card1") →│                            │
  │                         ├─ rknpu_open()               │
  │                         │  - 创建会话                  │
  │                         │                             │
  ├─ IOCTL_MEM_CREATE ─────→│                            │
  │                         ├─ rknpu_mem_create_ioctl()   │
  │                         │  - 分配 DMA 缓冲区           │
  │                         │  - 获取物理地址              │
  │                         │                             │
  ├─ IOCTL_RKNPU_ACTION ───→│                            │
  │   (RESET)               ├─ rknpu_soft_reset()         │
  │                         │  - Assert 复位信号 ─────────→│ NPU 复位
  │                         │  - Deassert 复位信号 ───────→│ NPU 就绪
  │                         │                             │
  ├─ 填充输入数据            │                             │
  │                         │                             │
  ├─ IOCTL_RKNPU_SUBMIT ───→│                            │
  │                         ├─ rknpu_submit()             │
  │                         │  └─ rknpu_job_alloc()       │
  │                         │     - 创建作业               │
  │                         │                             │
  │                         ├─ rknpu_job_schedule()       │
  │                         │  - 加入队列                  │
  │                         │                             │
  │                         ├─ rknpu_job_next()           │
  │                         │  - 从队列取出作业            │
  │                         │                             │
  │                         ├─ rknpu_job_commit()         │
  │                         │  └─ rknpu_job_subcore_commit_pc()
  │                         │     ├─ 写 PC_DATA_ADDR ────→│ 设置寄存器命令地址
  │                         │     ├─ 写 PC_DATA_AMOUNT ──→│ 设置命令数量
  │                         │     ├─ 写 INT_MASK ────────→│ 设置中断掩码
  │                         │     ├─ 写 INT_CLEAR ───────→│ 清除中断
  │                         │     ├─ 写 PC_TASK_CONTROL →│ 设置任务控制
  │                         │     ├─ 写 PC_DMA_BASE_ADDR →│ 设置 DMA 基址
  │                         │     └─ 写 PC_OP_EN (1→0) ─→│ 启动 NPU
  │                         │                             │
  │                         ├─ rknpu_job_wait()           │
  │                         │  - 等待中断...               │
  │                         │                             │
  │                         │                             ├─ NPU 执行
  │                         │                             │  - 读取寄存器命令
  │                         │                             │  - 读取输入数据
  │                         │                             │  - 执行卷积运算
  │                         │                             │  - 写入输出数据
  │                         │                             │
  │                         │                      中断 ←─┤ NPU 完成
  │                         │                             │
  │                         ├─ rknpu_irq_handler()        │
  │                         │  ├─ 读 INT_STATUS ─────────│ 读取中断状态
  │                         │  ├─ 验证中断状态            │
  │                         │  ├─ 写 INT_CLEAR ─────────→│ 清除中断
  │                         │  └─ rknpu_job_done()        │
  │                         │     - 标记完成               │
  │                         │     - 唤醒等待               │
  │                         │                             │
  │                         ├─ rknpu_job_wait() 返回      │
  │                         │                             │
  ←─ IOCTL 返回 ────────────┤                            │
  │   (task_counter=1)      │                             │
  │                         │                             │
  ├─ 读取输出数据            │                             │
  │                         │                             │
  ├─ IOCTL_MEM_DESTROY ────→│                            │
  │                         ├─ rknpu_mem_destroy_ioctl()  │
  │                         │  - 释放 DMA 缓冲区           │
  │                         │                             │
  ├─ close()  ─────────────→│                            │
  │                         ├─ rknpu_release()            │
  │                         │  - 清理会话                  │
  │                         │                             │
```

### 9.3 关键路径总结

**从用户态到硬件的完整路径:**

1. **用户态准备**
   - 打开设备: `/dev/dri/card1`
   - 分配内存: `rknpu_mem_create_ioctl()`
   - 填充数据: 输入矩阵、权重

2. **任务构建**
   - 生成寄存器命令: `npuop(op, value, reg)`
   - 创建任务结构: `RknpuTask`
   - 填充提交参数: `RknpuSubmit`

3. **内核处理**
   - 作业分配: `rknpu_job_alloc()`
   - 作业调度: `rknpu_job_schedule()`
   - 队列管理: `list_add_tail()`

4. **硬件执行**
   - 寄存器配置: 写入 7 个关键寄存器
   - NPU 启动: `PC_OP_EN = 1 → 0`
   - 硬件运算: CNA + DPU 协同工作

5. **中断处理**
   - 接收中断: `rknpu_irq_handler()`
   - 验证状态: 检查 `INT_STATUS`
   - 完成通知: `wake_up()`

6. **结果返回**
   - 读取输出: 从 DMA 缓冲区
   - 清理资源: `rknpu_mem_destroy_ioctl()`

---

## 总结

本文档详细说明了 RK3588 RKNPU 驱动的完整工作流程,涵盖了从驱动初始化、电源管理、内存分配、任务提交、作业调度到硬件执行和中断处理的所有关键环节。每个步骤都提供了对应的代码位置和详细说明,便于理解和调试。

**核心要点:**

- **三核心架构**: RK3588 NPU 有三个独立核心,每个核心 2 TOPS
- **PC 模式**: 使用程序计数器模式批量处理任务,提高吞吐量
- **DMA 传输**: 所有数据传输使用 DMA,减少 CPU 负担
- **中断驱动**: 使用中断通知任务完成,提高效率
- **电源管理**: 引用计数管理电源状态,节省功耗

**参考资源:**

- 内核驱动代码: `crates/rknpu/`
- 用户态库: `crates/rknpu2-rslab/rk3588-rs/`
- 测试程序: `crates/rknpu2-rslab/rknpu2/`
- RK3588 TRM: NPU 章节

---

**文档版本**: 1.0  
**最后更新**: 2025年10月15日  
**作者**: AI Assistant (Claude)

