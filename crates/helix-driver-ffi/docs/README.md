# helix-driver-ffi docs 入口

本目录是移动端/Flutter/C++/QuickJS 侧消费 `helix-driver-ffi` 的模块级文档入口。这里不承接移动端 bridge 实现；bridge / QuickJS binding / Flutter 工程落在 `mobile-qucik-c++` 或具体宿主项目。

## 当前事实入口

| 文档 | 角色 |
|---|---|
| `移动端Command-Query-Event契约SPEC.md` | 移动端 command/query/event 契约入口。新增移动端能力先改这里，再同步 mobile 项目接口文档 |
| `移动端QuickJS最后一公里接入SPEC.md` | QuickJS 最后一公里接入边界说明；实现不放回 helix |
| `多端联调测试指南.md` | PC/mobile/driver-ffi 联调测试入口 |
| `flutter-demo-pointer.md` | Flutter demo / pin / 接入指针 |
| `design/README.md` | FFI crate 内部设计图索引，源码真源仍是 `src/*.rs` |
| `design/cpp-direct/README.md` | C++ 直连模型与边界面设计 |

## 历史/支撑材料

| 文档 | 说明 |
|---|---|
| `SPEC-full-takeover.md` | 接管路线历史 spec，引用前需核对当前 src / gate |
| `移动端QuickJS外部联调实现计划.md` | 外部联调阶段计划；不代表当前 mobile 项目实现完成态 |
| `移动端QuickJS接入最终需求与验收标准.md` | 验收标准沉淀；当前执行以 mobile 项目 gate / collector 为准 |
| `PERF-INVARIANTS.md` | 性能/边界不变量 |

## 熵减规则

- `helix-driver-ffi` 只维护 ABI、driver、契约与模块内设计，不维护移动端 C++ bridge 或 Flutter UI。
- 当前 green 不能从历史计划、worker report、demo 文档推出，必须来自 fresh gate / reviewer / collector。
- 修改移动端可见 command/event 后，必须同步：
  - 本目录 `移动端Command-Query-Event契约SPEC.md`
  - `/System/Volumes/Data/workspace/c/mobile-qucik-c++/docs/移动端UC指令与事件集合接口文档.md`
  - `/System/Volumes/Data/workspace/c/mobile-qucik-c++/docs/接口文档/移动端Command与Event字段契约.md`
