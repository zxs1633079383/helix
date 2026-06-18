/// build.rs — 生成 helix.hpp（C++ 头文件，供 C++ host 包含）
///
/// cbindgen 扫描 src/，提取所有 `repr(C)` 类型和 `#[no_mangle] pub extern "C"` 函数，
/// 输出 C++ 头（pragma_once，HelixHandle 作不透明前向声明）到 OUT_DIR/helix.hpp。
///
/// ## 触发条件
///
/// cargo 在以下情况重新运行 build.rs：
///   - src/ 下任何 .rs 文件变化
///   - cbindgen.toml 变化
///
/// ## helix.hpp 位置
///
/// OUT_DIR/helix.hpp（cargo 编译输出目录，通常为
/// `target/<profile>/build/helix-driver-ffi-<hash>/out/helix.hpp`）。
/// C++ 构建系统通过 `$(cargo metadata)` 或硬链接到已知路径取得此文件。
fn main() {
    // 重新运行触发条件
    println!("cargo:rerun-if-changed=src/");
    println!("cargo:rerun-if-changed=cbindgen.toml");

    let crate_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set");
    let out_dir = std::env::var("OUT_DIR")
        .expect("OUT_DIR not set");
    let out_file = std::path::PathBuf::from(&out_dir).join("helix.hpp");

    let config = cbindgen::Config::from_file(
        std::path::PathBuf::from(&crate_dir).join("cbindgen.toml"),
    )
    .expect("failed to read cbindgen.toml");

    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
        .expect("cbindgen failed to generate helix.hpp")
        .write_to_file(&out_file);

    // 同时输出到 crate 根目录，方便开发时直接引用（非构建系统时使用）
    // OUT_DIR 内的文件是正式产物；crate 根目录版本仅供开发参考。
    let dev_file = std::path::PathBuf::from(&crate_dir).join("helix.hpp");
    cbindgen::Builder::new()
        .with_crate(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .with_config(
            cbindgen::Config::from_file(
                std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
                    .join("cbindgen.toml"),
            )
            .expect("failed to read cbindgen.toml for dev output"),
        )
        .generate()
        .expect("cbindgen dev output failed")
        .write_to_file(&dev_file);
}
