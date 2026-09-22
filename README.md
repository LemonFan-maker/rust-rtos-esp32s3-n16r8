# RustRTOS

基于Rust的高性能实时操作系统，专为ESP32-S3-N16R8设计。

## 特性

- **混合调度**: 协作式async/await + 中断驱动抢占
- **多优先级**: 三级执行器(高P3/中P2软件中断执行器 + 主执行器)
- **编译期优化**: LTO、单codegen-unit、`#[ram]`放置关键代码
- **定频采样节拍**: Ticker绝对期限对齐，超期丢弃补发不漂移
- **零拷贝同步**: Signal/Channel/RingBuffer(原子无锁)
- **条件日志**: defmt/esp-println可切换，release零开销

## 硬件目标

- **芯片**: ESP32-S3-N16R8
- **CPU**: 双核Xtensa LX7 @ 240MHz
- **Flash**: 16MB
- **PSRAM**: 8MB
- **内部SRAM**: 512KB

## 快速开始

### 1.安装工具链

```bash
# 安装espup
cargo install espup

# 安装ESP32-S3工具链
espup install

# 设置环境变量
source $HOME/export-esp.sh
```

### 2.安装烧录工具

```bash
# cargo子命令形态(推荐，可在编译后直接烧录)
cargo install cargo-espflash
```

### 3.构建

```bash
# 开发模式(esp-println日志 + panic回溯)
cargo build --features dev

# Release模式(无日志，最大优化)
cargo build --release
```

### 4.烧录运行

```bash
# 开发模式: 编译并烧录，随后进入串口监视
cargo espflash flash --features dev --monitor

# Release模式
cargo espflash flash --release --monitor

# 或使用.cargo/config.toml中定义的别名
cargo fd    # = espflash flash --features dev --monitor
```

### 5.运行示例

```bash
# LED闪烁 / 多优先级 / 基准测试
cargo espflash flash --example blinky --features dev --monitor
cargo espflash flash --example multi_priority --features dev --monitor

# 网络类示例需启用对应feature(见下方Feature表)
cargo espflash flash --example wifi_scan --features wifi,dev --monitor
```

## 项目结构

```
rustrtos/
├── .cargo/config.toml    # 编译配置(target/rustflags/别名)
├── Cargo.toml            # 依赖和优化配置
├── rust-toolchain.toml   # 工具链配置(channel = "esp")
├── ld/rodata.x           # 自定义链接段(App Descriptor置首)
├── src/
│   ├── main.rs           # 主入口(演示应用)
│   ├── lib.rs            # 库导出
│   ├── tasks/
│   │   ├── critical.rs   # 高优先级任务(100μs定频采样, IRAM)
│   │   ├── normal.rs     # 中/低优先级任务
│   │   └── multicore.rs  # 双核调度支持
│   ├── sync/
│   │   ├── primitives.rs # 同步原语(Signal/Channel/Mutex)
│   │   └── ringbuffer.rs # 无锁环形缓冲区
│   ├── mem/              # PSRAM/内存池/DMA缓冲
│   ├── fs/               # 分区表/LittleFS存储后端
│   ├── net/              # WiFi/BLE/TCP(条件编译)
│   └── util/
│       └── log.rs        # 条件编译日志
└── examples/             # blinky..benchmark_network共15个示例
```

## 软件中断分配

| 中断 | 优先级 | 用途 |
|------|--------|------|
| SW_INT0 | - | esp-rtos调度器 |
| SW_INT1 | P2 | 中优先级执行器 |
| SW_INT2 | P3 | 高优先级执行器 |

## Feature配置

| Feature | 说明 |
|---------|------|
| `dev` | 开发模式: esp-println日志 + esp-backtrace |
| `log-defmt` | 仅defmt日志 |
| `log-println` | 仅esp-println日志 |
| `wifi` / `ble` / `coex` | esp-radio射频功能 |
| `network` | WiFi + embassy-net(TCP/UDP/DNS/DHCP) |
| `full-network` | network + BLE |
| (默认) | Release: 日志全部剥离，零开销 |

## License

MIT License
