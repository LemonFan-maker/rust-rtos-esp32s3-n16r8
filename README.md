# RustRTOS

基于 Rust 的高性能实时操作系统，专为 ESP32-S3-N16R8 设计。

## 特性

- **混合调度**: 协作式 async/await + 中断驱动抢占
- **多优先级**: 3 级软件中断执行器 (P7/P5/P3)
- **最激进优化**: LTO、单 codegen-unit、IRAM 放置关键代码
- **零拷贝**: 高性能环形缓冲区
- **条件日志**: defmt/esp-println 可切换，release 零开销

## 硬件目标

- **芯片**: ESP32-S3-N16R8
- **CPU**: 双核 Xtensa LX7 @ 240MHz
- **Flash**: 16MB
- **PSRAM**: 8MB
- **内部 SRAM**: 512KB

## 快速开始

### 1. 安装工具链

```bash
# 安装 espup
cargo install espup

# 安装 ESP32-S3 工具链
espup install

# 设置环境变量
source $HOME/export-esp.sh

# 安装烧录工具
cargo install espflash
```

### 2. 构建项目

```bash
# 开发模式 (带日志)
cargo build --features dev

# Release 模式 (最大性能)
cargo build --release
```

### 3. 烧录运行

```bash
# 开发模式烧录
cargo run --features dev

# Release 模式烧录
cargo run --release
```

### 4. 运行示例

```bash
# LED 闪烁
cargo run --example blinky --features dev

# 多优先级演示
cargo run --example multi_priority --features dev

# 性能基准测试
cargo run --example benchmark --release --features dev
```

## 项目结构

```
rustrtos/
├── .cargo/config.toml    # 编译配置
├── Cargo.toml            # 依赖和优化配置
├── rust-toolchain.toml   # 工具链配置
├── src/
│   ├── main.rs           # 主入口
│   ├── lib.rs            # 库导出
│   ├── tasks/
│   │   ├── critical.rs   # 高优先级任务 (IRAM)
│   │   └── normal.rs     # 普通优先级任务
│   ├── sync/
│   │   ├── primitives.rs # 同步原语
│   │   └── ringbuffer.rs # 零拷贝环形缓冲区
│   └── util/
│       └── log.rs        # 条件编译日志
└── examples/
    ├── blinky.rs         # LED 闪烁示例
    ├── multi_priority.rs # 多优先级示例
    └── benchmark.rs      # 性能基准测试
```

## 软件中断分配

| 中断 | 优先级 | 用途 |
|------|--------|------|
| SW_INT0 | - | esp-rtos 调度器 |
| SW_INT1 | P5 | 中优先级任务 |
| SW_INT2 | P7 | 高优先级任务 |
| SW_INT3 | P6 | 预留 (双核扩展) |

## Feature 配置

| Feature | 说明 |
|---------|------|
| `dev` | 开发模式: defmt + esp-backtrace |
| `log-defmt` | 仅 defmt 日志 |
| `log-println` | 仅 esp-println 日志 |
| (默认) | Release 模式: 无日志，零开销 |

## 性能目标

| 指标 | 目标值 |
|------|--------|
| 中断响应延迟 | < 1μs |
| 任务切换时间 | < 500ns |
| Timer 抖动 | < 10μs (avg) |
| Release 二进制 | < 100KB |

## 内存布局

| 区域 | 大小 | 用途 |
|------|------|------|
| IRAM | 64KB | 关键代码、中断处理 |
| DRAM | 256KB | 任务栈、热数据 |
| PSRAM | 8MB | 大型缓冲区 (非关键) |

## License

MIT License
