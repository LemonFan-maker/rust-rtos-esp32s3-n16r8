//! RustRTOS - ESP32-S3 高性能实时操作系统
//!
//! 基于 esp-hal 1.0 和 esp-rtos，采用混合调度策略：
//! - 协作式 async/await 任务调度 (Embassy)
//! - 中断驱动的高优先级任务抢占 (InterruptExecutor)
//! - 双核支持 (Core0 + Core1)
//!
//! 硬件目标: ESP32-S3-N16R8 (双核 Xtensa LX7 @ 240MHz, 16MB Flash, 8MB PSRAM)

#![no_std]
#![no_main]
#![feature(asm_experimental_arch)]

// ===== 模块导入 =====
mod tasks;
mod sync;
mod util;
mod mem;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::{
    gpio::{Level, Output, OutputConfig},
    interrupt::{software::SoftwareInterruptControl, Priority},
    timer::timg::TimerGroup,
};
use esp_rtos::embassy::InterruptExecutor;
use static_cell::StaticCell;

// ===== ESP App Descriptor =====
esp_bootloader_esp_idf::esp_app_desc!();

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
#[esp_rtos::main]
async fn main(low_prio_spawner: Spawner) {
    // ========================================
    // 1. 硬件初始化 (esp-hal 1.0 API)
    // ========================================
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    log_info!("RustRTOS v{} starting on ESP32-S3", env!("CARGO_PKG_VERSION"));
    
    // ========================================
    // 2. GPIO 初始化 (esp-hal 1.0 新 API)
    // ========================================
    // 板载 LED (根据实际硬件调整引脚)
    let led = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());
    
    // ========================================
    // 3. 定时器初始化 (Embassy 时间驱动)
    // ========================================
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    
    // ========================================
    // 4. 软件中断控制初始化
    // ========================================
    let sw_ints = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    
    // ========================================
    // 5. 初始化 esp-rtos 调度器
    // ========================================
    // 注意: Xtensa 架构只需要 timer 参数，RISC-V 才需要 software_interrupt
    esp_rtos::start(timg0.timer0);
    
    log_info!("esp-rtos scheduler initialized");
    
    // ========================================
    // 6. 配置高优先级执行器 (Priority3 是 ESP32-S3 最高可用)
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
