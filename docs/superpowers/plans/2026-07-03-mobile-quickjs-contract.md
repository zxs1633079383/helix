# 移动端 QuickJS 契约实现计划

> **给执行 agent：** 必须使用 `superpowers:subagent-driven-development`（推荐）或 `superpowers:executing-plans` 按任务逐步执行本计划。步骤使用复选框（`- [ ]`）便于跟踪。

**目标：** 在 `helix` 中实现 Rust 侧移动端命令 / 查询 / 事件契约，并把 C++ 桥接、QuickJS、平台产物交给独立移动端集成项目继续实现。

**架构：** `helix` 负责 Rust 业务契约、稳定 C ABI、`helix.h` / `helix.hpp`、原生库构建、能力集 JSON、Rust/FFI 契约测试。移动端集成项目负责 C++ `CoreBridge`、QuickJS 原生注册、JS 封装层、Android/iOS 构建产物、C++/QuickJS 冒烟。本仓不新增移动端 C++ 桥接源码、QuickJS 封装层源码或移动端编译产物。

**技术栈：** Rust workspace（`helix-core`、`helix-im`、`helix-driver-ffi`、`helix-driver-host`）、通过 cbindgen 生成 C ABI 头文件（`helix.h` / `helix.hpp`）、通过 `serde_json` 处理 JSON 载荷、事件批格式来自 `crates/helix-driver-ffi/src/batch.rs`。

## 全局约束

- 不为每个 IM 用例新增一个 C ABI 符号；公开业务面保持 `name + JSON payload`。
- 不在 `helix` 仓创建移动端 C++ 桥接文件、QuickJS 封装层文件、QuickJS 冒烟文件或移动端编译产物。
- C++ 桥接 / QuickJS / Android / iOS 集成必须在独立移动端集成项目实现。
- 不把 `im:post:send-failed` 加成移动端 P0。发送链路保持：`im:post:sending` 乐观行、WS `post` echo -> `im:post:received`、超时 -> 持久化 `send_status='unsend'`。
- 不从 JS catch 或错误字符串推断消息失败状态。
- 不把 IM 表 / schema 知识放进 `helix-driver-ffi`；查询计划和响应整形属于 `helix-im`。
- 不让 `helix-core` 依赖 tokio/sqlx/reqwest/web-sys/tauri。
- 事件仍通过 `on_domain_events_batch` 批量交付；外部项目里的 C++ 在回调期间拷贝借出的 buffer。
- 移动端封装层可以暴露 camelCase 载荷，但 Rust 边界必须先归一化，再交给既有 helix-im 命令处理器。
- 如果修改 `AGENTS.md` 或 `CLAUDE.md`，两者必须保持镜像一致。

---

## 文件结构

在 `helix` 中新增：

- `crates/helix-im/src/mobile_contract.rs`  
  移动端公开命令别名、事件名、功能标记、能力集 JSON、载荷归一化辅助函数的单一真源。

- `crates/helix-im/src/mobile_query.rs`  
  IM 拥有的本地同步 FFI 查询计划 / 响应整形边界，避免存储表名进入 `helix-driver-ffi`。

- `crates/helix-driver-ffi/tests/mobile_contract_test.rs`  
  Rust FFI 契约测试，覆盖能力集 JSON、命令别名归一、query 返回 / 释放行为、发送状态事件交付。

在 `helix` 中修改：

- `crates/helix-im/src/lib.rs`  
  导出 `mobile_contract` 和 `mobile_query`。

- `crates/helix-im/src/outbound/registry.rs`  
  暴露稳定排序的 outbound 命令名，供契约快照测试使用。

- `crates/helix-im/src/query/core.rs`  
  暴露稳定的 query 命令名，供契约快照测试和桥接侧校验使用。

- `crates/helix-driver-ffi/src/api.rs`  
  入队前归一移动端命令名 / 载荷，实现元信息 / 本地 query 处理，并保持不支持名称的稳定行为。

- `crates/helix-driver-ffi/docs/移动端Command-Query-Event契约SPEC.md`  
  保持契约和仓库边界最新。

- `crates/helix-driver-ffi/docs/移动端QuickJS接入最终需求与验收标准.md`  
  保持职责边界最新。

不要在 `helix` 中创建：

- `crates/helix-driver-ffi/smoke/mobile_bridge.hpp`
- `crates/helix-driver-ffi/smoke/mobile_bridge_smoke.cpp`
- `crates/helix-driver-ffi/smoke/mobile_core_bridge.js`
- `crates/helix-driver-ffi/smoke/mobile_core_bridge_smoke.mjs`
- 任何 Android / iOS / QuickJS 编译产物

---

### 任务 1：在 helix-im 冻结移动端契约

**文件：**
- 新增：`crates/helix-im/src/mobile_contract.rs`
- 修改：`crates/helix-im/src/lib.rs`
- 修改：`crates/helix-im/src/outbound/registry.rs`
- 修改：`crates/helix-im/src/query/core.rs`
- 测试：`crates/helix-im/src/mobile_contract.rs` 内联测试

**接口：**
- 消费：既有 `helix_im::outbound::canonical_command_name`、`helix_im::commands::outbound_command_count`、`helix_im::query::is_query`
- 产出：
  - `helix_im::mobile_contract::CONTRACT_VERSION: u32`
  - `helix_im::mobile_contract::BUS_CHANNEL: &'static str`
  - `helix_im::mobile_contract::command_alias(public: &str) -> Option<&'static str>`
  - `helix_im::mobile_contract::normalize_command_payload(public: &str, payload: &[u8]) -> Result<Vec<u8>, ImError>`
  - `helix_im::mobile_contract::capabilities_json() -> Vec<u8>`
  - `helix_im::outbound::outbound_command_names() -> Vec<&'static str>`
  - `helix_im::query::query_command_names() -> &'static [&'static str]`

- [ ] **步骤 1：写失败测试**

先创建 `crates/helix-im/src/mobile_contract.rs`，写入这些测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_alias_normalizes_to_existing_core_command_and_snake_payload() {
        let input = br#"{"channelId":"ch1","temporaryId":"tmp1","text":"hi","type":"TEXT"}"#;
        let normalized = normalize_command_payload("im_send", input).expect("normalize");
        let v: serde_json::Value = serde_json::from_slice(&normalized).unwrap();
        assert_eq!(command_alias("im_send"), Some("im_send_message"));
        assert_eq!(v["channel_id"], "ch1");
        assert_eq!(v["temporary_id"], "tmp1");
        assert_eq!(v["text"], "hi");
        assert_eq!(v["type"], "TEXT");
        assert!(v.get("channelId").is_none());
        assert!(v.get("temporaryId").is_none());
    }

    #[test]
    fn capabilities_do_not_advertise_send_failed_as_p0() {
        let caps: serde_json::Value = serde_json::from_slice(&capabilities_json()).unwrap();
        assert_eq!(caps["abi"], 1);
        assert_eq!(caps["bus"], "im:__bus__");
        assert_eq!(caps["features"]["pendingSendTimeoutUnsend"], true);
        assert!(caps["features"].get("sendFailedProjection").is_none());
        let events = caps["events"].as_array().unwrap();
        assert!(events.iter().any(|e| e == "im:post:sending"));
        assert!(events.iter().any(|e| e == "im:post:received"));
        assert!(!events.iter().any(|e| e == "im:post:send-failed"));
    }
}
```

- [ ] **步骤 2：运行测试，确认先失败**

运行：

```bash
cargo test -p helix-im mobile_contract -- --nocapture
```

预期：失败，因为 `mobile_contract` 函数尚不存在。

- [ ] **步骤 3：暴露稳定的命令 / 查询名来源**

在 `crates/helix-im/src/outbound/registry.rs` 中新增：

```rust
pub fn outbound_command_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = registry()
        .map(|m| m.keys().copied().collect())
        .unwrap_or_default();
    names.sort_unstable();
    names
}
```

在 `crates/helix-im/src/query/core.rs` 中靠近 `is_query` 的位置新增：

```rust
pub const QUERY_COMMAND_NAMES: &[&str] = &[
    "im_query_messages_by_channel",
    "im_delete_all_dialogs",
    "im_query_dialog_list",
    crate::older_context::LOAD_OLDER_CONTEXT,
];

pub fn query_command_names() -> &'static [&'static str] {
    QUERY_COMMAND_NAMES
}

pub fn is_query(name: &str) -> bool {
    QUERY_COMMAND_NAMES.contains(&name)
}
```

移除旧的 `is_query` 实现。

- [ ] **步骤 4：实现移动端契约模块**

将 `crates/helix-im/src/mobile_contract.rs` 填成：

```rust
use serde_json::{json, Map, Value};

use crate::error::ImError;

pub const CONTRACT_VERSION: u32 = 1;
pub const BUS_CHANNEL: &str = "im:__bus__";

pub struct CommandAlias {
    pub public: &'static str,
    pub core: &'static str,
}

pub const COMMAND_ALIASES: &[CommandAlias] = &[
    CommandAlias {
        public: "im_send",
        core: "im_send_message",
    },
];

pub const PUBLIC_EVENTS: &[&str] = &[
    "im:post:sending",
    "im:post:received",
    "im:post:updated",
    "im:post:read",
    "im:channel:read_echo",
    "im:post:deleted",
    "im:post:batch-updated",
    "im:messages:query_result",
    "im:messages:older_loaded",
    "im:channels:projection",
    "im:channels:loaded",
    "im:channel:increment",
    "im:channel:update",
    "im:channel:update-by-post",
    "im:channel:created",
    "im:channel:closed",
    "im:channel:schedule-created",
    "im:channel:schedule-canceled",
    "im:channel:member-updated",
    "im:channel:memberNickname",
    "im:channel:members",
    "im:channel:replies",
    "im:read:result",
    "im:todo:updated",
    "im:health:result",
    "im:net:reconnect_requested",
    "im:http:unauthorized",
    "im:net:offline",
];

pub fn command_alias(public: &str) -> Option<&'static str> {
    COMMAND_ALIASES
        .iter()
        .find(|alias| alias.public == public)
        .map(|alias| alias.core)
}

pub fn normalize_command_payload(public: &str, payload: &[u8]) -> Result<Vec<u8>, ImError> {
    match public {
        "im_send" => normalize_im_send_payload(payload),
        _ => Ok(payload.to_vec()),
    }
}

fn normalize_im_send_payload(payload: &[u8]) -> Result<Vec<u8>, ImError> {
    let mut value: Value = serde_json::from_slice(payload)
        .map_err(|e| ImError::Parse(format!("im_send payload: {e}")))?;
    let obj = value
        .as_object_mut()
        .ok_or_else(|| ImError::Parse("im_send payload must be object".into()))?;
    move_key(obj, "channelId", "channel_id");
    move_key(obj, "temporaryId", "temporary_id");
    serde_json::to_vec(&value).map_err(|e| ImError::Serialize(e.to_string()))
}

fn move_key(obj: &mut Map<String, Value>, from: &str, to: &str) {
    if !obj.contains_key(to) {
        if let Some(v) = obj.remove(from) {
            obj.insert(to.to_string(), v);
        }
    } else {
        obj.remove(from);
    }
}

pub fn capabilities_json() -> Vec<u8> {
    let mut commands = crate::outbound::outbound_command_names();
    commands.push("im_send");
    commands.push("im_reconnect");
    commands.extend_from_slice(crate::query::query_command_names());
    commands.sort_unstable();
    commands.dedup();

    let value = json!({
        "abi": CONTRACT_VERSION,
        "bus": BUS_CHANNEL,
        "commands": commands,
        "queries": ["helix_capabilities", "helix_version"],
        "events": PUBLIC_EVENTS,
        "features": {
            "pendingSendTimeoutUnsend": true,
            "batchEventCallback": true,
            "readResultReqId": true,
            "noPerBusinessAbiSymbol": true
        }
    });
    serde_json::to_vec(&value).expect("static capabilities JSON must serialize")
}

#[cfg(test)]
mod tests;
```

- [ ] **步骤 5：导出模块**

添加到 `crates/helix-im/src/lib.rs`：

```rust
pub mod mobile_contract;
```

- [ ] **步骤 6：运行测试**

运行：

```bash
cargo test -p helix-im mobile_contract -- --nocapture
```

预期：通过。

- [ ] **步骤 7：提交**

```bash
git add crates/helix-im/src/mobile_contract.rs crates/helix-im/src/lib.rs crates/helix-im/src/outbound/registry.rs crates/helix-im/src/query/core.rs
git commit -m "feat(im): 冻结移动端公开契约元数据"
```

---

### 任务 2：在 FFI 边界归一化移动端命令

**文件：**
- 修改：`crates/helix-driver-ffi/src/api.rs`
- 测试：`crates/helix-driver-ffi/tests/mobile_contract_test.rs`

**接口：**
- 消费：
  - `helix_im::mobile_contract::command_alias`
  - `helix_im::mobile_contract::normalize_command_payload`
- 产出：
  - `helix_command(..., "im_send", camelCasePayload)` 入队为 `Tick::Command { name: "im_send_message", payload: snake_case_payload }`
  - 既有 `"im_send_message"` 路径仍然可用。

- [ ] **步骤 1：为移动端发送别名写失败的 FFI 测试**

创建 `crates/helix-driver-ffi/tests/mobile_contract_test.rs`：

```rust
use helix_driver_ffi::api::{helix_command, helix_create_with_network, helix_destroy, FFI_OK};
use helix_driver_ffi::batch::decode_batch;
use helix_driver_ffi::vtable::CppHostVtable;
use std::sync::Mutex;
use std::time::{Duration, Instant};

type Recorder = Mutex<Vec<Vec<u8>>>;

extern "C" fn fixed_now_ms(_ctx: *mut std::ffi::c_void) -> u64 {
    1_700_000_000_000
}

extern "C" fn record_batch(ctx: *mut std::ffi::c_void, ptr: *const u8, len: usize) {
    let rec = unsafe { &*(ctx as *const Recorder) };
    let buf = unsafe { std::slice::from_raw_parts(ptr, len) };
    if let Some(events) = decode_batch(buf) {
        let mut guard = rec.lock().unwrap();
        for event in events {
            guard.push(event.to_vec());
        }
    }
}

fn create_recording_handle(recorder: &Recorder) -> *mut helix_driver_ffi::api::HelixHandle {
    let api = b"https://api.example.test";
    let ws = b"wss://ws.example.test/socket";
    let vtable = CppHostVtable {
        on_domain_events_batch: Some(record_batch),
        now_ms: Some(fixed_now_ms),
        ctx: recorder as *const Recorder as *mut std::ffi::c_void,
    };
    let handle = unsafe {
        helix_create_with_network(api.as_ptr(), api.len(), ws.as_ptr(), ws.len(), vtable)
    };
    assert!(!handle.is_null());
    handle
}

fn wait_for_event(recorder: &Recorder, needle: &str) -> String {
    let start = Instant::now();
    loop {
        if let Some(found) = recorder
            .lock()
            .unwrap()
            .iter()
            .filter_map(|bytes| std::str::from_utf8(bytes).ok())
            .find(|text| text.contains(needle))
            .map(str::to_owned)
        {
            return found;
        }
        assert!(start.elapsed() < Duration::from_secs(3), "missing event {needle}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn mobile_im_send_alias_accepts_camelcase_payload_and_emits_sending() {
    let recorder = Recorder::new(Vec::new());
    let handle = create_recording_handle(&recorder);
    let name = b"im_send";
    let payload =
        br#"{"channelId":"01hq0000000000000000000000","temporaryId":"tmp-mobile-1","text":"hi","type":"TEXT"}"#;

    let status = unsafe {
        helix_command(handle, name.as_ptr(), name.len(), payload.as_ptr(), payload.len())
    };
    assert_eq!(status, FFI_OK);

    let event = wait_for_event(&recorder, "im:post:sending");
    assert!(event.contains("tmp-mobile-1"));
    assert!(event.contains("01hq0000000000000000000000"));
    assert!(!event.contains("im:post:send-failed"));

    unsafe { helix_destroy(handle) };
}
```

- [ ] **步骤 2：运行失败测试**

运行：

```bash
cargo test -p helix-driver-ffi --test mobile_contract_test mobile_im_send_alias -- --nocapture
```

预期：失败，因为 `"im_send"` 目前尚未映射到 `"im_send_message"`，并且载荷字段仍是 camelCase。

- [ ] **步骤 3：在 `helix_command` 中归一化名称和载荷**

在 `crates/helix-driver-ffi/src/api.rs` 中更新 `helix_command`，使其执行以下归一化：

```rust
let public_name = name_str;
let payload_vec = if payload.is_null() || payload_len == 0 {
    Vec::new()
} else {
    unsafe { std::slice::from_raw_parts(payload, payload_len) }.to_vec()
};

let normalized_payload = match helix_im::mobile_contract::normalize_command_payload(
    public_name.as_str(),
    &payload_vec,
) {
    Ok(bytes) => bytes,
    Err(e) => {
        tracing::warn!(command = public_name.as_str(), error = ?e, "mobile command payload rejected");
        return FFI_INVALID;
    }
};
let payload_bytes = Bytes::from(normalized_payload);

let core_name = helix_im::mobile_contract::command_alias(public_name.as_str())
    .unwrap_or(public_name.as_str());
let name: Cow<'static, str> = match helix_im::outbound::canonical_command_name(core_name) {
    Some(static_name) => Cow::Borrowed(static_name),
    None if core_name == "im_send_message" => Cow::Borrowed("im_send_message"),
    None if core_name == "im_reconnect" => Cow::Borrowed("im_reconnect"),
    None => Cow::Owned(core_name.to_string()),
};
```

保持既有 `try_send(Tick::Command(cmd))` 逻辑不变。

- [ ] **步骤 4：运行测试**

运行：

```bash
cargo test -p helix-driver-ffi --test mobile_contract_test mobile_im_send_alias -- --nocapture
```

预期：通过。

- [ ] **步骤 5：提交**

```bash
git add crates/helix-driver-ffi/src/api.rs crates/helix-driver-ffi/tests/mobile_contract_test.rs
git commit -m "feat(ffi): 支持移动端命令别名与payload归一"
```

---

### 任务 3：实现能力集与本地元信息查询

**文件：**
- 修改：`crates/helix-driver-ffi/src/api.rs`
- 修改：`crates/helix-driver-ffi/tests/mobile_contract_test.rs`

**接口：**
- 消费：`helix_im::mobile_contract::capabilities_json`
- 产出：
  - `helix_query("helix_capabilities", "{}") -> FFI_OK + JSON`
  - `helix_query("helix_version", "{}") -> FFI_OK + {"abi":1}`
  - 未知查询仍返回 `FFI_UNIMPLEMENTED`，并清空输出参数。

- [ ] **步骤 1：添加先失败的查询测试**

追加到 `crates/helix-driver-ffi/tests/mobile_contract_test.rs`：

```rust
use helix_driver_ffi::api::{helix_query, helix_query_free, FFI_UNIMPLEMENTED};

#[test]
fn helix_capabilities_query_returns_mobile_contract_json() {
    let recorder = Recorder::new(Vec::new());
    let handle = create_recording_handle(&recorder);
    let name = b"helix_capabilities";
    let payload = b"{}";
    let mut out_ptr: *mut u8 = std::ptr::null_mut();
    let mut out_len: usize = 0;

    let status = unsafe {
        helix_query(
            handle,
            name.as_ptr(),
            name.len(),
            payload.as_ptr(),
            payload.len(),
            &mut out_ptr,
            &mut out_len,
        )
    };
    assert_eq!(status, FFI_OK);
    assert!(!out_ptr.is_null());
    assert!(out_len > 0);

    let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
    let caps: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(caps["abi"], 1);
    assert_eq!(caps["bus"], "im:__bus__");
    assert_eq!(caps["features"]["pendingSendTimeoutUnsend"], true);
    assert!(caps["commands"].as_array().unwrap().iter().any(|v| v == "im_send"));
    assert!(!caps["events"].as_array().unwrap().iter().any(|v| v == "im:post:send-failed"));

    unsafe { helix_query_free(out_ptr, out_len) };
    unsafe { helix_destroy(handle) };
}

#[test]
fn unknown_query_remains_unimplemented_and_clears_out_params() {
    let recorder = Recorder::new(Vec::new());
    let handle = create_recording_handle(&recorder);
    let name = b"not_a_query";
    let payload = b"{}";
    let mut out_ptr: *mut u8 = 1usize as *mut u8;
    let mut out_len: usize = 999;

    let status = unsafe {
        helix_query(
            handle,
            name.as_ptr(),
            name.len(),
            payload.as_ptr(),
            payload.len(),
            &mut out_ptr,
            &mut out_len,
        )
    };
    assert_eq!(status, FFI_UNIMPLEMENTED);
    assert!(out_ptr.is_null());
    assert_eq!(out_len, 0);

    unsafe { helix_destroy(handle) };
}
```

- [ ] **步骤 2：运行失败测试**

运行：

```bash
cargo test -p helix-driver-ffi --test mobile_contract_test helix_capabilities -- --nocapture
```

预期：失败，因为 `helix_query` 仍然返回 `FFI_UNIMPLEMENTED`。

- [ ] **步骤 3：在 `api.rs` 中添加 boxed JSON 辅助函数**

在 `crates/helix-driver-ffi/src/api.rs` 的 `helix_query_free` 附近添加辅助函数：

```rust
unsafe fn write_query_bytes(bytes: Vec<u8>, out_ptr: *mut *mut u8, out_len: *mut usize) -> i32 {
    let mut boxed = bytes.into_boxed_slice();
    let len = boxed.len();
    let ptr = boxed.as_mut_ptr();
    std::mem::forget(boxed);
    *out_ptr = ptr;
    *out_len = len;
    FFI_OK
}
```

- [ ] **步骤 4：实现元信息查询名**

在 `helix_query` 中，清空 `out_ptr/out_len` 后解析 `_query_name` 并处理元信息查询：

```rust
let query_name = match unsafe { ffi_str(_query_name, _query_name_len) } {
    Ok(s) => s,
    Err(code) => return code,
};

match query_name.as_str() {
    "helix_capabilities" => unsafe {
        write_query_bytes(helix_im::mobile_contract::capabilities_json(), out_ptr, out_len)
    },
    "helix_version" => unsafe {
        write_query_bytes(br#"{"abi":1}"#.to_vec(), out_ptr, out_len)
    },
    _ => FFI_UNIMPLEMENTED,
}
```

本任务不要实现业务表扫描。

- [ ] **步骤 5：运行测试**

运行：

```bash
cargo test -p helix-driver-ffi --test mobile_contract_test -- --nocapture
cargo test -p helix-driver-ffi query_returns_unimplemented
```

预期：通过。

- [ ] **步骤 6：提交**

```bash
git add crates/helix-driver-ffi/src/api.rs crates/helix-driver-ffi/tests/mobile_contract_test.rs
git commit -m "feat(ffi): 公开移动端capabilities查询"
```

---

### 任务 4：为同步 FFI 读取添加 IM 拥有的本地查询计划

**文件：**
- 新增：`crates/helix-im/src/mobile_query.rs`
- 修改：`crates/helix-im/src/lib.rs`
- 修改：`crates/helix-driver-ffi/src/api.rs`
- 测试：`crates/helix-im/src/mobile_query.rs` 内联测试
- 测试：`crates/helix-driver-ffi/tests/mobile_contract_test.rs`

**接口：**
- 消费：通过 `helix_core::ports::Storage` 调用 `HostStorage::scan(ScanSpec)`
- 产出：
  - `helix_im::mobile_query::plan(name, payload) -> Result<MobileQueryPlan, ImError>`
  - `helix_im::mobile_query::shape_json(shape, rows) -> Vec<u8>`
  - FFI 同步支持本地读取：
    - `im_query_dialog_list`
    - `im_query_messages_by_channel`
  - HTTP read-family 查询在本阶段仍保持 command+event：`im_get_replies`、`im_get_reply_branch`、`im_health`、成员读取。

- [ ] **步骤 1：编写先失败的 helix-im 查询计划测试**

创建 `crates/helix-im/src/mobile_query.rs`，先写入测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::SqlValue;

    #[test]
    fn message_query_plan_keeps_schema_inside_helix_im() {
        let plan = plan(
            "im_query_messages_by_channel",
            br#"{"channelId":"ch1","limit":25}"#,
        )
        .expect("plan");
        match plan {
            MobileQueryPlan::Scan { spec, shape } => {
                assert_eq!(spec.table, "message");
                assert_eq!(spec.limit, Some(25));
                assert_eq!(shape, MobileQueryShape::MessagesByChannel { channel_id: "ch1".into() });
                assert_eq!(spec.filter, Some(("channel_id", SqlValue::Text("ch1".into()))));
            }
        }
    }
}
```

- [ ] **步骤 2：运行失败测试**

运行：

```bash
cargo test -p helix-im mobile_query -- --nocapture
```

预期：失败，因为 `MobileQueryPlan` 尚不存在。

- [ ] **步骤 3：实现 `mobile_query.rs`**

按下面的实现形状落地：

```rust
use helix_core::effect::{Row, ScanSpec, SqlValue};
use serde_json::Value;

use crate::error::ImError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobileQueryShape {
    MessagesByChannel { channel_id: String },
    DialogList,
}

pub enum MobileQueryPlan {
    Scan {
        spec: ScanSpec,
        shape: MobileQueryShape,
    },
}

pub fn plan(name: &str, payload: &[u8]) -> Result<MobileQueryPlan, ImError> {
    match name {
        "im_query_messages_by_channel" => message_query_plan(payload),
        "im_query_dialog_list" => dialog_list_plan(payload),
        _ => Err(ImError::Parse(format!("unsupported mobile sync query '{name}'"))),
    }
}

fn message_query_plan(payload: &[u8]) -> Result<MobileQueryPlan, ImError> {
    let v: Value = serde_json::from_slice(payload)
        .map_err(|e| ImError::Parse(format!("im_query_messages_by_channel payload: {e}")))?;
    let channel_id = v
        .get("channelId")
        .or_else(|| v.get("channel_id"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ImError::Parse("im_query_messages_by_channel 缺 channelId".into()))?
        .to_string();
    let limit = v
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| (n as u32).clamp(1, 500))
        .unwrap_or(50);
    Ok(MobileQueryPlan::Scan {
        spec: ScanSpec {
            table: "message",
            limit: Some(limit),
            filter: Some(("channel_id", SqlValue::Text(channel_id.clone()))),
            order_desc_col: Some("create_at"),
        },
        shape: MobileQueryShape::MessagesByChannel { channel_id },
    })
}

fn dialog_list_plan(payload: &[u8]) -> Result<MobileQueryPlan, ImError> {
    let limit = serde_json::from_slice::<Value>(payload)
        .ok()
        .and_then(|v| v.get("limit").and_then(Value::as_u64))
        .map(|n| (n as u32).clamp(1, 2000))
        .unwrap_or(500);
    Ok(MobileQueryPlan::Scan {
        spec: ScanSpec {
            table: "channel",
            limit: Some(limit),
            filter: None,
            order_desc_col: Some("last_post_at"),
        },
        shape: MobileQueryShape::DialogList,
    })
}

pub fn shape_json(shape: MobileQueryShape, rows: Vec<Row>) -> Vec<u8> {
    let rows_json = rows_to_json(rows);
    let value = match shape {
        MobileQueryShape::MessagesByChannel { channel_id } => serde_json::json!({
            "event": "im:messages:query_result",
            "data": {
                "channel_id": channel_id,
                "messages": crate::render_ready::shape_message_rows(&rows_json),
            }
        }),
        MobileQueryShape::DialogList => serde_json::json!({
            "event": "im:channels:projection",
            "data": {
                "dialogList": rows_json.as_array().cloned().unwrap_or_default(),
            }
        }),
    };
    serde_json::to_vec(&value).expect("mobile query JSON shape must serialize")
}

fn rows_to_json(rows: Vec<Row>) -> Value {
    Value::Array(
        rows.into_iter()
            .map(|row| {
                let mut obj = serde_json::Map::new();
                for (key, value) in row.into_iter() {
                    obj.insert(key, sql_value_to_json(value));
                }
                Value::Object(obj)
            })
            .collect(),
    )
}

fn sql_value_to_json(value: SqlValue) -> Value {
    match value {
        SqlValue::Null => Value::Null,
        SqlValue::Integer(n) => Value::from(n),
        SqlValue::Real(n) => Value::from(n),
        SqlValue::Text(s) => Value::String(s),
        SqlValue::Blob(bytes) => Value::Array(bytes.into_iter().map(Value::from).collect()),
    }
}
```

- [ ] **步骤 4：导出模块**

添加到 `crates/helix-im/src/lib.rs`：

```rust
pub mod mobile_query;
```

- [ ] **步骤 5：实现 FFI 查询执行**

在 `crates/helix-driver-ffi/src/api.rs` 中添加：

```rust
use helix_core::ports::Storage;
```

在元信息查询处理之后扩展 `helix_query`：

```rust
let query_payload = if _query_payload.is_null() || _query_payload_len == 0 {
    &[][..]
} else {
    unsafe { std::slice::from_raw_parts(_query_payload, _query_payload_len) }
};
let plan = match helix_im::mobile_query::plan(query_name.as_str(), query_payload) {
    Ok(plan) => plan,
    Err(_) => return FFI_UNIMPLEMENTED,
};

let handle_ref = unsafe { &*handle };
let storage = handle_ref.query.storage.clone();
let (tx, rx) = std::sync::mpsc::sync_channel(1);
handle_ref.query.runtime.spawn(async move {
    let result = match plan {
        helix_im::mobile_query::MobileQueryPlan::Scan { spec, shape } => {
            storage.scan(spec).await.map(|rows| helix_im::mobile_query::shape_json(shape, rows))
        }
    };
    let _ = tx.send(result);
});

match rx.recv_timeout(std::time::Duration::from_secs(3)) {
    Ok(Ok(bytes)) => unsafe { write_query_bytes(bytes, out_ptr, out_len) },
    Ok(Err(e)) => {
        tracing::warn!(query = query_name.as_str(), error = ?e, "mobile sync query storage failed");
        FFI_INVALID
    }
    Err(_) => FFI_BUSY,
}
```

- [ ] **步骤 6：添加 FFI 本地查询测试**

追加到 `mobile_contract_test.rs`：

```rust
#[test]
fn local_dialog_list_query_returns_json_or_empty_projection() {
    let recorder = Recorder::new(Vec::new());
    let handle = create_recording_handle(&recorder);
    let name = b"im_query_dialog_list";
    let payload = br#"{"limit":10}"#;
    let mut out_ptr: *mut u8 = std::ptr::null_mut();
    let mut out_len: usize = 0;
    let status = unsafe {
        helix_query(
            handle,
            name.as_ptr(),
            name.len(),
            payload.as_ptr(),
            payload.len(),
            &mut out_ptr,
            &mut out_len,
        )
    };
    assert_eq!(status, FFI_OK);
    let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
    let value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(value["event"], "im:channels:projection");
    assert!(value["data"]["dialogList"].is_array());
    unsafe { helix_query_free(out_ptr, out_len) };
    unsafe { helix_destroy(handle) };
}
```

- [ ] **步骤 7：运行测试**

运行：

```bash
cargo test -p helix-im mobile_query -- --nocapture
cargo test -p helix-driver-ffi --test mobile_contract_test -- --nocapture
```

预期：通过。

- [ ] **步骤 8：提交**

```bash
git add crates/helix-im/src/mobile_query.rs crates/helix-im/src/lib.rs crates/helix-driver-ffi/src/api.rs crates/helix-driver-ffi/tests/mobile_contract_test.rs
git commit -m "feat(ffi): 接通移动端本地同步查询"
```

---

### 任务 5：交接边界与验收文档

**文件：**
- 修改：`crates/helix-driver-ffi/docs/移动端Command-Query-Event契约SPEC.md`
- 修改：`crates/helix-driver-ffi/docs/移动端QuickJS接入最终需求与验收标准.md`
- 修改：`SESSION.md`

**接口：**
- 消费：以上所有 Rust 侧任务。
- 产出：
  - 一段仓库边界声明：C++/QuickJS 源码与编译产物属于移动端集成项目。
  - 一份 Helix Rust FFI 契约就绪验收清单。
  - 一份外部项目 C++/QuickJS/Flutter 实现清单，但不在 Helix 中创建这些文件。

- [ ] **步骤 1：检查 Helix 中不存在移动端桥接文件**

运行：

```bash
test ! -e crates/helix-driver-ffi/smoke/mobile_bridge.hpp
test ! -e crates/helix-driver-ffi/smoke/mobile_bridge_smoke.cpp
test ! -e crates/helix-driver-ffi/smoke/mobile_core_bridge.js
test ! -e crates/helix-driver-ffi/smoke/mobile_core_bridge_smoke.mjs
```

预期：所有命令退出码为 0。

- [ ] **步骤 2：按需更新规格边界**

确认 `crates/helix-driver-ffi/docs/移动端Command-Query-Event契约SPEC.md` 包含这段声明：

```markdown
仓库边界：

- helix 仓负责：Rust C ABI、`helix.h` / `helix.hpp`、native library 构建、capabilities、command/query/event 契约、Rust/FFI 层验收。
- 移动端集成项目负责：C++ `CoreBridge` class、QuickJS native class 注册、JS wrapper、Android/iOS 平台构建脚本、C++/QuickJS 编译产物、真机 smoke。
- helix 仓不得继续新增移动端 C++ bridge 源码、QuickJS wrapper 源码、QuickJS demo 编译产物；这里只保留契约和低层 ABI 验证。
```

- [ ] **步骤 3：按需更新最终验收边界**

确认 `crates/helix-driver-ffi/docs/移动端QuickJS接入最终需求与验收标准.md` 写明：

```markdown
`helix` 仓负责把 Rust FFI ABI、公开 command/query/event 契约、capabilities、header/native library 和底层验收说明交付出来；C++ bridge、QuickJS 注册代码、JS wrapper、平台编译产物和真机 smoke 应在移动端集成项目中创建和维护。
```

- [ ] **步骤 4：向 SESSION 追加终态行**

向 `SESSION.md` §0 追加一行：

```markdown
✅ DONE 移动端 QuickJS 契约 Rust 侧实现 @<timestamp> | 仅 helix：命令别名 + 载荷归一、capabilities/version、本地同步查询、Rust/FFI 契约测试 | C++ bridge、QuickJS、平台产物由外部移动端集成项目负责 | send-failed 仍非 P0 | 验证：<commands> | <branch>
```

使用真实时间戳和真实验证结果。

- [ ] **步骤 5：提交文档**

```bash
git add SESSION.md crates/helix-driver-ffi/docs/移动端Command-Query-Event契约SPEC.md crates/helix-driver-ffi/docs/移动端QuickJS接入最终需求与验收标准.md
git commit -m "docs(ffi): 明确移动端桥接产物归属边界"
```

---

### 任务 6：最终 Rust 侧验证

**文件：**
- 除非验证失败定位到任务 1-4 文件里的 bug，否则不改源码。

**接口：**
- 消费：任务 1-5。
- 产出：已验证的 Rust 侧移动端契约与外部交接边界。

- [ ] **步骤 1：运行聚焦测试**

运行：

```bash
cargo test -p helix-im mobile_contract -- --nocapture
cargo test -p helix-im mobile_query -- --nocapture
cargo test -p helix-driver-ffi --test mobile_contract_test -- --nocapture
```

预期：通过。

- [ ] **步骤 2：运行不依赖外部 demo 的 FFI 验证**

运行：

```bash
bash scripts/ffi-verify.sh --no-demo
```

预期：通过。这里仍只验证 Rust/ABI，不得依赖 Helix 中的 QuickJS 或外部移动端桥接文件。

- [ ] **步骤 3：运行产品目标检查**

运行：

```bash
cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi
cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri' || true
```

预期：`cargo check` 通过。`cargo tree | grep` 命令必须不输出任何内容。

- [ ] **步骤 4：运行边界回归检查**

运行：

```bash
rg -n "im:post:send-failed|sendFailedProjection" crates/helix-im crates/helix-driver-ffi docs/superpowers/plans/2026-07-03-mobile-quickjs-contract.md
find crates/helix-driver-ffi -path '*mobile_bridge*' -o -path '*mobile_core_bridge*'
```

预期：send-failed 只出现在负向断言 / 仲裁说明中。`find` 命令不输出任何内容。

- [ ] **步骤 5：运行结构 / 漂移闸门**

运行：

```bash
scripts/structure-gate.sh
git diff --check
```

预期：通过。

- [ ] **步骤 6：如有验证修复则提交**

如果验证过程需要修复：

```bash
git add <fixed-files>
git commit -m "fix(ffi): 收口移动端契约验收"
```

如果没有文件变更，不要创建空提交。

---

## 外部项目后续计划

本计划在 `helix` 落地后，在移动端集成项目中另起一份实现计划。外部计划应创建：

- 封装 `helix.h` / `helix.hpp` 的 C++ `CoreBridge` 类
- QuickJS 原生类注册
- 暴露 `CoreBridge.call/query/capabilities/onEvents/dispose` 的 JS 封装层
- Android / iOS 构建脚本与原生产物打包
- 链接真实 `helix-driver-ffi` 的 C++ 直连冒烟
- 覆盖每个公开封装方法的 QuickJS 冒烟
- 通过 JS 与事件批驱动的 Flutter 发消息端到端验证

不要在 `helix` 中实现这些文件。

---

## 已知仲裁闸门

当前存在一个 LoopForge/Helix worktree 路径曾试验过 `im:post:send-failed`。在 `main` 上执行本计划前，先检查来自 `/System/Volumes/Data/workspace/rust/helix/worktrees/loopforge-ui-uc-gap-closure` 的任何待合并候选。

如果该分支新增了 `im:post:send-failed` 投影工厂或 capabilities 标记，停下来先选择一个契约：

1. 保持当前 main 契约：没有 P0 send-failed 事件；移动端遵守 PC/Tauri 的 pending-send timeout 模型。
2. 升级跨平台契约：PC/Tauri 和移动端一起增加 send-failed，同时更新 `projection-schema.md`、回放测试、LoopForge 与本计划。

不要在本计划声明 send-failed 非 P0 的同时，静默合入一个仅移动端拥有的 send-failed 事件。

---

## 自检

**规格覆盖：** 覆盖 Rust 侧通过 `helix_command` / `helix_query` 承接的 `CoreBridge` 输入契约、capabilities、命令别名、移动端 camelCase 载荷归一、事件批交付、本地同步查询支持与发送状态契约。C++/QuickJS/Flutter 实现明确属于本仓外部。

**占位扫描：** 没有任务要求在 Helix 中创建未知文件。外部项目工作只作为后续范围列出，不作为 Helix 文件操作。

**类型一致性：** 公开命令别名统一为 `im_send -> im_send_message`。Capabilities 字段统一为 `pendingSendTimeoutUnsend`，不是 `sendFailedProjection`。查询计划归 `helix-im` 拥有；`driver-ffi` 只执行计划。
