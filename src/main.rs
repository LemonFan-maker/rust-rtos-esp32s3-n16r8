//! RustRTOS - ESP32-S3 高性能实时操作系统
//!
//! 基于 Embassy 异步运行时，采用混合调度策略：
//! - 协作式 async/await 任务调度
//! - 中断驱动的高优先级任务抢占
//!
//! 硬件目标: ESP32-S3-N16R8 (双核 Xtensa LX7 @ 240MHz, 16MB Flash, 8MB PSRAM)

#![no_std]
#![no_main]
#![feature(asm_experimental_arch)]

// ===== 模块导入 =====
mod tasks;
mod sync;
mod util;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::{
    gpio::{Level, Output},
    interrupt::{software::SoftwareInterruptControl, Priority},
    timer::timg::TimerGroup,
};
use esp_hal_embassy::InterruptExecutor;
use static_cell::StaticCell;

// ===== ESP-IDF 兼容 App Descriptor (手动定义) =====
// 设置 min_efuse_blk_rev_full = 0 以支持所有芯片版本
#[repr(C)]
struct EspAppDesc {
    magic_word: u32,
    secure_version: u32,
    reserv1: [u32; 2],
    version: [u8; 32],
    project_name: [u8; 32],
    time: [u8; 16],
    date: [u8; 16],
    idf_ver: [u8; 32],
    app_elf_sha256: [u8; 32],
    min_efuse_blk_rev_full: u16,  // 设置为 0
    max_efuse_blk_rev_full: u16,  // 设置为 u16::MAX
    mmu_page_size: u8,
    reserv3: [u8; 3],
    reserv2: [u32; 18],
}

#[link_section = ".flash.appdesc"]
#[used]
static ESP_APP_DESC: EspAppDesc = EspAppDesc {
    magic_word: 0xABCD5432,
    secure_version: 0,
    reserv1: [0; 2],
    version: *b"0.1.0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
    project_name: *b"rustrtos\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
    time: *b"00:00:00\0\0\0\0\0\0\0\0",
    date: *b"2025-01-01\0\0\0\0\0\0",
    idf_ver: *b"v5.0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
    app_elf_sha256: [0; 32],
    min_efuse_blk_rev_full: 0,     // 支持所有芯片版本
    max_efuse_blk_rev_full: u16::MAX,
    mmu_page_size: 16,  // 64KB = 2^16
    reserv3: [0; 3],
    reserv2: [0; 18],
};

// ===== 条件编译日志 =====
#[allow(unused_imports)]
use crate::util::log::*;

// ===== Panic Handler =====
#[cfg(feature = "dev")]
use esp_backtrace as _;

// defmt panic handler implementation
#[cfg(all(feature = "defmt", feature = "dev"))]
#[defmt::panic_handler]
fn defmt_panic() -> ! {
    loop {
        unsafe { core::arch::asm!("break 1, 15") }
    }
}

#[cfg(not(feature = "dev"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}

// ===== 静态分配 =====
/// 高优先级执行器 - 关键实时任务
static HIGH_PRIO_EXECUTOR: StaticCell<InterruptExecutor<2>> = StaticCell::new();

/// 中优先级执行器 - 普通任务
static MID_PRIO_EXECUTOR: StaticCell<InterruptExecutor<1>> = StaticCell::new();

// ===== 内存布局优化: 关键数据 32 字节缓存行对齐 =====
/// 系统状态结构 - 强制 DRAM 放置
#[repr(C, align(32))]
pub struct SystemState {
    /// 系统启动时间戳 (μs)
    pub boot_time: u64,
    /// 任务切换计数
    pub context_switches: u32,
    /// 系统状态标志
    pub flags: u32,
    /// 填充到缓存行边界
    _pad: [u8; 16],
}

impl SystemState {
    pub const fn new() -> Self {
        Self {
            boot_time: 0,
            context_switches: 0,
            flags: 0,
            _pad: [0; 16],
        }
    }
}

/// 全局系统状态 - 放入 DRAM
#[link_section = ".dram.data"]
static mut SYSTEM_STATE: SystemState = SystemState::new();

// ===== 主入口点 =====
#[esp_hal_embassy::main]
async fn main(low_prio_spawner: Spawner) {
    // ========================================
    // 1. 硬件初始化 (esp-hal 0.23 新 API)
    // ========================================
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    log_info!("RustRTOS starting on ESP32-S3");
    
    // ========================================
    // 2. GPIO 初始化
    // ========================================
    // 板载 LED (根据实际硬件调整引脚)
    let led = Output::new(peripherals.GPIO2, Level::Low);
    
    // ========================================
    // 3. 定时器初始化 (Embassy 时间驱动)
    // ========================================
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    
    // ========================================
    // 4. 软件中断控制初始化
    // ========================================
    let sw_ints = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    
    // ========================================
    // 5. 初始化 Embassy
    // ========================================
    esp_hal_embassy::init(timg0.timer0);
    
    log_info!("Embassy initialized");
    
    // ========================================
    // 6. 配置高优先级执行器 (Priority3 是较高优先级)
    // ========================================
    let high_prio_executor = InterruptExecutor::new(sw_ints.software_interrupt2);
    let high_prio_executor = HIGH_PRIO_EXECUTOR.init(high_prio_executor);
    
    let high_prio_spawner = high_prio_executor.start(Priority::Priority3);
    
    log_info!("High priority executor started (Priority3)");
    
    // 生成高优先级任务
    high_prio_spawner.must_spawn(tasks::critical::critical_sensor_task());
    
    // ========================================
    // 7. 配置中优先级执行器 (Priority2)
    // ========================================
    let mid_prio_executor = InterruptExecutor::new(sw_ints.software_interrupt1);
    let mid_prio_executor = MID_PRIO_EXECUTOR.init(mid_prio_executor);
    
    let mid_prio_spawner = mid_prio_executor.start(Priority::Priority2);
    
    log_info!("Mid priority executor started (Priority2)");
    
    // 生成中优先级任务
    mid_prio_spawner.must_spawn(tasks::normal::periodic_task());
    
    // ========================================
    // 8. 低优先级任务 (主执行器)
    // ========================================
    low_prio_spawner.must_spawn(tasks::normal::led_blink_task(led));
    low_prio_spawner.must_spawn(tasks::normal::background_task());
    
    log_info!("All tasks spawned, entering main loop");
    
    // ========================================
    // 9. 主循环 - 系统监控
    // ========================================
    let mut tick_count: u64 = 0;
    
    loop {
        tick_count += 1;
        
        // 更新系统状态
        unsafe {
            SYSTEM_STATE.flags = tick_count as u32;
        }
        
        // 每 10 秒输出系统状态
        if tick_count % 10 == 0 {
            log_info!("System heartbeat: {} ticks", tick_count);
        }
        
        Timer::after(Duration::from_secs(1)).await;
    }
}
