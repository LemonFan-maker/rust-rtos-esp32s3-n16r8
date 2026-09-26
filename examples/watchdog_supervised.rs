#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::timer::timg::TimerGroup;

use rustrtos::watchdog::{supervised_feed, Heartbeat, Supervisor, Watchdog};

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

/// 静态监督器: 至多4个受监督任务, 无堆分配。
static SUPERVISOR: Supervisor<4> = Supervisor::new();

/// 被监督的工作任务: 每200ms跳动一次, 截止期1000ms。
/// 若因任何原因(死锁、忙等、优先级反转)超过1000ms未跳动,
/// 喂狗任务将停止喂狗, RWDT超时后复位系统。
#[embassy_executor::task]
async fn worker_task(hb: &'static Heartbeat) {
    let mut round = 0u32;
    loop {
        hb.beat();
        round += 1;
        println!("worker alive, round {}", round);
        Timer::after(Duration::from_millis(200)).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let hb = SUPERVISOR
        .register(1_000)
        .expect("supervisor slot available");
    spawner.spawn(worker_task(hb)).ok();

    // RWDT Stage0: 3000ms内未收到喂狗则复位系统
    let mut wdt = Watchdog::enable(peripherals.LPWR, 3_000);
    println!("watchdog armed, supervised feed every 500ms");

    loop {
        Timer::after(Duration::from_millis(500)).await;
        let fed = supervised_feed(&mut wdt, &SUPERVISOR);
        println!("feed cycle: {}", if fed { "fed" } else { "task stalled, withholding" });
    }
}
