use std::env;

fn main() {

    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-link-search={}/ld", manifest_dir);

    // defmt的链接片段(defmt.x提供_defmt_panic等PROVIDE符号),仅在log-defmt特性下注入
    if env::var("CARGO_FEATURE_LOG_DEFMT").is_ok() {
        println!("cargo:rustc-link-arg=-Tdefmt.x");
    }
}
