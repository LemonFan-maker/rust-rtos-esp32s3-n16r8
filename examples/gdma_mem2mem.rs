#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::dma_descriptors;
use esp_hal::peripherals::{DMA_CH0, SPI2};
use esp_hal::timer::timg::TimerGroup;

use rustrtos::mem::dma::DmaBuffer;
use rustrtos::mem::gdma::GdmaCopy;
use rustrtos::perf::{cpu_freq_hz, cycles, cycles_to_ns};

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

const COPY_SIZE: usize = 4096;

fn pattern_byte(i: usize) -> u8 {
    (i as u8).wrapping_mul(31).wrapping_add(7)
}

/// GDMA内存到内存拷贝演示: 同一4KB搬运分别由GDMA硬件与CPU完成, 输出周期与时延对比。
#[embassy_executor::task]
async fn gdma_demo_task(dma_ch0: DMA_CH0<'static>, spi2: SPI2<'static>) {
    static SRC: DmaBuffer<COPY_SIZE> = DmaBuffer::new();
    static DST: DmaBuffer<COPY_SIZE> = DmaBuffer::new();
    static CPU_DST: DmaBuffer<COPY_SIZE> = DmaBuffer::new();

    println!("GDMA mem2mem demo started");

    let freq = cpu_freq_hz();
    let (rx_descriptors, tx_descriptors) = dma_descriptors!(COPY_SIZE);

    let mut copier = match GdmaCopy::new(dma_ch0, spi2, rx_descriptors, tx_descriptors) {
        Ok(copier) => copier,
        Err(e) => {
            println!("GDMA mem2mem init failed: {:?}", e);
            loop {
                Timer::after(Duration::from_secs(3600)).await;
            }
        }
    };

    SRC.with_mut(|s| {
        for (i, b) in s.iter_mut().enumerate() {
            *b = pattern_byte(i);
        }
    });
    DST.fill(0);

    // GDMA mem2mem拷贝: CPU侧回写/作废缓存, 搬运全程由GDMA硬件完成
    let dst_ptr = DST.prepare_rx();
    let src_view = SRC.prepare_tx();
    let start = cycles();
    let result = unsafe {
        copier.copy(core::slice::from_raw_parts_mut(dst_ptr, COPY_SIZE), src_view)
    };
    let gdma_cycles = cycles().wrapping_sub(start);
    SRC.finish_tx();
    DST.finish_rx();

    match result {
        Ok(()) => {
            let errors = DST
                .as_slice()
                .iter()
                .enumerate()
                .filter(|(i, b)| **b != pattern_byte(*i))
                .count();
            println!(
                "GDMA copy {} bytes: {} cycles ({} ns), mismatches={}",
                COPY_SIZE,
                gdma_cycles,
                cycles_to_ns(gdma_cycles, freq),
                errors
            );
        }
        Err(e) => {
            println!("GDMA copy failed: {:?}", e);
        }
    }

    // CPU memcpy基线, 同尺寸对照
    let start = cycles();
    CPU_DST.copy_from_slice(SRC.as_slice());
    let cpu_cycles = cycles().wrapping_sub(start);
    println!(
        "CPU memcpy {} bytes: {} cycles ({} ns)",
        COPY_SIZE,
        cpu_cycles,
        cycles_to_ns(cpu_cycles, freq)
    );

    let cpu_errors = CPU_DST
        .as_slice()
        .iter()
        .enumerate()
        .filter(|(i, b)| **b != pattern_byte(*i))
        .count();
    println!(
        "CPU memcpy mismatches={} (GDMA/CPU周期比 {:.2})",
        cpu_errors,
        cpu_cycles as f64 / gdma_cycles as f64
    );

    println!("GDMA mem2mem demo complete");

    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    spawner
        .spawn(gdma_demo_task(peripherals.DMA_CH0, peripherals.SPI2))
        .ok();

    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}
