# helix-tauri —— Tauri 宿主设计文档

> **版本** 2026-06-08 · 基于 cses-client 源码实测（17 个 Tauri 耦合点）
>
> **本文目的**：
> 1. 详述 `#[tauri::command]` 薄壳如何封装 core engine（对照实测 15 个 im_* 命令）。
> 2. 说明 `emit` / `listen` / Channel API 如何把 `DomainEvent` → `im:__bus__` → 前端。
> 3. 说明 core ports + driver 如何通过 `app.manage()` + `tauri::State` 注入。
> 4. 分析当前 17 个 Tauri 耦合点如何收敛进 driver + 宿主。

---

## 0. 分层与角色

```
Angular 前端（IPC bridge: TauriService）
    │ invoke / listen
    ▼
helix-tauri（Tauri 宿主）
    ├── commands.rs   ← #[tauri::command] 薄壳（≤5行，只做参数转换）
    ├── event_bus.rs  ← DomainEvent → im:__bus__ 序列化 + Tauri emit
    ├── setup.rs      ← app.manage() 注入所有 State
    └── plugin.rs     ← tauri::Builder.plugin(init())
         │
         │  depends on
         ▼
helix-driver-native（实现 core ports）
         │
         ▼
helix-core + helix-im（确定性内核）
```

**tauri 宿主的角色只有三件事**：

1. 把前端 `invoke` 请求翻译成 `engine.step(Tick::AppCommand(...))`。
2. 把 `DomainEvent` 序列化后通过 Tauri `emit` 发给前端。
3. `app.manage()` 注入 driver 实例（不做业务逻辑）。

---

## 1. State 注入：`app.manage()` + `tauri::State`

### 1.1 禁止全局 LazyLock

```rust
// ❌ 禁止（全局单例，无 late-binding，测试困难）
static ENGINE: LazyLock<Arc<Mutex<CoreEngine>>> = LazyLock::new(|| ...);

// ✅ 正确（Tauri State，late-binding，命令层 panic 可被 Tauri 捕获）
app.manage(EngineState(Arc::new(Mutex::new(engine))));
```

### 1.2 需要 manage 的类型

```rust
// helix-tauri/src/setup.rs

/// CoreEngine 封装（Arc<Mutex<>> 保护 step 调用）
pub struct EngineState(pub Arc<tokio::sync::Mutex<helix_core::CoreEngine>>);

/// engine_tx：前端命令 → Tick 入口
pub struct EngineTxState(pub futures_channel::mpsc::UnboundedSender<helix_core::engine::Tick>);

/// DbManager：多租户 SQLite 管理
pub struct DbManagerState(pub Arc<helix_driver_native::DbManager>);

/// HttpRequester：运行时注入 token/connectionId
pub struct HttpState(pub helix_driver_native::NativeHttpRequester);

/// WS 连接控制
pub struct TransportState(pub Arc<tokio::sync::Mutex<helix_driver_native::NativeTransport>>);

pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let app_data_dir = app.path().app_data_dir()?;
    let db_manager = Arc::new(helix_driver_native::DbManager::new(app_data_dir));
    let clock = helix_driver_native::NativeClock;
    let (event_sink, event_rx) = helix_driver_native::NativeEventSink::new();
    let http = helix_driver_native::NativeHttpRequester::new(
        std::env::var("HELIX_API_BASE").unwrap_or_default()
    );
    let (engine_tx, engine_rx) = futures_channel::mpsc::unbounded::<helix_core::engine::Tick>();
    let engine = helix_core::CoreEngine::new(/* ports */);

    // Tauri 2：app.manage() 注入全部 State
    app.manage(EngineState(Arc::new(tokio::sync::Mutex::new(engine))));
    app.manage(EngineTxState(engine_tx.clone()));
    app.manage(DbManagerState(db_manager));
    app.manage(HttpState(http.clone()));

    // 启动 native driver 的后台 loop（tokio 任务，不在主线程）
    let app_handle = app.handle().clone();
    tokio::spawn(helix_driver_native::run_engine_loop(
        engine_rx,
        event_rx,
        app_handle,
        http,
        clock,
    ));

    Ok(())
}
```

---

## 2. `#[tauri::command]` 薄壳设计

### 2.1 原则

- 命令层 **≤5 行**，只做参数转换 + `engine_tx.send(Tick)`。
- **不做业务逻辑**（if/match 分支 > 2 层 = 警告，应该进 helix-im）。
- 参数类型用基本类型（`String`, `u64`, `bool`）或 `serde` JSON。
- 返回 `Result<T, String>`（Tauri IPC 约定），内部用 `map_err(|e| e.to_string())`。

### 2.2 实测 15 个 im_* 命令的映射

| cses-client 命令（实测） | helix-tauri 薄壳形态 | 映射到 Tick |
|---|---|---|
| `im_send_message(action, data, seq)` | `send_command(data: SendMessageCmd)` | `Tick::AppCommand(Box::new(ImCommand::Send(data)))` |
| `im_read_channel(channel_id, seq)` | `read_channel(channel_id, seq)` | `Tick::AppCommand(Box::new(ImCommand::Read { channel_id, seq }))` |
| `im_resend_message(temporary_id)` | `resend_message(temporary_id)` | `Tick::AppCommand(Box::new(ImCommand::Resend { temporary_id }))` |
| `im_get_messages(channel_id, params)` | `get_messages(channel_id, params)` | 直接查 Storage（只读，不走 engine.step） |
| `im_get_channel_cursor(channel_id)` | `get_cursor(channel_id)` | 直接查 Storage |
| `im_sync_channel(channel_id)` | `sync_channel(channel_id)` | `Tick::AppCommand(Box::new(ImCommand::Sync { channel_id }))` |
| `store_set(table, key, value)` | `store_set(table, key, value)` | 直接调 Storage::set |
| `store_get(table, key)` | `store_get(table, key)` | 直接调 Storage::get |
| `im_setup(config)` | `im_setup(config)` | 初始化 engine（manage 前调用） |
| `im_connect(tenant_key, token, ...)` | `im_connect(params)` | 切 DbManager 租户 + 启动 WS |
| `im_disconnect` | `im_disconnect` | `Tick::AppCommand(Box::new(ImCommand::Disconnect))` |

### 2.3 代码示例（最薄的命令形态）

```rust
// helix-tauri/src/commands.rs

/// 发消息：最薄形态，参数直接转 Tick
#[tauri::command]
pub async fn send_message(
    data: SendMessagePayload,
    engine_tx: tauri::State<'_, EngineTxState>,
) -> Result<(), String> {
    engine_tx.0
        .unbounded_send(Tick::AppCommand(Box::new(ImCommand::Send(data))))
        .map_err(|e| e.to_string())
}

/// 只读查询：绕过 engine，直接读 Storage（不需要走 step）
#[tauri::command]
pub async fn get_messages(
    channel_id: String,
    cursor: u64,
    limit: u32,
    db: tauri::State<'_, DbManagerState>,
    tenant: tauri::State<'_, CurrentTenantState>,
) -> Result<Vec<MessageRow>, String> {
    let storage = db.0.get_or_open(&tenant.0).await
        .map_err(|e| e.to_string())?;
    storage.query(
        "SELECT * FROM messages WHERE channel_id=? AND seq<=? ORDER BY seq DESC LIMIT ?",
        &[SqlParam::Text(channel_id), SqlParam::Int(cursor as i64), SqlParam::Int(limit as i64)],
    ).await.map_err(|e| e.to_string())
}

/// store_set：通用 KV 写（不经 engine，直接落 Storage）
#[tauri::command]
pub async fn store_set(
    table: String,
    key: String,
    value: String,
    db: tauri::State<'_, DbManagerState>,
    tenant: tauri::State<'_, CurrentTenantState>,
) -> Result<(), String> {
    let storage = db.0.get_or_open(&tenant.0).await
        .map_err(|e| e.to_string())?;
    storage.execute(
        "INSERT OR REPLACE INTO kv_store(table_name, key, value) VALUES(?,?,?)",
        &[SqlParam::Text(table), SqlParam::Text(key), SqlParam::Text(value)],
    ).await.map_err(|e| e.to_string())
}
```

### 2.4 命令注册（`generate_handler!` 编译期检查）

```rust
// helix-tauri/src/lib.rs
pub fn run() {
    tauri::Builder::default()
        .plugin(helix_tauri_plugin::init())
        .invoke_handler(tauri::generate_handler![
            commands::send_message,
            commands::read_channel,
            commands::resend_message,
            commands::get_messages,
            commands::get_cursor,
            commands::sync_channel,
            commands::store_set,
            commands::store_get,
            commands::im_connect,
            commands::im_disconnect,
            // ... 其余命令
        ])
        .setup(setup::setup)
        .run(tauri::generate_context!())
        .expect("tauri run");
}
```

---

## 3. EventBus：DomainEvent → `im:__bus__` → 前端

### 3.1 单总线设计（省 -190ms 启动）

所有 DomainEvent 走同一个 `im:__bus__` 通道，前端 `ImEventBusDispatcher` 按 `channel` 字段解复用。这个设计来自 cses-client 实测（100+ 事件合并成一个 listen 省 -190ms 启动时间）。

### 3.2 bus loop 实现

```rust
// helix-driver-native/src/event_sink.rs（已在 02-driver-native.md 定义 NativeEventSink）

// helix-tauri/src/event_bus.rs

/// Envelope 结构（与前端 ImEventBusDispatcher 约定）
#[derive(serde::Serialize)]
struct ImBusEnvelope {
    channel: &'static str,     // "im:post:received" 等 wire 名
    payload: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    tracing: Option<TracingContext>,  // OTel trace 透传
}

/// bus loop：消费 broadcast → 序列化 → Tauri emit
pub async fn run_bus_loop(
    mut event_rx: tokio::sync::broadcast::Receiver<helix_core::events::DomainEvent>,
    app_handle: tauri::AppHandle,
) {
    loop {
        match event_rx.recv().await {
            Ok(event) => {
                if let Some((channel, payload)) = domain_event_to_wire(&event) {
                    let envelope = ImBusEnvelope {
                        channel,
                        payload,
                        tracing: event.tracing_context(),
                    };
                    // emit_to：只发给有权限的 window
                    if let Err(e) = app_handle.emit_to(
                        tauri::EventTarget::labeled("main"),
                        "im:__bus__",
                        &envelope,
                    ) {
                        tracing::warn!("emit_to failed: {e}");
                    }
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                // broadcast 消费者太慢，lagged 跳过 n 条
                tracing::warn!("bus loop lagged {n} events");
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}
```

### 3.3 wire 名映射（emit_im 单点收口）

**铁律**：所有 `im:` 前缀的 wire 名**只在 `event_bus.rs` 里出现**。CI grep 守卫：

```bash
# Makefile / CI
.PHONY: check-emit-im-discipline
check-emit-im-discipline:
    @count=$$(grep -r '\.emit("im:' src-tauri/src/ --include='*.rs' | grep -v event_bus.rs | wc -l); \
    if [ "$$count" -gt 0 ]; then echo "❌ emit_im 旁路：$$count 处违规"; exit 1; fi
    @echo "✅ emit_im 收口正常"
```

```rust
// helix-tauri/src/event_bus.rs（domain_event_to_wire 是唯一 wire 名来源）

pub fn domain_event_to_wire(event: &DomainEvent) -> Option<(&'static str, serde_json::Value)> {
    use DomainEvent::*;
    match event {
        PostReceived { post, channel_id } => Some((
            "im:post:received",
            serde_json::json!({ "post": post, "channelId": channel_id }),
        )),
        PostUpdated { post } => Some((
            "im:post:updated",
            serde_json::to_value(post).ok()?,
        )),
        PostDeleted { post_id, channel_id } => Some((
            "im:post:deleted",
            serde_json::json!({ "postId": post_id, "channelId": channel_id }),
        )),
        PostBatchUpdated { posts } => Some((
            "im:post:batch-updated",
            serde_json::to_value(posts).ok()?,
        )),
        PostRead { channel_id, last_read_seq } => Some((
            "im:post:read",
            serde_json::json!({ "channelId": channel_id, "lastReadSeq": last_read_seq }),
        )),
        ChannelReadEcho { channel_id, last_read_seq } => Some((
            "im:channel:read_echo",
            serde_json::json!({ "channelId": channel_id, "lastReadSeq": last_read_seq }),
        )),
        ChannelUpdateByPost { channel_id } => Some((
            "im:channel:update-by-post",
            serde_json::json!({ "channelId": channel_id }),
        )),
        SyncTooLong { channel_id } => Some((
            "im:sync:too_long",
            serde_json::json!({ "channelId": channel_id }),
        )),
        Connected { connection_id } => Some((
            "im:connection:established",
            serde_json::json!({ "connectionId": connection_id }),
        )),
        _ => None,
    }
}
```

---

## 4. Channel API（有序流式，如下载进度）

对于长时间有序流（文件上传进度、批量同步进度），用 Tauri 2 Channel API：

```rust
// helix-tauri/src/commands.rs

#[tauri::command]
pub async fn sync_bulk(
    channel_id: String,
    on_progress: tauri::ipc::Channel<SyncProgressEvent>,
    engine_tx: tauri::State<'_, EngineTxState>,
) -> Result<(), String> {
    // Channel 是有序、有背压的流，避免事件积压
    let (progress_tx, mut progress_rx) = futures_channel::mpsc::channel::<SyncProgressEvent>(32);

    // 把 Channel 发到 engine（通过命令参数注入）
    tokio::spawn(async move {
        while let Some(event) = progress_rx.next().await {
            if on_progress.send(event).is_err() {
                break;  // 前端关闭 Channel
            }
        }
    });

    engine_tx.0.unbounded_send(Tick::AppCommand(Box::new(
        ImCommand::SyncBulk { channel_id, progress_tx }
    ))).map_err(|e| e.to_string())
}
```

**规则**：`im:__bus__` 用于实时高频事件（消息收发、状态变更）；Channel API 用于单次长流（进度条、批量操作）。不混用。

---

## 5. 17 个 Tauri 耦合点收敛方案

### 5.1 当前耦合点分析（源码实测）

| 文件 | 行号 | 耦合内容 | 类型 |
|---|---|---|---|
| `common.rs` | 44 | `AppHandle` 直接传入 handler，handler 内 `app.emit` | 直接 emit |
| `post.rs` | 104 | `AppHandle::emit_to` 发 `im:post:received` | 直接 emit |
| `post.rs` | 462 | `AppHandle::emit_to` 发 sync 结果 | 直接 emit |
| `post.rs` | 555 | `AppHandle::emit_to` 发 cursor 更新 | 直接 emit |
| `post.rs` | 640 | `AppHandle::emit_to` 发 `im:sync:too_long` | 直接 emit |
| `event_bus.rs` | 45 | `emit_im` 单点收口（当前已有，保留） | 单点收口 |
| `message_loop.rs` | 33 | `AppHandle` 注入 message_loop | 正确 |
| `message_loop.rs` | 318 | `app.emit` 发 lifecycle 事件 | 直接 emit |
| `commands.rs` | 全部 | 15 个 `#[tauri::command]` | 正确（薄壳） |

### 5.2 收敛目标

**现状问题**：`post.rs` 里的 handler 直接持有 `AppHandle` 并调 `emit_to`，业务逻辑与 Tauri API 耦合。收敛后：

```
❌ 旧：handler → AppHandle::emit_to("im:post:received", ...)  （分散在 post.rs 多处）
✅ 新：handler → EffectContext::push(Effect::Emit(DomainEvent::PostReceived {...}))
                → driver effect executor → NativeEventSink::emit(event)
                → event_bus.rs 的 bus loop → Tauri emit "im:__bus__"
```

### 5.3 逐点收敛方案

**`common.rs:44`（AppHandle 传入 handler）**：

- 移除 `AppHandle` 参数。
- handler 改为 `fn process(&mut self, frame: InboundFrame, ctx: &mut EffectContext)` 签名。
- 所有 `emit` 改为 `ctx.push(Effect::Emit(DomainEvent::xxx))`。

**`post.rs:104, 462, 555, 640`（四处分散 emit）**：

- 全部替换为对应的 `DomainEvent` variant push 到 `EffectContext`。
- `post.rs` 里不再 `use tauri`，不再引入 `AppHandle`。

**`message_loop.rs:318`（lifecycle emit）**：

- `WS lifecycle` 事件（Connected/Disconnected）改为 `DomainEvent::Connected { connection_id }` / `DomainEvent::Disconnected`。
- 在 `domain_event_to_wire` 里映射为 `"ws:lifecycle"` wire 名。

**收敛后的验证**：

```bash
# post.rs / common.rs / message_loop.rs 中不应再有 tauri import
grep -n 'use tauri' src-tauri/src/features/im/handlers/post.rs
# 期望：0 行
grep -n 'AppHandle' src-tauri/src/features/im/handlers/common.rs
# 期望：0 行
```

---

## 6. capabilities（最小权限）

```json
// src-tauri/capabilities/main.json（Tauri 2 格式）
{
  "$schema": "https://schemas.tauri.app/schemas/desktop-capability.json",
  "identifier": "main-window",
  "description": "Main window capabilities",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "core:event:allow-listen",
    "core:event:allow-emit",
    "helix-im:allow-send-message",
    "helix-im:allow-get-messages",
    "helix-im:allow-sync-channel",
    "helix-im:allow-store-get",
    "helix-im:allow-store-set",
    "helix-im:allow-im-connect",
    "helix-im:allow-im-disconnect"
  ]
}
```

**最小权限原则**：每个命令对应一条 `allow-*` permission。不授 `shell` / `fs` / `http` 等不需要的权限。前端通过 TauriService 单例调 invoke，不直接用 `@tauri-apps/api`（业务禁直连 invoke）。

---

## 7. helix-tauri plugin 结构

```rust
// helix-tauri/src/plugin.rs

pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("helix-im")
        .invoke_handler(tauri::generate_handler![
            commands::send_message,
            commands::read_channel,
            commands::resend_message,
            commands::get_messages,
            commands::get_cursor,
            commands::sync_channel,
            commands::store_set,
            commands::store_get,
            commands::im_connect,
            commands::im_disconnect,
        ])
        .setup(|app, _api| {
            setup::setup(app)?;
            Ok(())
        })
        .build()
}
```

把 IM 功能封装成 Tauri plugin 有两个好处：

1. 插件和主 app 解耦——非 IM 窗口不装载此 plugin。
2. plugin setup 函数是 `app.manage()` 注入 State 的天然位置，不污染 `lib.rs`（`lib.rs` ≤ 25 行，只调 `builder.plugin(init())`）。

---

## 8. hello 握手与 connectionId 回注

WS 连接成功后，server 发 `type=hello` 帧，内含 `connectionId`。处理流程：

```
WS 入站: type=hello, connectionId=xxx
    │
    ├─ helix-im handler → EffectContext::push(Effect::Emit(DomainEvent::Connected { connection_id }))
    │
    ▼
NativeEventSink broadcast → event_bus.rs bus loop
    │ domain_event_to_wire → ("im:connection:established", { connectionId })
    │ app.emit_to("main", "im:__bus__", envelope)
    │
    ▼
前端 ImEventBusDispatcher → "im:connection:established" → TauriService.setConnectionId(id)
    │
    ▼
tauri invoke "set_connection_id" → helix-tauri commands → HttpState::set_header("X-Connection-Id", id)
```

**关键**：`connectionId` 回注 HTTP header 发生在 Tauri 宿主层（`HttpState::set_header`），不在 core 层——保持 core 对 HTTP header 机制无感知。

---

## 9. 前端 pendingSends 队列 + businessReady gate

握手期（WS 连接中 / hello 未收到）前端发消息需要缓冲。这个逻辑在前端 Angular 侧（TauriService），不在 Rust：

```typescript
// src/app/core/services/tauri/tauri.service.ts（前端，仅说明对应关系）
//
// businessReady: boolean
// pendingSends: QueuedSend[]
//
// 收到 "im:connection:established" → businessReady = true → flush pendingSends
// businessReady=false 期间：invoke("send_message") 先进 pendingSends
```

Rust 侧不需要 `pendingSends` 机制，因为 `engine.step(Tick::AppCommand(Send(...)))` 在 WS 断连时会产出 `Effect::Send(frame)`，driver 的 `FfiTransport::send` 返回 `Err(NotConnected)`，effect executor 把失败喂回 engine（`Tick::SendFailed { ... }`），engine 内核产出 `Effect::Persist { sendStatus=unSend }` effect——整个失败路径在 Rust 侧是确定性的。前端 `pendingSends` 是 UX 层防抖，不是可靠性机制。

---

## 10. 汇总：17 个耦合点收敛前后对比

| 耦合点 | 收敛前 | 收敛后 |
|---|---|---|
| `common.rs:44` `AppHandle` 传入 handler | handler 直接 emit | handler 推 `Effect::Emit(DomainEvent)` |
| `post.rs:104` emit received | 直接 `app.emit_to` | `ctx.push(Effect::Emit(PostReceived {...}))` |
| `post.rs:462` emit sync result | 直接 `app.emit_to` | `ctx.push(Effect::Emit(PostBatchUpdated {...}))` |
| `post.rs:555` emit cursor | 直接 `app.emit_to` | `ctx.push(Effect::Emit(ChannelUpdateByPost {...}))` |
| `post.rs:640` emit too_long | 直接 `app.emit_to` | `ctx.push(Effect::Emit(SyncTooLong {...}))` |
| `event_bus.rs:45` emit_im | 单点收口（保留） | 唯一 wire 名出口（不变） |
| `message_loop.rs:33` AppHandle 注入 | 通过函数参数 | 通过 `EventSink` port（driver 注入） |
| `message_loop.rs:318` lifecycle emit | 直接 emit `ws:lifecycle` | `DomainEvent::Connected/Disconnected` → bus loop |
| `commands.rs` 15 个命令 | 已是薄壳 | 保持薄壳形态，参数类型优化 |

**收敛验证命令**：

```bash
# 收敛完成后，以下 grep 必须全部 0 行
grep -rn 'use tauri' crates/helix-im/src/
grep -rn 'AppHandle' crates/helix-im/src/
grep -rn '\.emit("im:' crates/ --include='*.rs' | grep -v event_bus.rs
```

---

## 11. 踩坑备档

1. **`generate_handler!` 是编译期检查**：函数签名不匹配会在 `cargo check` 阶段报错，不是运行时 panic——这是 Tauri 相比 inventory 注册的优势。
2. **`emit_to` vs `emit`**：`emit` 发给所有窗口，`emit_to(label)` 按 label 发给特定窗口。IM 事件必须用 `emit_to("main")`，防止多窗口场景下事件广播到错误窗口。
3. **State 泛型约束**：`tauri::State<'_, T>` 要求 `T: Send + Sync + 'static`。`Arc<Mutex<CoreEngine>>` 满足；裸 `Mutex<CoreEngine>` 不满足（不 `Sync`）；`Rc<RefCell<>>` 不满足（不 `Send`）。
4. **bus loop lagged**：`tokio::sync::broadcast::channel(1024)` 的 capacity 是 1024。若 bus loop 消费比 engine 产出慢（如 Tauri serialize 很慢），会触发 `RecvError::Lagged`，丢失最老的 `n` 条事件。监控 lagged warn 日志；若频繁出现，增大 capacity 或换 `unbounded`（注意内存）。
5. **capabilities 是 Tauri 2 硬约束**：不配置 permission 的命令在前端 invoke 时返回 `Error: command not allowed`，不是 404。首次接入时把 capabilities 调好再测功能。
