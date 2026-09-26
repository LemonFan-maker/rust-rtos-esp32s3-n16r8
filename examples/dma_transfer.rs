#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::timer::timg::TimerGroup;
use rustrtos::mem::dma::DmaBuffer;

#[cfg(feature = "dev")]
use esp_println::println;

#[cfg(not(feature = "dev"))]
macro_rules! println {
    ($($arg:tt)*) => {{
        fn _log_disabled(_: ::core::fmt::Arguments<'_>) {}
        _log_disabled(::core::format_args!($($arg)*));
    }};
}

#[cfg(feature = "dev")]
use esp_backtrace as _;

#[cfg(not(feature = "dev"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}

fn pattern_byte(i: usize) -> u8 {
    (i as u8).wrapping_mul(31).wrapping_add(7)
}

/// 演示DmaBuffer的缓存维护生命周期:
/// 发送侧 fill -> prepare_tx(回写缓存) -> (外设读取物理内存) -> finish_tx;
/// 接收侧 prepare_rx(作废缓存) -> (外设写入物理内存) -> finish_rx -> 读取。
/// 真实GDMA搬运路径见gdma_mem2mem示例。
#[embassy_executor::task]
async fn dma_demo_task() {
    static BUF: DmaBuffer<256> = DmaBuffer::new();

    println!("DMA Demo Task Started");
    println!("Buffer created: size={} alignment={}", BUF.size(), BUF.alignment());

    println!("Write Test Data");
    BUF.with_mut(|s| {
        for (i, b) in s.iter_mut().enumerate() {
            *b = pattern_byte(i);
        }
    });
    println!("Wrote {} bytes of test pattern", BUF.size());

    println!("TX path: prepare_tx flushes dirty cache lines");
    let tx_view = BUF.prepare_tx();
    // 此处外设(DMA)从物理内存读取tx_view; 示例中以逐字节校验代替。
    let mut tx_errors = 0usize;
    for (i, b) in tx_view.iter().enumerate() {
        if *b != pattern_byte(i) {
            tx_errors += 1;
        }
    }
    BUF.finish_tx();
    println!("TX verification: {} errors found", tx_errors);

    println!("RX path: prepare_rx invalidates stale cache lines");
    let rx_ptr = BUF.prepare_rx();
    // 模拟外设把数据写入物理内存。
    unsafe {
        for (i, b) in core::slice::from_raw_parts_mut(rx_ptr, BUF.size())
            .iter_mut()
            .enumerate()
        {
            *b = 0xAA ^ (i as u8);
        }
    }
    BUF.finish_rx();
    let mut rx_errors = 0usize;
    for (i, b) in BUF.as_slice().iter().enumerate() {
        if *b != 0xAA ^ (i as u8) {
            rx_errors += 1;
        }
    }
    println!("RX verification: {} errors found", rx_errors);

    println!("Fill Operation");
    BUF.fill(0x5A);
    println!("Filled buffer with 0x5A");

    println!("DMA demo complete!");
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    spawner.spawn(dma_demo_task()).ok();

    loop {
        Timer::after(Duration::from_secs(10)).await;
    }
}
