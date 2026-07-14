# Helix 三端 Rust 业务门面与平台适配设计

> 状态：2026-07-14 用户拍板；A1-A4 已落地，下一阶段 A5 真链路与清理。
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
- `HttpRequester` 已把业务 HTTP 意图与真实客户端分离；A4 已由 Web Browser Fetch 实现兑现。
- `Tick::Connected/Inbound/Disconnected` 已能表达平台连接事件，WS 业务处理无需重写。
- `helix-driver-host::run_engine_loop` 已被 native/FFI 共用，是 PC/Flutter 的有效默认实现。

### 2.2 拍板时需要修正的部分（A1-A4 已收口）

1. ✅ A2：`helix-driver-ffi::helix_query` 曾通过 `helix-im::mobile_query` 直接执行 Storage scan，
   与 `helix-im::query` 形成第二条查询真源；local-first/remote fallback 无法三端共享。
2. ✅ A1：`mobile_contract` 的命令别名、payload 归一化已经是公共客户端契约，却以 mobile 命名，
   native/Web 无法自然把它当三端唯一入口。
3. ✅ A3：core 旧 `Transport` 同时包含 `connect/send/recv/close`；业务 Effect 实际只需要 outbound
   `send`，物理连接生命周期不应成为所有平台必须实现的 core 能力。
4. ✅ A4：host pump 依赖 tokio、`Send + Sync`、rusqlite、reqwest 和 tungstenite，适合 PC/Flutter；
   Web 已以独立 `helix-driver-web` 兑现平台 I/O，没有把 host 改造成大量 `cfg` 的万能执行器。
5. ✅ A2：本地读取曾只有 rows，没有显式能力/覆盖状态；现已用 `LocalStoreMode` 与
   `LocalReadCoverage` 区分真无数据、缓存 miss 和平台不支持本地存储。

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

已新增：

```text
crates/helix-core/src/ports/frame_sender.rs
```

目标接口：

```rust
pub trait FrameSender: MaybeSend + MaybeSync + 'static {
    async fn send(&self, frame: Bytes) -> Result<(), PortError>;
}
```

落地结果：

1. core `FrameSender` 只定义 `send(frame)`，并保持 native `Send` / wasm `?Send` 的 additive cfg。
2. `helix-driver-host::TransportTable`、`dispatch_effects` 和 `run_engine_loop` 只约束 `FrameSender`。
3. `SharedWsClient` 与 `NativeTransport` 以具体类型的 inherent 方法管理 `connect/recv/close`，
   reconnect 继续属于 host；未为尚不存在的第二套生命周期预建 `ManagedConnection` trait。
4. native、FFI、host-cli 与归档 wasm 骨架已迁移，core 旧 `Transport` 已删除。

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
crates/helix-im/src/query/
├── local_first.rs
└── local_first/data.rs
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
- `Complete` 必须有当前连接会话内的显式窗口证明：成功取得权威远端窗口、cache Persist 成功，
  且下一次本地 Scan 的消息身份序列与该窗口相符；“本地行数达到 limit”本身不是证明；
- 已知 `cursor < increment_target`、channel 仍有 inflight sync 或 gate buffer 时，即使身份相符也只能
  判 `Partial`；hello/disconnect 会清空 proof，旧 HTTP/Persist 晚回包不得污染新会话；
- Web 刷新后 session coverage 清空，必须走远端；
- cursor 与 coverage 同属 IM 状态，不能由 Web 单独持久化一个超前 cursor；
- remote fallback 仍由 `helix-im` 生成 `Effect::Http`，driver 不拼业务 URL/body；
- 本地与远端结果最终走同一投影函数和同一 DomainEvent schema。

当前最近消息远端真源固定为 `POST /posts/getLatestPost`：服务端先取当前用户可见的最近 20 条，
满 20 时可能补齐同 segment，所以响应可多于 20。现有 query ABI 仍兼容 `limit ≤ 500`，但单次
远端响应只证明最近 20 基础窗口：`limit > 20` 时可以返回本地与远端的去重合并结果，却不得把它
记录成 `Complete`；只有服务端明确返回不足 20（证明频道已耗尽）时，较大 limit 才可能完整。

`im_query_dialog_list` 暂不套用 HTTP fallback：`/channels/load/increment` 的真实数据经 WS 异步推送，
`/channel/query` 又不等价当前用户的 unread/mention dialog projection。会话列表继续走本地/会话投影，
待 WS `increment_channel_end` 能形成显式 dialog coverage 后再扩展，禁止伪造同步远端接口。

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

当前 `helix-driver-native` 只有 engine/port/event seam，尚无 public-name/payload 或真实 Tauri
command 入口。A1 不为验收凭空增加 wrapper 或 `helix-im` 依赖；真实 Tauri 入口落地时必须直接
调用 `helix-im::client_api`。

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

A4 已在共享边界完成后创建：

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

A4 落地结果：Browser Fetch 支持完整 body、显式 timeout 与 shutdown abort；page-session store 按
表/作用域有界且不跨刷新；ExternalWsBridge 只搬运连接生命周期和原始帧；浏览器 timer、有界
event batch v2 与 WASM API 均已接入。HTTP outcome 到业务事件的映射由 `helix-im` 单一持有，host/Web
只注入 observer；事件/动作/运行时问题轮询队列均有硬上限，溢出通过 gap/drop 计数或 fail-closed
显式暴露。`wasm32-unknown-unknown` final artifact 已构建通过，相关六 crate
全测通过，同 Tick native/FFI/Web 业务输出 parity 通过；这不等价于 A5 真浏览器/真 Go 产品链完成。

`helix-im` 原先用 `linkme` 分散注册 HTTP command 与 WS handler，但 `linkme` 不支持
`wasm32-unknown-unknown`。A4 将注册机制全平台单轨迁为 `inventory`，避免为 Web 复制第二份注册表；
WASM constructors 的显式初始化只属于 `helix-driver-web` final artifact/bootstrap，不能下沉到
`helix-im`，业务模块仍不认识浏览器生命周期。

### 4.8 历史 `helix-driver-wasm`

不在原 crate 上继续开发。A4 `helix-driver-web` 已通过 Rust/WASM 验收；以下清理仍留到 A5
真链路与正式 CI gate 一并完成：

1. 审计历史代码，只迁移被测试证明可复用的无业务片段；
2. 单独提交删除 `helix-driver-wasm`；
3. HX-C003 改为守护正式 `helix-driver-web`，不再指向历史骨架。

## 5. 分阶段实施

### A1：统一业务门面，不改行为 ✅（2026-07-14）

已实现：`helix-im/client_api`、`mobile_contract` 兼容 re-export、FFI 薄绑定。native 当前没有
同形公开调用入口，因此未增加无消费方的 wrapper；后续真实 Tauri 入口直接复用共享 API。

验收：共享/兼容路径对同一 public name/payload 生成字节级一致 `AppCommand`；known command
保持 borrowed、控制/未知 command 保持 owned fallback；非法 payload 映射 `FFI_INVALID`；现有
command/event contract 全绿。

### A2：查询旁路收口 + local-first 状态机 ✅（2026-07-14）

修改：`query/local_first.rs`、`query/local_first/data.rs`、`query/dispatch.rs`、`correlation.rs`、
`port_reply.rs`、`state.rs`、`mobile_query.rs` 和 FFI compatibility query。

实现边界：FFI 同步 `helix_query` 在 A5 消费方迁移前仍直接执行 Storage，但 payload 解析、ScanSpec、
结果 shape 必须委托 `helix-im::query`；它不在同步 ABI 内另造 HTTP 编排。真正的 local-first 只走
`Tick::Command → Effect → PortReply → DomainEvent` 异步主路径。

验收剧本至少覆盖：

- Durable + Complete：只读本地，零 HTTP；
- Durable + Partial：本地后远端，结果去重有序；
- Session + Miss：远端回源并写入 session store；
- Disabled/Unsupported：不发 Storage effect，直接 HTTP；
- HTTP 失败：返回明确业务错误/状态，不把空列表冒充成功；
- 破坏 coverage 判定时测试必须失败。

落地结果：异步查询已形成 `Scan → coverage → 可选 Http → 可选 cache Persist → Emit` 单一路径；
远端不足 20 条才可证明当前用户可见集合耗尽，满 20 后的同 segment 扩展行不扩大 coverage。
hello/disconnect、消息写入、sync 与乐观发送都会撤销旧 proof；旧会话 Scan/HTTP/Persist 晚回包均丢弃。
FFI 同步 compatibility 只复用共享 parser/ScanSpec/shape，不复制 HTTP 编排，按计划留到 A5 删除。

验证：五 crate `cargo check` 通过；`scripts/gate.sh` 10/10 通过，含全 workspace 测试、HX-C001、
trace/业务边界、harness、FFI 1–5 层、native↔FFI byte parity、C++ 直连 ABI 冒烟与 public API 闸门。

### A3：WS port 缩窄 ✅

已新增 send-only `FrameSender`，迁移 host engine/dispatch、native/FFI/host-cli 与归档 wasm
骨架，并删除 core 旧 `Transport`。连接、收帧、关闭是具体连接类型的方法，reconnect 留在 host。

验收覆盖：只实现 send 的 dyn-safe core 契约；精确 TransportId/bytes 的 Send 路由；动态注册
sender 后继续发送；native 受控本地 WS 产出 Connected、原字节 Inbound 与 reader-side Disconnected，
并守住 reader 内部 Inbound→Disconnected；两份独立 shell 对相同**注入**生命周期 Tick 逐 step
产出一致并带非空业务锚；host reconnect 回归保留。

边界说明：A3 证明的是“平台把同一 Tick 剧本喂入后，业务输出一致”，当时不声称 native/FFI
物理连接时序相同；这个刻意保留的缺口已由下方 A3.1 以真实 adapter 收口，仍未反向扩大 core port。

### A3.1：真实 WS 生命周期发布屏障 ✅

`SharedWsClient::connect` 现在只完成物理握手并返回一次性 activation：sender 已可发送，但 reader
等待 gate，不会抢跑。host 动态注册消息携带 oneshot ack，engine 严格先 `transports.insert` 再回 ack；
FFI 初连与 reconnect 复用同一个 `connect_register_activate`，收到 ack 后由 activation 先发布
Connected，再放行 reader。native host-cli 走同义的预填表路径：先 insert `Arc<NativeTransport>`，
再消费 activation。

每条物理连接的生命周期只有一份 `Prepared → Active → Finished` 状态，reader remote-close 与显式
cleanup close 都调用同一个 finish-once；close 调用方被取消或 Close 帧写失败也不再跳过
reader/state/lifecycle 清理。host engine 合同测试冻结动态注册确认；native/FFI 真实本地 WS 测试
分别冻结严格 Tick 顺序、关闭取消安全/去重，以及初连→断线→重连的两代 Tick 顺序。

### A4：创建 Web Rust 平台壳 ✅（2026-07-14）

已实现 `helix-driver-web` 的 BrowserHttpRequester（完整 body、timeout、shutdown abort）、有界
WebSessionStore、ExternalWsBridge、browser timer、有界 event batch v2 与 WASM API；未实现前端页面。
HTTP outcome 业务策略由 `helix-im` 单一持有，host/Web 只注入 observer；事件/动作/运行时问题轮询
队列都有硬上限，event batch 用 gap/drop 计数显式报告丢弃，bridge 饱和时 fail-closed 且保留原连接状态。

为解除 `linkme` 对 `wasm32-unknown-unknown` 的平台阻塞，`helix-im` 的 HTTP command/WS handler
注册单轨迁为 `inventory`；Web final artifact 通过 wasm-bindgen start 显式初始化 constructors，初始化
责任没有下沉到业务模块。

验收结果：wasm target build 通过；相关六 crate 全测通过；同 Tick 剧本 native/FFI/Web 业务输出
一致；Web 无 SQLite、无业务 fetch/WS parser/sync/cursor 副本。A4 只证明 Rust 平台壳，不冒充 A5
真浏览器/真 Go 链路。

### A5：真实链路与清理

用最小平台 glue 验证 hello → 收帧 → query → HTTP fallback → send → echo → disconnect/reconnect；
激活 HX-C003/CI 后删除历史 wasm crate 和 FFI `mobile_query` 旁路。真实 adapter 的
“注册 sender → 发布 Connected → 放行 reader”与断线去重已经由 A3.1 冻结；A5 只需在 Web
真链路继续复用该合同，不能退回手工 Tick 剧本替代物理生产者。

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
