#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::timer::timg::TimerGroup;
use portable_atomic::{AtomicU32, Ordering};

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

static CORE0_COUNTER: AtomicU32 = AtomicU32::new(0);
static CORE1_COUNTER: AtomicU32 = AtomicU32::new(0);
static IPC_CHANNEL: rustrtos::tasks::multicore::IpcChannel<u32, 8> =
    rustrtos::tasks::multicore::IpcChannel::new();
static IPC_SIGNAL: rustrtos::tasks::multicore::IpcSignal =
    rustrtos::tasks::multicore::IpcSignal::new();

#[embassy_executor::task]
async fn core0_task() {
    println!("Core0 task started");

    loop {
        CORE0_COUNTER.fetch_add(1, Ordering::Relaxed);
        Timer::after(Duration::from_millis(100)).await;
    }
}

#[embassy_executor::task]
async fn monitor_task() {
    println!("Monitor task started");

    loop {
        Timer::after(Duration::from_secs(2)).await;

        let c0 = CORE0_COUNTER.load(Ordering::Relaxed);
        let c1 = CORE1_COUNTER.load(Ordering::Relaxed);

        println!("Core Status");
        println!("Core0 counter: {}", c0);
        #[cfg(feature = "multicore")]
        println!("Core1 counter: {} (real Core1 task)", c1);
        #[cfg(not(feature = "multicore"))]
        println!("Core1 counter: {} (simulated)", c1);
        println!("Total: {}", c0 + c1);
    }
}

#[embassy_executor::task]
async fn ipc_receiver_task() {
    println!("IPC receiver running on Core0");
    let mut received = 0u32;

    while received < 5 {
        if let Some(value) = IPC_CHANNEL.try_recv() {
            println!("Core0 received {} from Core1", value);
            received += 1;
        }

        if IPC_SIGNAL.try_wait() {
            println!("Core0 consumed IPC signal");
        }

        Timer::after(Duration::from_millis(10)).await;
    }

    println!("Cross-core IPC verified: received {} messages", received);
}

#[cfg(not(feature = "multicore"))]
#[embassy_executor::task]
async fn simulated_ipc_producer_task() {
    println!("IPC producer simulated on Core0");
    for value in 0..5u32 {
        loop {
            match IPC_CHANNEL.try_send(value) {
                Ok(()) => {
                    IPC_SIGNAL.signal();
                    println!("Core0 simulated producer sent {}", value);
                    break;
                }
                Err(_) => Timer::after(Duration::from_millis(10)).await,
            }
        }
        Timer::after(Duration::from_millis(500)).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("Dual Core Example");
    #[cfg(feature = "multicore")]
    println!("Mode: real dual-core, esp-rtos scheduler on Core1");
    #[cfg(not(feature = "multicore"))]
    println!("Mode: Core1 simulated on Core0 (enable 'multicore' feature for real dual-core)");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);
    #[cfg(feature = "multicore")]
    {
        use esp_hal::interrupt::software::SoftwareInterruptControl;
        use esp_hal::system::Stack;
        use rustrtos::tasks::multicore::Core1;
        use static_cell::StaticCell;

        let sw_ints = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
        static CORE1_STACK: StaticCell<Stack<8192>> = StaticCell::new();
        let core1_stack = CORE1_STACK.init(Stack::new());

        Core1::start_with_rtos(
            peripherals.CPU_CTRL,
            sw_ints.software_interrupt0,
            sw_ints.software_interrupt1,
            core1_stack,
            || {
                println!("Core1 entry running on APP_CPU");
                let handle = esp_rtos::CurrentThreadHandle::get();
                let mut next_message = 0u32;
                loop {
                    CORE1_COUNTER.fetch_add(1, Ordering::Relaxed);
                    if next_message < 5 && IPC_CHANNEL.try_send(next_message).is_ok() {
                        IPC_SIGNAL.signal();
                        println!("Core1 sent {} to Core0", next_message);
                        next_message += 1;
                    }
                    handle.delay(esp_hal::time::Duration::from_millis(200));
                }
            },
        );
        Core1::wait_ready();
        println!("Core1 scheduler ready");
    }

    spawner.spawn(core0_task()).ok();
    spawner.spawn(monitor_task()).ok();
    spawner.spawn(ipc_receiver_task()).ok();
    #[cfg(not(feature = "multicore"))]
    spawner.spawn(simulated_ipc_producer_task()).ok();

    #[cfg(not(feature = "multicore"))]
    loop {
        CORE1_COUNTER.fetch_add(1, Ordering::Relaxed);
        Timer::after(Duration::from_millis(200)).await;
    }
}
