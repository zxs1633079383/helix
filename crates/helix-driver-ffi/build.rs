/// build.rs — 生成 **两份** cbindgen 头（G-8 双配置）：
///   - `helix.hpp`（C++，cbindgen.toml）：C++ 直连 smoke（scripts/cpp-smoke.sh）。
///   - `helix.h` （C  ，cbindgen-c.toml）：Dart `ffigen` 走 libclang 解析的干净 C 头
///     （C++ 头带 `<ostream>/<new>` 等污染让 ffigen 困惑，故单出一份 C 头）。
///
/// cbindgen 扫描 src/，提取所有 `repr(C)` 类型和 `#[no_mangle] pub extern "C"` 函数。
///
/// ## 触发条件
///
/// cargo 只在以下变化时重跑（收窄到实际影响导出的文件，不监听整个 src/，避免无关 .rs 改动脏化头）：
///   - 影响 C ABI 导出 / repr(C) 类型的源文件（api.rs / api/action.rs / vtable.rs / lib.rs）
///   - 两份 cbindgen 配置（cbindgen.toml / cbindgen-c.toml）
///
/// ## 头文件位置
///
/// OUT_DIR/<name>（cargo 编译输出目录）+ crate 根副本（git 跟踪，供 C++/ffigen 引用）。
fn main() {
    println!("cargo:rerun-if-changed=src/api.rs");
    println!("cargo:rerun-if-changed=src/api/action.rs");
    println!("cargo:rerun-if-changed=src/vtable.rs");
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-changed=cbindgen-c.toml");

    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");

    // C++ 头（cpp-smoke 用）+ C 头（ffigen 用），同一 ABI 表面两种语言投影。
    generate_header(&crate_dir, &out_dir, "cbindgen.toml", "helix.hpp");
    generate_header(&crate_dir, &out_dir, "cbindgen-c.toml", "helix.h");
}

/// 用 `config_name` 生成头到 OUT_DIR/`out_name` + crate 根副本（仅内容有差异时改写，避免脏化工作树）。
fn generate_header(crate_dir: &str, out_dir: &str, config_name: &str, out_name: &str) {
    let config = cbindgen::Config::from_file(std::path::PathBuf::from(crate_dir).join(config_name))
        .unwrap_or_else(|e| panic!("failed to read {config_name}: {e}"));

    let generated = cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(config)
        .generate()
        .unwrap_or_else(|e| panic!("cbindgen failed to generate {out_name}: {e}"));

    // OUT_DIR 内是正式产物，每次都写。
    let out_file = std::path::PathBuf::from(out_dir).join(out_name);
    generated.write_to_file(&out_file);

    // crate 根副本（git 跟踪）：仅在内容**确有差异**时才写（rerun 时若 ABI 未变 → 不动文件）。
    let new_bytes = std::fs::read(&out_file)
        .unwrap_or_else(|e| panic!("failed to read OUT_DIR {out_name}: {e}"));
    let dev_file = std::path::PathBuf::from(crate_dir).join(out_name);
    let differs = match std::fs::read(&dev_file) {
        Ok(existing) => existing != new_bytes,
        Err(_) => true, // 不存在 / 读失败 → 写出
    };
    if differs {
        std::fs::write(&dev_file, &new_bytes)
            .unwrap_or_else(|e| panic!("failed to write crate-root {out_name}: {e}"));
    }
}
