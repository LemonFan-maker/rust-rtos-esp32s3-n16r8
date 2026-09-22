#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::timer::timg::TimerGroup;
use rustrtos::mem::dma::{DmaBuffer, DmaStrategy};

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

#[embassy_executor::task]
async fn dma_demo_task() {
    println!("DMA Demo Task Started");

    println!("DMA Buffer Allocation");

    let mut buffer: DmaBuffer<256> = DmaBuffer::new(DmaStrategy::ForceDram);

    println!("Buffer created:");
    println!("Size: {} bytes", buffer.size());
    println!("Alignment: {} bytes", buffer.alignment());
    println!("Strategy: {:?}", buffer.strategy());

    println!("Write Test Data");
    {
        let data = buffer.as_mut_slice();
        for i in 0..256 {
            data[i] = i as u8;
        }
    }
    println!("Wrote {} bytes of test pattern", 256);

    println!("Verify Data");
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

    println!("Slice Operations");
    {
        let read_data = buffer.as_slice();
        let first_16 = &read_data[0..16];
        let sum: u32 = first_16.iter().map(|&x| x as u32).sum();
        println!("Sum of first 16 bytes: {} (expected: {})", sum, (0..16).sum::<u32>());
    }

    println!("Fill Operation");
    buffer.fill(0xAA);
    println!("Filled buffer with 0xAA");

    {
        let data = buffer.as_slice();
        let all_aa = data.iter().all(|&b| b == 0xAA);
        println!("Fill verification: {}", if all_aa { "PASS" } else { "FAIL" });
    }

    println!("DMA demo complete!");
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("DMA Transfer Example");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    spawner.spawn(dma_demo_task()).ok();

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
