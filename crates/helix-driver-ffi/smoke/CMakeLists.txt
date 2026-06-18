# helix C++ 直连冒烟（cpp → rust → cpp）的 cmake/ctest 自动化 harness。
#
# 前置：`cargo build -p helix-driver-ffi --release` 已产出 cdylib + helix.hpp
#       （由 scripts/cpp-smoke.sh 自动先跑；手动跑也可）。
# 这测的是「真 C++ link cdylib + #include helix.hpp」的 C ABI 端到端，cargo test 测不到这层。
cmake_minimum_required(VERSION 3.16)
project(helix_cpp_smoke CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# 仓库根 target/release（cargo 产物落点）+ helix.hpp 目录（crate 根提交副本）。
set(HELIX_TARGET_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../../target/release"
    CACHE PATH "cargo target/release dir (libhelix_driver_ffi.dylib 落点)")
set(HELIX_HPP_DIR "${CMAKE_CURRENT_SOURCE_DIR}/.."
    CACHE PATH "helix.hpp 目录（crates/helix-driver-ffi/helix.hpp）")

add_executable(helix_smoke smoke.cpp)
target_include_directories(helix_smoke PRIVATE "${HELIX_HPP_DIR}")
target_link_directories(helix_smoke PRIVATE "${HELIX_TARGET_DIR}")
target_link_libraries(helix_smoke PRIVATE helix_driver_ffi)
# 运行期能找到 dylib（macOS rpath / Linux RUNPATH）。
set_target_properties(helix_smoke PROPERTIES BUILD_RPATH "${HELIX_TARGET_DIR}")

enable_testing()
add_test(NAME helix_cpp_direct_smoke COMMAND helix_smoke)
set_tests_properties(helix_cpp_direct_smoke PROPERTIES
    ENVIRONMENT "DYLD_LIBRARY_PATH=${HELIX_TARGET_DIR};LD_LIBRARY_PATH=${HELIX_TARGET_DIR}"
    TIMEOUT 30)
