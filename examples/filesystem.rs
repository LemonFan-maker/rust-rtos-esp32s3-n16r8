#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::timer::timg::TimerGroup;

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

const LFS_BLOCK_COUNT: usize = 256;

const DEMO_TEXT: &[u8] = b"RustRTOS littlefs2 on ESP32-S3 internal SPI flash - round 1";

#[embassy_executor::task]
async fn fs_demo_task(flash_periph: esp_hal::peripherals::FLASH<'static>) {
    println!("Filesystem Demo Task Started");

    println!("partition table");

    use rustrtos::fs::{DataSubType, PartitionTable, PartitionType};

    let mut table = PartitionTable::new();

    let _ = table.add_partition(
        "nvs",
        PartitionType::Data,
        DataSubType::Nvs.as_u8(),
        0x9000,
        0x5000,
    );

    let _ = table.add_partition(
        "storage",
        PartitionType::Data,
        DataSubType::LittleFs.as_u8(),
        0xC00000,
        0x400000,
    );

    println!("Created partition table with {} partitions:", table.len());
    for partition in table.partitions() {
        println!(
            "  {}: offset=0x{:X}, size=0x{:X} ({} KB)",
            partition.label.as_str(),
            partition.offset,
            partition.size,
            partition.size / 1024
        );
    }

    if let Some(storage) = table.find_by_label("storage") {
        println!(
            "Found storage partition: offset=0x{:X}, {} MB, is_littlefs={}",
            storage.offset,
            storage.size / 1024 / 1024,
            storage.is_littlefs()
        );
    }

    println!("flash storage");

    use rustrtos::fs::{FileSystem, FlashConfig, FlashStorage, LfsDevice, MountPolicy, OpenOptions, SeekFrom};

    let config = FlashConfig {
        partition_offset: 0xC00000,
        partition_size: 0x400000,
        ..FlashConfig::default()
    };
    println!(
        "storage window: offset=0x{:X}, size=0x{:X} (blocks={})",
        config.partition_offset, config.partition_size, LFS_BLOCK_COUNT
    );
    println!("NOTE: erase/write below are REAL operations on this window");

    let storage = FlashStorage::new(flash_periph, config);
    let mut device = match LfsDevice::<LFS_BLOCK_COUNT>::new(storage) {
        Ok(d) => d,
        Err(e) => {
            println!("storage window validation FAILED: {} (check flash size / offset)", e);
            return;
        }
    };
    println!("flash capacity: {} MB", device.flash().capacity() / 1024 / 1024);

    println!("littlefs mount");

    let mut alloc = FileSystem::allocate();
    let (fs, _formatted) = match FileSystem::mount_with(
        &mut alloc,
        &mut device,
        MountPolicy::FormatIfAbsent,
    ) {
        Ok((fs, formatted)) => {
            println!("mount OK (formatted_now={})", formatted);
            (fs, formatted)
        }
        Err(e) => {
            println!("mount/format FAILED: {}", e);
            if let Some(se) = device.take_last_error() {
                println!("  underlying storage error: {}", se);
            }
            return;
        }
    };

    println!("write/verify/seek");

    match fs.write_file("/hello.txt", DEMO_TEXT) {
        Ok(()) => println!("wrote /hello.txt ({} bytes)", DEMO_TEXT.len()),
        Err(e) => {
            println!("write_file FAILED: {}", e);
            return;
        }
    }

    let mut readback = [0u8; 64];
    match fs.read_file("/hello.txt", &mut readback) {
        Ok(n) => {
            let ok = &readback[..n] == DEMO_TEXT;
            println!("read back {} bytes, verify={}", n, if ok { "OK" } else { "MISMATCH" });
        }
        Err(e) => println!("read_file FAILED: {}", e),
    }

    match fs.read_file_at("/hello.txt", 16, &mut readback[..16]) {
        Ok(n) => println!("seek+read at offset 16: {} bytes, first byte=0x{:02X}", n, readback[0]),
        Err(e) => println!("read_file_at FAILED: {}", e),
    }

    let _ = fs.append_file("/hello.txt", b" +appended");
    match fs.file_size("/hello.txt") {
        Ok(sz) => println!("/hello.txt size after append: {}", sz),
        Err(e) => println!("file_size FAILED: {}", e),
    }

    match fs.with_file(
        "/hello.txt",
        OpenOptions::read_write(),
        |file| {
            let head = file.seek(SeekFrom::Start(0))?;
            let end = file.seek(SeekFrom::End(0))?;
            file.sync()?;
            println!("with_file handle: head={}, end={}", head, end);
            Ok(())
        },
    ) {
        Ok(()) => {}
        Err(e) => println!("with_file FAILED: {}", e),
    }

    println!("directories");

    if let Err(e) = fs.create_dir_all("/logs/2026") {
        println!("create_dir_all FAILED: {}", e);
    }
    if let Err(e) = fs.write_file("/logs/2026/day.bin", &[0xA5u8; 32]) {
        println!("write /logs FAILED: {}", e);
    }

    let mut entries = heapless::Vec::<_, 8>::new();
    match fs.read_dir_collect("/logs/2026", &mut entries) {
        Ok(n) => {
            println!("/logs/2026 contains {} entries:", n);
            for m in entries.iter() {
                println!("  [{}] {} ({} bytes)", if m.is_dir() { 'd' } else { 'f' }, m.name, m.size);
            }
        }
        Err(e) => println!("read_dir FAILED: {}", e),
    }

    match fs.metadata("/logs") {
        Ok(m) => println!("metadata /logs: is_dir={}, name={}", m.is_dir(), m.name),
        Err(e) => println!("metadata FAILED: {}", e),
    }

    println!("rename/delete");

    match fs.rename("/hello.txt", "/hello2.txt") {
        Ok(()) => println!("renamed /hello.txt -> /hello2.txt"),
        Err(e) => println!("rename FAILED: {}", e),
    }
    match fs.exists("/hello.txt") {
        Ok(exists) => println!("/hello.txt exists after rename: {}", exists),
        Err(e) => println!("exists FAILED: {}", e),
    }
    match fs.remove("/hello2.txt") {
        Ok(()) => println!("removed /hello2.txt"),
        Err(e) => println!("remove FAILED: {}", e),
    }
    let _ = fs.remove("/logs/2026/day.bin");
    let _ = fs.remove_dir("/logs/2026");
    let _ = fs.remove_dir("/logs");

    match fs.free_blocks() {
        Ok(free) => println!(
            "free blocks: {}/{} (used {}, total {} KB)",
            free,
            fs.total_blocks(),
            fs.used_blocks().unwrap_or(0),
            fs.total_bytes() / 1024
        ),
        Err(e) => println!("free_blocks FAILED: {}", e),
    }

    match fs.unmount() {
        Ok(()) => println!("unmounted"),
        Err(e) => println!("unmount FAILED: {}", e),
    }

    println!("filesystem demo done");
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("Filesystem Example");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    spawner.spawn(fs_demo_task(peripherals.FLASH)).ok();

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
