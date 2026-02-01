use std::env;

fn main() {
    // esp-hal 1.0 已修复 App Descriptor 和链接脚本问题
    // 不再需要手动修补 rodata.x 和 esp32s3.x
    
    // 配置 PSRAM 模式 (ESP32-S3-N16R8 使用 Octal PSRAM)
    println!("cargo:rustc-env=ESP_HAL_CONFIG_PSRAM_MODE=octal");
    
    // 告诉 cargo 在 build.rs 变化时重新运行
    println!("cargo:rerun-if-changed=build.rs");
    
    // 添加 ld 目录到链接路径（如果有自定义链接脚本）
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-link-search={}/ld", manifest_dir);
}
