---
id: HX-C006
title: FFI 边界铁律（不许 unwind 跨 extern C / 单一 owner / 同步调用包 spawn_blocking）
status: active
severity: blocker
scope: helix
created: 2026-06-08
source: 2026-06-08 review（MAJ-4）+ FFI 规约
recurrence_count: 1
trigger_paths:
  - "crates/helix-driver-ffi/**"
trigger_keywords:
  - "extern \"C\""
  - "no_mangle"
  - "Box::into_raw"
  - "Box::from_raw"
  - "catch_unwind"
  - "spawn_blocking"
  - "panic = \"abort\""
related:
  - HX-C001
graduates_to:
  - ~/.claude/rules/rust/runtime-agnostic-core.md
---

# HX-C006 — FFI 边界铁律

## §1 触发场景
- 写 helix-driver-ffi 的任何 `#[no_mangle] extern "C"` 导出
- Rust 对象跨 C ABI 交给 Swift/Kotlin（opaque pointer）
- 在 FFI driver 里调用同步阻塞 API（rusqlite C 调用）

## §2 背景（why）
**MAJ-4（本仓 review 发现）**：FFI driver 的同步 SQLite C 调用**没包 `spawn_blocking`**，会阻塞 tokio worker 线程（当前是 `todo!()`，FFI 落地前必补）。

FFI 通用铁律（违反 = UB / 崩溃 / 内存损坏）：
- panic **unwind 跨 `extern "C"` 是 UB** → 防御策略**二选一不混用**（2026-06-10 勘误：原表述「catch_unwind + panic=abort」自相矛盾——crate 级 `panic="abort"` 使 `catch_unwind` 静默失效，panic 直接 abort 不走 unwind）：
  - **生产 cdylib**：`panic="abort"`（profile 级）——panic 即 abort，无 unwind 可跨边界，无需也不能 catch_unwind
  - **dev/test**（panic=unwind 默认）：每个 `extern "C"` 导出包 `catch_unwind` 转错误码
- opaque pointer **单一 owner**：`Box::into_raw` 交出一次、`Box::from_raw` 收回一次；double-free 即堆损坏
- 错误用 error code / out-param，不靠异常

## §3 Required / Forbidden
```rust
// ✅ 单一 owner 生命周期
#[no_mangle] pub extern "C" fn helix_new() -> *mut Core { Box::into_raw(Box::new(Core::default())) }
#[no_mangle] pub extern "C" fn helix_free(p: *mut Core) {
    if !p.is_null() { unsafe { drop(Box::from_raw(p)); } }   // 恰好一次
}
// ✅ dev/test（panic=unwind）：导出包 catch_unwind，不让 panic 越界
//    生产 cdylib（panic=abort）：catch_unwind 无效，靠 abort 语义本身保证不跨边界
#[no_mangle] pub extern "C" fn helix_step(p: *mut Core) -> i32 {
    std::panic::catch_unwind(|| { /* ... */ 0 }).unwrap_or(-1)
}
// ✅ 同步阻塞调用包 spawn_blocking（不阻塞 async runtime）
let row = tokio::task::spawn_blocking(move || conn.query_row(...)).await?;
```
```rust
// ❌ 同步 SQLite 直接在 async 里调 → 阻塞 tokio worker（MAJ-4）
let row = conn.query_row(...);            // 阻塞整个 runtime
// ❌ panic 越过 extern C（UB）
#[no_mangle] pub extern "C" fn bad() { something().unwrap(); }  // 无 catch_unwind
// ❌ double-free
let b = Box::from_raw(p); /* ... */ Box::from_raw(p);          // 第二次 = 堆损坏
```
release cdylib profile 必须 `panic = "abort"`（设在最终 binary/cdylib profile，不设 library crate 级——否则 dev 的 catch_unwind 也失效）。

## §4 Verification
```bash
# 每个 extern "C" 导出应有 catch_unwind 包裹（或证明不会 panic）
grep -rn 'extern "C"' crates/helix-driver-ffi/src | wc -l
grep -rn 'catch_unwind' crates/helix-driver-ffi/src | wc -l   # 应覆盖会 panic 的导出
# 同步阻塞调用应在 spawn_blocking 内
grep -rn 'query_row\|execute\|conn\.' crates/helix-driver-ffi/src   # 检查是否包 spawn_blocking
grep -rn 'panic = "abort"' Cargo.toml crates/helix-driver-ffi/Cargo.toml
```
> 残留：当前 spawn_blocking 包装是 `todo!()`，FFI driver 落地（接 Swift/Kotlin 冒烟）前必须实现。

## §5 Recurrence Log
| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-08 | review MAJ-4 | FFI 同步 SQLite 未包 spawn_blocking，阻塞 tokio worker | 同步调用混进 async runtime；已文档化待实现 |

## §6 关联
- 上游：rules/runtime-agnostic-core.md §7
- 兄弟：HX-C001
- 残留风险：docs/harness/decisions.md（P1：spawn_blocking 实现）

## §7 历史与演进
- drafting → active：2026-06-08，MAJ-4 文档化 + spawn_blocking 占位后确立（实现待补）。
- 2026-06-10 勘误（四专家 review，待办 C4）：原「catch_unwind + panic=abort」并列表述自相矛盾，改为二选一策略；同步修正项目 CLAUDE.md §2 与 .claude/rules/runtime-agnostic-core.md §7。
