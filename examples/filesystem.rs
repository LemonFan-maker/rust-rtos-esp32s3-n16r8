//! 文件系统示例 - LittleFS 演示
//!
//! 演示文件系统操作:
//! - 文件创建和写入
//! - 文件读取
//! - 目录操作
//!
//! # 运行
//! ```bash
//! cargo run --example filesystem --features dev --target xtensa-esp32s3-none-elf
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

/// 文件系统演示任务
#[embassy_executor::task]
async fn fs_demo_task() {
    println!("Filesystem Demo Task Started");
    
    // 演示分区表
    println!("\n=== Partition Table Demo ===");
    
    use rustrtos::fs::{PartitionTable, PartitionType, DataSubType};
    
    // 创建分区表并手动添加分区
    let mut table = PartitionTable::new();
    
    // 添加 NVS 分区
    let _ = table.add_partition(
        "nvs", 
        PartitionType::Data, 
        DataSubType::Nvs.as_u8(), 
        0x9000, 
        0x5000
    );
    
    // 添加 storage 分区 (LittleFS)
    let _ = table.add_partition(
        "storage", 
        PartitionType::Data, 
        DataSubType::LittleFs.as_u8(), 
        0x110000, 
        0xF00000  // 15MB
    );
    
    println!("Created partition table with {} partitions:", table.len());
    for partition in table.partitions() {
        println!("  {}: offset=0x{:X}, size=0x{:X} ({} KB)", 
            partition.label.as_str(),
            partition.offset,
            partition.size,
            partition.size / 1024
        );
    }
    
    // 查找 LittleFS 分区
    println!("\n=== Finding LittleFS Partition ===");
    if let Some(storage) = table.find_by_label("storage") {
        println!("Found storage partition:");
        println!("  Offset: 0x{:X}", storage.offset);
        println!("  Size: {} MB", storage.size / 1024 / 1024);
        println!("  Is LittleFS: {}", storage.is_littlefs());
    }
    
    // 演示 Flash 存储抽象
    println!("\n=== Flash Storage Demo ===");
    
    use rustrtos::fs::FlashStorage;
    
    let storage = FlashStorage::with_defaults();
    let config = storage.config();
    
    println!("Flash configuration:");
    println!("  Total size: {} MB", config.total_size / 1024 / 1024);
    println!("  Sector size: {} bytes", config.sector_size);
    println!("  Block size: {} bytes", config.block_size);
    println!("  Page size: {} bytes", config.page_size);
    
    // 计算块数
    let block_count = config.partition_size / config.block_size;
    println!("  Partition blocks: {}", block_count);
    
    println!("\nFilesystem demo complete!");
    println!("Note: Actual Flash operations require hardware");
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    println!("Filesystem Example");
    println!("==================");
    
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);
    
    spawner.spawn(fs_demo_task()).ok();
    
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
