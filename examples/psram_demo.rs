//! PSRAM 示例 - 外部 RAM 使用演示
//!
//! 演示 PSRAM 内存管理功能:
//! - 大数组分配
//! - 缓存模式配置
//! - 内存统计
//!
//! # 运行
//! ```bash
//! cargo run --example psram_demo --features dev --target xtensa-esp32s3-none-elf
//! ```

#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::timer::timg::TimerGroup;

// ===== 条件编译日志 =====
#[cfg(feature = "dev")]
use esp_println::println;

#[cfg(not(feature = "dev"))]
macro_rules! println {
    ($($arg:tt)*) => {};
}

// ===== Panic Handler =====
#[cfg(feature = "dev")]
use esp_backtrace as _;

#[cfg(not(feature = "dev"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}

/// PSRAM 演示任务
#[embassy_executor::task]
async fn psram_demo_task() {
    println!("PSRAM Demo Task Started");
    
    // 获取 PSRAM 统计信息
    let stats = rustrtos::mem::psram::stats();
    println!("PSRAM Stats:");
    println!("  Total: {} bytes", stats.total);
    println!("  Used: {} bytes", stats.used);
    println!("  Free: {} bytes", stats.free);
    
    // 演示大数组分配
    println!("\nTrying to allocate large array in PSRAM...");
    
    // 使用 PSRAM 分配
    match rustrtos::mem::psram::alloc_array::<u32, 1024>() {
        Ok(mut array) => {
            println!("Allocated 1024 x u32 = 4KB in PSRAM");
            
            // 写入数据
            for i in 0..1024 {
                array[i] = i as u32;
            }
            
            // 验证
            let sum: u32 = array.iter().sum();
            println!("Array sum: {} (expected: {})", sum, 1024 * 1023 / 2);
            
            // 数组会在作用域结束时自动释放
        }
        Err(e) => {
            println!("PSRAM allocation failed: {:?}", e);
            println!("Note: PSRAM may not be initialized or available");
        }
    }
    
    // 显示更新后的统计
    Timer::after(Duration::from_millis(100)).await;
    let stats = rustrtos::mem::psram::stats();
    println!("\nUpdated PSRAM Stats:");
    println!("  Used: {} bytes", stats.used);
    println!("  Free: {} bytes", stats.free);
    
    println!("\nPSRAM demo complete!");
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    println!("PSRAM Demo Example");
    println!("==================");
    
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);
    
    // 初始化 PSRAM
    match rustrtos::mem::psram::init() {
        Ok(info) => {
            println!("PSRAM initialized: {} bytes", info.size);
        }
        Err(e) => {
            println!("PSRAM init failed: {:?}", e);
        }
    }
    
    spawner.spawn(psram_demo_task()).ok();
    
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
