/// build.rs — 生成 helix.hpp（C++ 头文件，供 C++ host 包含）
///
/// cbindgen 扫描 src/，提取所有 `repr(C)` 类型和 `#[no_mangle] pub extern "C"` 函数，
/// 输出 C++ 头（pragma_once，HelixHandle 作不透明前向声明）到 OUT_DIR/helix.hpp。
///
/// ## 触发条件
///
/// cargo 在以下情况重新运行 build.rs（收窄到实际影响导出的文件，不监听整个 src/，
/// 避免无关 .rs 改动触发重跑 + 脏化已跟踪的 helix.hpp）：
///   - 影响 C ABI 导出 / repr(C) 类型的源文件（api.rs / vtable.rs / lib.rs）
///   - cbindgen.toml 变化
///
/// ## helix.hpp 位置
///
/// OUT_DIR/helix.hpp（cargo 编译输出目录，通常为
/// `target/<profile>/build/helix-driver-ffi-<hash>/out/helix.hpp`）。
/// C++ 构建系统通过 `$(cargo metadata)` 或硬链接到已知路径取得此文件。
fn main() {
    // 重新运行触发条件：只监听实际决定 C ABI 表面的文件（导出函数 / repr(C) 类型 /
    // 模块根），不监听整个 src/——否则任何内部实现改动都重跑 cbindgen 并脏化 helix.hpp。
    println!("cargo:rerun-if-changed=src/api.rs");
    println!("cargo:rerun-if-changed=src/vtable.rs");
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");

    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let out_file = std::path::PathBuf::from(&out_dir).join("helix.hpp");

    let config =
        cbindgen::Config::from_file(std::path::PathBuf::from(&crate_dir).join("cbindgen.toml"))
            .expect("failed to read cbindgen.toml");

    let generated = cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
        .expect("cbindgen failed to generate helix.hpp");

    // OUT_DIR 内是正式产物，每次都写。
    generated.write_to_file(&out_file);

    // crate 根目录副本（git 已跟踪，供 C++ 侧引用）：仅在内容**确有差异**时才写，
    // 避免无谓改写脏化工作树（rerun 时若 ABI 未变，新产物与现有一致 → 不动文件）。
    // 比对源用刚写出的 OUT_DIR 产物（即本次生成结果），不重新 generate。
    let new_bytes = std::fs::read(&out_file).expect("failed to read generated OUT_DIR helix.hpp");
    let dev_file = std::path::PathBuf::from(&crate_dir).join("helix.hpp");
    let differs = match std::fs::read(&dev_file) {
        Ok(existing) => existing != new_bytes,
        Err(_) => true, // 不存在 / 读失败 → 写出
    };
    if differs {
        std::fs::write(&dev_file, &new_bytes).expect("failed to write crate-root helix.hpp");
    }
}
