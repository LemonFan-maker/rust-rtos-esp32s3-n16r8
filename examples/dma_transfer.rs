//! DMA 传输示例 - 直接内存访问
//!
//! 演示 DMA 缓冲区管理:
//! - DMA 对齐的缓冲区分配
//! - 零拷贝数据传输概念
//!
//! # 运行
//! ```bash
//! cargo run --example dma_transfer --features dev --target xtensa-esp32s3-none-elf
//! ```

#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::timer::timg::TimerGroup;
use rustrtos::mem::dma::{DmaBuffer, DmaStrategy};

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

/// DMA 演示任务
#[embassy_executor::task]
async fn dma_demo_task() {
    println!("DMA Demo Task Started");
    
    // 创建 DMA 缓冲区
    println!("\n=== DMA Buffer Allocation ===");
    
    let mut buffer: DmaBuffer<256> = DmaBuffer::new(DmaStrategy::ForceDram);
    
    println!("Buffer created:");
    println!("  Size: {} bytes", buffer.size());
    println!("  Alignment: {} bytes", buffer.alignment());
    println!("  Strategy: {:?}", buffer.strategy());
    
    // 写入测试数据
    println!("\n=== Write Test Data ===");
    {
        let data = buffer.as_mut_slice();
        for i in 0..256 {
            data[i] = i as u8;
        }
    }
    println!("Wrote {} bytes of test pattern", 256);
    
    // 验证数据
    println!("\n=== Verify Data ===");
    let mut errors = 0;
    {
        let read_data = buffer.as_slice();
        for i in 0..256 {
            if read_data[i] != i as u8 {
                errors += 1;
            }
        }
    }
    println!("Verification: {} errors found", errors);
    
    // 测试切片操作
    println!("\n=== Slice Operations ===");
    {
        let read_data = buffer.as_slice();
        let first_16 = &read_data[0..16];
        let sum: u32 = first_16.iter().map(|&x| x as u32).sum();
        println!("Sum of first 16 bytes: {} (expected: {})", sum, (0..16).sum::<u32>());
    }
    
    // 测试填充操作
    println!("\n=== Fill Operation ===");
    buffer.fill(0xAA);
    println!("Filled buffer with 0xAA");
    
    // 验证填充
    {
        let data = buffer.as_slice();
        let all_aa = data.iter().all(|&b| b == 0xAA);
        println!("Fill verification: {}", if all_aa { "PASS" } else { "FAIL" });
    }
    
    println!("\nDMA demo complete!");
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    println!("DMA Transfer Example");
    println!("====================");
    
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);
    
    spawner.spawn(dma_demo_task()).ok();
    
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
