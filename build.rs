use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    // 获取 OUT_DIR 的父目录（esp-hal 的 output 目录）
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    
    // 查找 esp-hal 的 build output 目录
    // 格式通常是: target/<target>/<profile>/build/esp-hal-<hash>/out
    let build_dir = out_dir.parent().unwrap().parent().unwrap(); // 到 build/ 目录
    
    // 遍历查找 esp-hal 目录
    if let Ok(entries) = fs::read_dir(build_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap().to_string_lossy();
                if name.starts_with("esp-hal-") && !name.contains("embassy") && !name.contains("procmacros") {
                    let out_path = path.join("out");
                    
                    // 修补 rodata.x - 添加 .flash.appdesc section
                    // 关键：.flash.appdesc 必须在 RODATA 的最开头
                    // 这样 .data 的 LMA (AT > RODATA) 会自动跟在 appdesc 后面
                    let rodata_path = out_path.join("rodata.x");
                    if rodata_path.exists() {
                        if let Ok(content) = fs::read_to_string(&rodata_path) {
                            if !content.contains(".flash.appdesc") {
                                // 创建新的 rodata.x 内容
                                // 注意：section 顺序很重要！
                                let new_content = r#"
SECTIONS {
  /* For ESP App Description, must be placed first in DROM segment */
  /* This fix backports esp-hal PR #4745 for esp-hal 0.23 */
  /* The appdesc MUST be at the very beginning of RODATA region */
  /* so that .data's LMA (AT > RODATA) comes after it */
  .flash.appdesc : ALIGN(4)
  {
      KEEP(*(.flash.appdesc));
      KEEP(*(.flash.appdesc.*));
  } > RODATA

  .rodata : ALIGN(4)
  {
    . = ALIGN (4);
    _rodata_start = ABSOLUTE(.);
    *(.rodata .rodata.*)
    *(.srodata .srodata.*)
    . = ALIGN(4);
    _rodata_end = ABSOLUTE(.);
  } > RODATA

  .rodata.wifi : ALIGN(4)
  {
    . = ALIGN(4);
    *( .rodata_wlog_*.* )
    . = ALIGN(4);
  } > RODATA
}
"#;
                                if let Err(e) = fs::write(&rodata_path, new_content) {
                                    println!("cargo:warning=Failed to patch rodata.x: {}", e);
                                } else {
                                    println!("cargo:warning=Patched rodata.x to include .flash.appdesc section");
                                }
                            }
                        }
                    }
                    
                    // 修补 esp32s3.x - 调整 section 包含顺序
                    // 让 rodata.x 在 rwdata.x 之前被处理
                    let esp32s3_path = out_path.join("esp32s3.x");
                    if esp32s3_path.exists() {
                        if let Ok(content) = fs::read_to_string(&esp32s3_path) {
                            // 查找并调整 INCLUDE 顺序
                            // 原顺序: rwtext.x, text.x, rwdata.x, rodata.x
                            // 新顺序: rodata.x, rwtext.x, text.x, rwdata.x
                            if content.contains("INCLUDE \"rwdata.x\"") && 
                               content.contains("INCLUDE \"rodata.x\"") &&
                               !content.contains("/* PATCHED */") {
                                let new_content = content
                                    .replace(
                                        "INCLUDE \"rwtext.x\"\nINCLUDE \"text.x\"\nINCLUDE \"rwdata.x\"\nINCLUDE \"rodata.x\"",
                                        "/* PATCHED */\nINCLUDE \"rodata.x\"\nINCLUDE \"rwtext.x\"\nINCLUDE \"text.x\"\nINCLUDE \"rwdata.x\""
                                    );
                                if let Err(e) = fs::write(&esp32s3_path, &new_content) {
                                    println!("cargo:warning=Failed to patch esp32s3.x: {}", e);
                                } else {
                                    println!("cargo:warning=Patched esp32s3.x to reorder section includes");
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    // 告诉 cargo 在 ld 目录变化时重新运行
    println!("cargo:rerun-if-changed=ld/");
    println!("cargo:rerun-if-changed=build.rs");
    
    // 添加我们的 ld 目录到链接路径（备用）
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-link-search={}/ld", manifest_dir);
}
