# RustRTOS

面向ESP32-S3-N16R8的no_std嵌入式实时软件框架，基于Rust构建，底层使用esp-hal、esp-rtos与Embassy异步运行时。

## 特性

- **混合执行**: 协作式async/await任务 + 中断驱动执行器（embassy-executor `InterruptExecutor`）
- **多优先级**: 三级执行器（高Priority3 / 中Priority2软件中断执行器 + Core0主执行器）
- **定频采样节拍**: `Ticker`按绝对期限对齐，超期丢弃补发不累积漂移
- **编译期优化**: fat LTO、单codegen-unit、`#[ram]`放置关键代码
- **无锁同步**: Signal / Channel / RingBuffer（原子操作；RingBuffer切片读取在该API调用处不复制载荷）
- **条件日志**: esp-println文本日志与defmt二进制日志可切换；未启用日志特性时日志宏不产生串口/RTT输出
- **静态内存**: 固定容量内存池（DRAM）、PSRAM链表分配器、DMA缓冲区对齐与缓存同步辅助
- **片上Flash存储**: 分区表 + LittleFS文件系统适配层
- **网络**: WiFi扫描/连接、embassy-net TCP/UDP/DHCP/DNS、BLE（trouble-host）、HTTP OTA升级

功能边界由编译特性与示例程序决定；性能与稳定性受目标硬件、feature组合和运行环境影响。

## 硬件目标

- **芯片**: ESP32-S3-N16R8（双核Xtensa LX7 @ 240MHz）
- **Flash**: 16MB
- **PSRAM**: 8MB（octal模式；esp-hal默认quad探测不到，已在`.cargo/config.toml`经`ESP_HAL_CONFIG_PSRAM_MODE=octal`显式指定）
- **内部SRAM**: 512KB

## 快速开始

### 1. 安装工具链

```bash
# 安装espup
cargo install espup

# 安装ESP32-S3工具链
espup install

# 设置环境变量
source $HOME/export-esp.sh
```

`rust-toolchain.toml`钉住`channel = "esp"`（Xtensa目标需要esp专用工具链，非普通stable Rust）。

### 2. 安装烧录工具

```bash
# cargo子命令形态(推荐，可在编译后直接烧录)
cargo install cargo-espflash
```

### 3. 构建

```bash
# 开发模式(esp-println日志 + panic回溯)
cargo build --features dev

# Release模式(无日志，LTO + codegen-units=1)
cargo build --release

# 体积优化Release(注意: 使用--profile,不是--release)
cargo build --profile release-size

# 性能测量Profile(继承release，保留调试符号)
cargo build --profile bench
```

### 4. 烧录运行

```bash
# 开发模式: 编译并烧录，随后进入串口监视
cargo espflash flash --features dev --monitor

# Release模式
cargo espflash flash --release --monitor

# 或使用.cargo/config.toml中定义的别名
cargo fd    # = espflash flash --features dev --monitor
cargo fr    # = espflash flash --release --monitor
```

### 5. 运行示例

```bash
# LED闪烁 / 多优先级
cargo espflash flash --example blinky --features dev --monitor
cargo espflash flash --example multi_priority --features dev --monitor

# 网络类示例需启用对应feature(required-features以Cargo.toml为准)
cargo espflash flash --example wifi_scan --features wifi,dev --monitor
cargo espflash flash --example tcp_client --features network,dev --monitor

# BLE示例(ble与ble-esp互斥，不可同时启用)
cargo espflash flash --example ble_gatt_server --features ble,dev --monitor

# 双核示例(需multicore才启动Core1并进行跨核消息传递)
cargo espflash flash --example dual_core --features dev,multicore --monitor
```

## 示例程序

`examples/`目录共16个示例，名称与`required-features`以`Cargo.toml`为准：

| 示例 | 强制特性 | 内容 |
|------|----------|------|
| `blinky` | 无（dev可选） | GPIO周期性翻转 |
| `multi_priority` | 无（dev可选） | 三级优先级任务与Signal/Channel |
| `dual_core` | 无（需multicore才跨核） | Core1启动与IpcChannel/IpcSignal消息传递 |
| `memory_pool` | 无（dev可选） | 固定容量内存池分配、池满失败与复用 |
| `psram_demo` | 无（dev可选） | PSRAM初始化、统计与大数组分配 |
| `dma_transfer` | 无（dev可选） | DmaBuffer布局、对齐与缓存同步辅助 |
| `filesystem` | 无（dev可选） | LittleFS挂载/读写/目录/清理（会擦写Flash窗口） |
| `wifi_scan` | wifi,dev | AP扫描与RSSI表 |
| `wifi_connect` | network,dev | STA连接与DHCP |
| `tcp_client` | network,dev | HTTP GET与TCP收发 |
| `ble_advertise` | ble,dev | BLE广播 |
| `ble_gatt_server` | ble,dev | GATT服务与通知 |
| `benchmark` | 无（dev可选） | 任务spawn延迟、yield路径、定时器精度 |
| `benchmark_memory` | 无（dev可选） | 内存池操作耗时 |
| `benchmark_network` | network,dev | TCP吞吐与时延（需远端TCP服务） |
| `ota_update` | network,dev | HTTP下载镜像写入OTA槽并切换启动槽 |

## 项目结构

```
rustrtos/
├── .cargo/config.toml    # 编译配置(target/rustflags/环境变量/别名)
├── Cargo.toml            # 依赖、feature与profile配置
├── rust-toolchain.toml   # 工具链配置(channel = "esp")
├── build.rs              # 链接脚本搜索路径与defmt链接片段
├── ld/rodata.x           # 自定义rodata段布局(App Descriptor置首)
├── partitions.csv        # 16MB OTA分区布局(nvs/phy/otadata/factory/ota_0/ota_1/storage)
├── src/
│   ├── main.rs           # 主入口(演示应用与执行器装配)
│   ├── lib.rs            # 库导出
│   ├── tasks/
│   │   ├── critical.rs   # 高优先级任务(定频采样, IRAM放置)
│   │   ├── normal.rs     # 中/低优先级任务
│   │   └── multicore.rs  # Core1启动与跨核IPC
│   ├── sync/
│   │   ├── primitives.rs # 同步原语(Signal/Channel/Mutex)
│   │   └── ringbuffer.rs # 无锁环形缓冲区
│   ├── mem/              # pool.rs(内存池) / psram.rs / dma.rs
│   ├── fs/               # partition.rs / storage.rs / littlefs.rs
│   ├── net/              # wifi.rs / tcp.rs / ble.rs / config.rs(条件编译)
│   ├── ota.rs            # OTA槽位切换、镜像头校验与确认状态
│   └── util/log.rs       # 条件编译日志
└── examples/             # 16个示例(见上表)
```

## 软件中断分配

| 中断 | 优先级 | 用途 |
|------|--------|------|
| SW_INT0 + SW_INT1 | — | esp-rtos启动Core1时使用的核间IPI（`multicore`启用后） |
| SW_INT2 | Priority2 | 中优先级执行器 |
| SW_INT3 | Priority3 | 高优先级执行器 |

Core1由`Core1::start_with_rtos()`经`esp_rtos::start_second_core()`启动，需要占用SW_INT0与SW_INT1。系统不提供SMP任务迁移，`CoreAssignment`仅是任务放置建议。

## Feature配置

| Feature | 说明 |
|---------|------|
| `dev` | 开发模式：esp-println日志 + esp-backtrace崩溃回溯 |
| `log-println` | 仅esp-println文本日志（含esp-backtrace） |
| `log-defmt` | 仅defmt二进制日志（RTT）；panic处理器由`dev`门控，需与`dev`组合使用 |
| `wifi` | esp-radio WiFi（STA/AP），不含TCP/IP协议栈 |
| `ble` | BLE，使用trouble-host（纯Rust实现） |
| `ble-esp` | BLE，仅使用esp-radio内置BLE；与`ble`互斥 |
| `coex` | WiFi + BLE共存（聚合`wifi`与`ble`并启用esp-radio/coex） |
| `network` | `wifi` + embassy-net（TCP/UDP/DNS/DHCP） |
| `full-network` | `network` + `ble` |
| `multicore` | 启用Core1启动路径（`start_with_rtos`） |
| （默认） | `default = []`；文件系统模块属基础构建内容，无条件编译 |

## OTA升级

`examples/ota_update.rs`演示经HTTP下载应用镜像、写入OTA槽、切换启动槽并复位，配合`partitions.csv`（factory + ota_0 + ota_1 + storage）使用。

```bash
# 首次部署须显式指定分区表(项目未配置espflash.toml自动加载)
cargo espflash flash --example ota_update --features network,dev \
    --partition-table partitions.csv --monitor
```

使用限制：

- ESP32-S3仅支持2.4GHz Wi-Fi，配置的SSID须工作在2.4GHz频段；
- 示例使用明文HTTP，仅校验ESP镜像起始magic字节，不含TLS、签名或来源认证，只适用于可信隔离网络；
- 自动回滚需要自行编译并启用`CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE=y`的ESP-IDF第二阶段引导程序，且重新烧写时须注意`cargo espflash flash`可能覆盖0x0处的引导程序；
- `--erase-parts otadata`会清除启动槽选择记录，执行前须确认设备内容可清除。

## License

MIT License
