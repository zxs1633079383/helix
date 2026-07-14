# Helix 三端 Rust 业务门面与平台适配设计

> 状态：2026-07-14 用户拍板，待按阶段实施。
>
> 决策真源：[`../harness/decisions.md`](../harness/decisions.md) ADR-014。
>
> 本文只定义代码模块、依赖方向、迁移顺序与验收合同；具体 IM wire/投影仍以代码、
> `projection-schema.md` 和当前 UC 真源为准。

## 1. 一句话方案

三端只调用 `helix-im` 提供的 Rust 业务门面；`helix-core + helix-im` 共享全部业务语义，
PC/Flutter 继续使用 `helix-driver-host` 的默认 SQLite/HTTP/WS 实现，Web 使用独立
`helix-driver-web` 兑现内存会话存储、Rust Browser Fetch 和浏览器物理 WebSocket bridge。

不新增 `helix-client` / `helix-runtime` 之类共享大 crate，不把 Web 条件分支塞进
`helix-driver-host`，不复制 IM query/WS/sync 状态机。

## 2. 当前代码事实与真正缺口

### 2.1 已经正确的部分

- `helix-core::ExecutionShell` 已是 sans-IO：输入 `Tick`，输出 `Effect`。
- `helix-im::query` 已有异步查询主路径：command → `Effect::Persist{Scan}` →
  `PortReply` → DomainEvent。
- `HttpRequester` 已把业务 HTTP 意图与真实客户端分离；Web 只需新增浏览器实现。
- `Tick::Connected/Inbound/Disconnected` 已能表达平台连接事件，WS 业务处理无需重写。
- `helix-driver-host::run_engine_loop` 已被 native/FFI 共用，是 PC/Flutter 的有效默认实现。

### 2.2 需要修正的部分

1. `helix-driver-ffi::helix_query` 通过 `helix-im::mobile_query` 直接执行 Storage scan，
   与 `helix-im::query` 形成第二条查询真源；local-first/remote fallback 无法三端共享。
2. `mobile_contract` 的命令别名、payload 归一化已经是公共客户端契约，却以 mobile 命名，
   native/Web 无法自然把它当三端唯一入口。
3. core `Transport` 同时包含 `connect/send/recv/close`；业务 Effect 实际只需要 outbound
   `send`，物理连接生命周期不应成为所有平台必须实现的 core 能力。
4. host pump 依赖 tokio、`Send + Sync`、rusqlite、reqwest 和 tungstenite，适合 PC/Flutter，
   不应为了 Web 改造成大量 `cfg` 的万能执行器。
5. 本地读取只有 rows，没有显式能力/覆盖状态；空数组不能同时代表“真无数据”“缓存 miss”
   和“平台不支持本地存储”。

## 3. 目标模块图

```text
业务 UI
  │  invoke/query（业务名 + versioned payload）
  ▼
helix-im::client_api
  │  归一化、校验、生成 AppCommand
  ▼
helix-im::use cases / query / ws / sync
  │  Tick → Effect / PortReply → DomainEvent
  ▼
helix-core::ExecutionShell
  │
  ├─ PC/Flutter ── helix-driver-host ── SQLite + reqwest + SharedWsClient
  │                  ▲                  ▲
  │             native 薄壳         FFI 薄壳
  │
  └─ Web ─────── helix-driver-web ─── WebSessionStore + Browser Fetch
                                      + ExternalWsBridge
```

依赖只能是：

```text
helix-im              → helix-core
helix-driver-host     → helix-core
helix-driver-native   → helix-core + helix-im + helix-driver-host
helix-driver-ffi      → helix-core + helix-im + helix-driver-host
helix-driver-web      → helix-core + helix-im
```

禁止：

```text
helix-core            → 任意 driver / 业务模块
helix-im              → 任意 driver
helix-driver-web      → helix-driver-host
UI                    → 业务 HTTP / WS parser / cursor / sync
```

## 4. 各 crate 调整

### 4.1 `helix-core`：缩窄 WS port，不引入 Web 逻辑

新增：

```text
crates/helix-core/src/ports/frame_sender.rs
```

目标接口：

```rust
pub trait FrameSender: MaybeSend + MaybeSync + 'static {
    async fn send(&self, frame: Bytes) -> Result<(), PortError>;
}
```

迁移规则：

1. 先新增 `FrameSender`；为现有 host transport 提供兼容实现。
2. `helix-driver-host::TransportTable`、`dispatch_effects` 和 `run_engine_loop` 的泛型约束
   从 `Transport` 改为 `FrameSender`。
3. `connect/recv/close` 移出 core，在 host driver 内形成私有 `ManagedConnection` 或具体实现。
4. native、FFI、host-cli 全部迁移后，删除 core 旧 `Transport`。

core 不新增 tokio、web-sys、浏览器生命周期、重连退避或鉴权逻辑。

### 4.2 `helix-im`：成为三端唯一业务门面

新增目录：

```text
crates/helix-im/src/client_api/
├── mod.rs
├── command.rs
├── query.rs
└── capabilities.rs
```

职责：

- `command.rs`：承接现有 `mobile_contract` 的 alias、payload normalization、canonical command；
- `query.rs`：定义三端查询请求、request_id 和异步结果契约；
- `capabilities.rs`：公开平台能力描述，不包含具体 driver 类型；
- `mod.rs`：提供唯一 `build_command` / `build_query_command` 入口。

第一阶段不把现有 80+ command 强行改成一个超大 enum。公开 ABI 继续使用“业务名 +
versioned payload”，但所有平台必须经过同一份 registry/normalizer，driver 不再自己拼
`AppCommand`。

现有 `mobile_contract` 先 re-export `client_api` 保持兼容，消费方迁完后删除 mobile 命名。

### 4.3 查询：统一到现有异步主路径

现有真源保留在：

```text
crates/helix-im/src/query/
├── core.rs
├── dispatch.rs
└── ...
```

新增：

```text
crates/helix-im/src/query/local_first.rs
```

核心类型放在 `helix-im`，不污染 core：

```rust
pub enum LocalStoreMode {
    Durable,
    Session,
    Disabled,
}

pub enum LocalReadCoverage {
    Complete,
    Partial,
    Miss,
    Unsupported,
}
```

`LocalStoreMode` 由 driver 在构造 `ImModule` 时注入；`LocalReadCoverage` 由 IM 根据平台能力、
本地 scan 结果、同步水位和当前会话覆盖范围判定。`Durable` 不自动等于 `Complete`。

最近消息查询统一状态机：

```text
GetRecentMessages
  ├─ Disabled ───────────────────────────────→ HTTP remote
  └─ Durable / Session
       └─ Storage Scan
            ├─ Complete ─────────────────────→ Emit result
            ├─ Partial ──→ HTTP remote → merge/cache → Emit result
            ├─ Miss ─────→ HTTP remote → cache       → Emit result
            └─ Unsupported → HTTP remote             → Emit result
```

正确性要求：

- 空 rows 只能派生 `Miss`，不能表示 `Complete` 或 `Unsupported`；
- Web 刷新后 session coverage 清空，必须走远端；
- cursor 与 coverage 同属 IM 状态，不能由 Web 单独持久化一个超前 cursor；
- remote fallback 仍由 `helix-im` 生成 `Effect::Http`，driver 不拼业务 URL/body；
- 本地与远端结果最终走同一投影函数和同一 DomainEvent schema。

### 4.4 `helix-driver-host`：固定为 PC/Flutter 默认执行器

保留现有：

```text
HostStorage
SharedHttpClient
SharedWsClient
run_engine_loop
BoundedSpawner
TimerRegistry
BatchSink
```

调整：

- `run_engine_loop` / `dispatch_effects` 只依赖 `FrameSender` 发帧；
- 新增 host 私有连接生命周期边界，持有 connect/recv/close/reconnect；
- native/FFI 都装配 `LocalStoreMode::Durable`；
- 不加入 wasm/web-sys feature，不为 Web 添加 `cfg` 分支。

### 4.5 `helix-driver-native`：Tauri 薄壳

只保留：

- 把 Tauri invoke 转交 `helix-im::client_api`；
- 注入登录态/header/network config；
- 启停 host executor；
- 把 DomainEvent 批次交给 Tauri emit。

不得保留命令 alias、query SQL、remote fallback 或 WS parser 副本。

### 4.6 `helix-driver-ffi`：C ABI 薄壳

`helix_command` 保留 ABI，但内部 `build_app_command` 搬到 `helix-im::client_api`。

`helix_query` 的迁移：

1. 先标记为 compatibility API；
2. 新客户端统一通过异步 query command + DomainEvent 获取结果；
3. 兼容期仍可直接返回旧结果，但 query 解析/shape 必须复用 `helix-im::query` 真源；
4. 移动端完成切换后删除 `mobile_query` 和同步直查 Storage 旁路。

FFI 只负责指针、buffer、错误码、批量事件回调和线程边界。

### 4.7 新 `helix-driver-web`：纯 Rust Web 平台壳

新 crate 在共享边界完成后创建：

```text
crates/helix-driver-web/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── api.rs
    ├── executor.rs
    ├── browser_http.rs
    ├── session_store.rs
    ├── external_ws.rs
    ├── event_sink.rs
    └── timer.rs
```

职责：

- `api.rs`：WASM 业务 invoke/query 和平台 bridge API；
- `executor.rs`：用浏览器单线程事件循环驱动同一 `ExecutionShell`；
- `browser_http.rs`：Rust 通过 Browser Fetch 兑现 `HttpRequester`；
- `session_store.rs`：Rust 内存存储，只在页面会话内有效，按表/作用域有界；
- `external_ws.rs`：接收浏览器 open/frame/close，输出待发送帧；
- `event_sink.rs`：批量发布 versioned DomainEvent；
- `timer.rs`：浏览器 timer 兑现 `ScheduleTimer`。

Web UI 业务代码只调用业务 API。只有平台 bootstrap glue 可以调用：

```text
ws_connected
ws_frame
ws_disconnected
take_platform_actions / on_send_frame
```

这些是基础设施接线，不是业务接口；它们不得解析 IM 帧或维护 cursor/echo/sync。

首版明确不做 IndexedDB/OPFS。若以后增加跨刷新缓存，必须另行拍板容量、coverage 与 cursor
联合检查点，不改变 `helix-im` 的业务接口。

### 4.8 历史 `helix-driver-wasm`

不在原 crate 上继续开发。新 `helix-driver-web` 通过验收后：

1. 审计历史代码，只迁移被测试证明可复用的无业务片段；
2. 单独提交删除 `helix-driver-wasm`；
3. HX-C003 改为守护正式 `helix-driver-web`，不再指向历史骨架。

## 5. 分阶段实施

### A1：统一业务门面，不改行为

修改：`helix-im/client_api`、`mobile_contract` 兼容 re-export、FFI/native 调用入口。

验收：同一 public name/payload 在 native/FFI 生成字节级一致 `AppCommand`；现有 command/event
contract 全绿。

### A2：查询旁路收口 + local-first 状态机

修改：`query/local_first.rs`、`query/dispatch.rs`、`correlation.rs`、`port_reply.rs`、`state.rs`、
`mobile_query.rs` 和 FFI compatibility query。

验收剧本至少覆盖：

- Durable + Complete：只读本地，零 HTTP；
- Durable + Partial：本地后远端，结果去重有序；
- Session + Miss：远端回源并写入 session store；
- Disabled/Unsupported：不发 Storage effect，直接 HTTP；
- HTTP 失败：返回明确业务错误/状态，不把空列表冒充成功；
- 破坏 coverage 判定时测试必须失败。

### A3：WS port 缩窄

先新增 `FrameSender`，迁移 host engine/dispatch，再迁移 native/FFI/host-cli，最后删除旧
core `Transport`。

验收：相同 Connected/Inbound/Disconnected 剧本的 Effect/DomainEvent 与迁移前一致；
host reconnect 行为不变；core 依赖树仍零运行时。

### A4：创建 Web Rust 平台壳

实现 `helix-driver-web` 的 BrowserHttpRequester、WebSessionStore、ExternalWsBridge、timer、
event batch 与 WASM API，不实现前端页面。

验收：wasm target 编译；同 Tick 剧本 native/FFI/Web 业务输出一致；Web 无 SQLite、无业务
fetch/WS parser 副本。

### A5：真实链路与清理

用最小平台 glue 验证 hello → 收帧 → query → HTTP fallback → send → echo → disconnect/reconnect；
激活 HX-C003/CI 后删除历史 wasm crate 和 FFI `mobile_query` 旁路。

## 6. 提交边界

每阶段独立提交，禁止把下列变化混成一个大提交：

1. client API 纯迁移；
2. local-first 行为变化；
3. core WS public API 变化；
4. host/native/FFI 适配；
5. Web 新 crate；
6. 历史代码删除。

每个阶段先以回放/契约测试固定旧行为或新决策，再改实现；涉及 core public API 的阶段必须
跑双端验收、public-api diff 与零运行时证明。

## 7. 最终完成定义

- 三端 UI 不直接组织业务 HTTP；
- 三端使用同一 command/query contract 和同一 DomainEvent schema；
- local-first/remote fallback 只存在于 `helix-im`；
- PC/Flutter 使用同一个 host 默认 SQLite/HTTP/WS 生命周期实现；
- Web 使用 Rust Browser Fetch，浏览器只管理物理 WS；
- WS 帧解析、sequence/cursor、echo、sync 三端只有一份；
- Web 刷新不保留无法由数据 coverage 证明的 cursor；
- `helix-core` 仍零 runtime/platform 依赖；
- 历史 `helix-driver-wasm` 与 `mobile_query` 重复旁路完成清理。
