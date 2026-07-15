# helix-driver-native 与 helix-host-cli 架构

> `helix-driver-native` 是 PC/Tauri 的具体端口薄壳；`helix-host-cli` 是复用它的无 UI 联调宿主和 SQLite 检查工具。两者不拥有 IM 业务规则。

## 1. `helix-driver-native` crate 模块

### 模块类图

```mermaid
classDiagram
  class EngineConfig
  class NativeStorage
  class NativeHttp
  class NativeTransport
  class NativeEventSink
  class NativeClock
  class HostStorage
  class SharedHttpClient
  class SharedWsClient
  class HostEngine
  EngineConfig *-- NativeStorage : 装配存储
  EngineConfig *-- NativeHttp : 装配HTTP
  EngineConfig *-- NativeEventSink : 装配事件出口
  EngineConfig *-- NativeClock : 注入系统时间
  NativeStorage *-- HostStorage : 委托共享实现
  NativeHttp *-- SharedHttpClient : 委托共享实现
  NativeTransport *-- SharedWsClient : 委托共享实现
  EngineConfig --> HostEngine : 转换为EngineDeps
```

### 模块流程图

```mermaid
flowchart TD
  A["PC 或 host-cli 构造具体端口"] -->|聚合为 EngineConfig| B["native 薄壳"]
  B -->|转换为共享 EngineDeps| C["helix-driver-host 事件泵"]
  C -->|请求时间| D["NativeClock"]
  C -->|持久化| E["NativeStorage 转发 HostStorage"]
  C -->|发送 HTTP| F["NativeHttp 转发共享客户端"]
  C -->|发送或接收 WS| G["NativeTransport 转发 SharedWsClient"]
  C -->|发布领域事件| H["NativeEventSink broadcast"]
```

## 2. `helix-driver-native/src/` 文件夹

### 文件职责

| 文件 | 关键类型 | 作用 |
|---|---|---|
| `lib.rs` | facade | 导出 native adapters 和 host 兼容 API |
| `clock.rs` | `NativeClock` | 用 `SystemTime` 实现 `Clock` |
| `storage.rs` | `NativeStorage` | 完整转发 8 个 Storage 操作到 `HostStorage` |
| `http.rs` | `NativeHttp` | Plain/Cross 两种形态，当前 host-cli 实际使用 crosscut |
| `transport.rs` | `NativeTransport` | 包装 shared WS、握手头、入站 Tick、幂等断开 |
| `engine_loop.rs` | `EngineConfig`、`TransportTable` | 只装配具体端口并调用 host pump |
| `event_sink.rs` | `NativeEventSink`、`EventReceiver` | Tokio broadcast，显式暴露 Lagged |
| `bus_guard.rs` | `GuardedEnvelope` | 防 `im:__*` 自环并归一数组 data |

### `src/` 类图

```mermaid
classDiagram
  class Clock {
    <<interface>>
  }
  class Storage {
    <<interface>>
  }
  class HttpRequester {
    <<interface>>
  }
  class Transport {
    <<interface>>
  }
  class BatchSink {
    <<interface>>
  }
  class NativeClock
  class NativeStorage
  class NativeHttp
  class NativeTransport
  class NativeEventSink
  class EventReceiver
  class GuardedEnvelope
  Clock <|.. NativeClock : 实现时间端口
  Storage <|.. NativeStorage : 实现存储端口
  HttpRequester <|.. NativeHttp : 实现请求端口
  Transport <|.. NativeTransport : 实现传输端口
  BatchSink <|.. NativeEventSink : 实现批次出口
  NativeEventSink --> EventReceiver : 创建订阅者
  NativeEventSink --> GuardedEnvelope : 转换总线信封
```

### WebSocket 入站状态图

```mermaid
stateDiagram-v2
  state "未连接" as Disconnected
  state "正在连接" as Connecting
  state "已连接" as Connected
  state "处理入站帧" as Reading
  state "连接失败" as Failed
  state "已断开" as Closed
  [*] --> Disconnected: 创建传输适配器
  Disconnected --> Connecting: 注入握手头并发起连接
  Connecting --> Connected: 连接成功并发送已连接输入
  Connecting --> Failed: 连接返回错误
  Connected --> Reading: 共享读取任务收到帧
  Reading --> Connected: 发送入站 Tick
  Connected --> Closed: 连接关闭且首次发送断开 Tick
  Reading --> Closed: 读取错误且首次发送断开 Tick
  Failed --> [*]: 返回装配方处理
  Closed --> [*]: 连接生命周期结束
```

### native 事件出口流程图

```mermaid
flowchart LR
  A["Effect::Emit"] -->|调用同步 emit| B["NativeEventSink"]
  B -->|防止内部总线自环| C["bus_guard"]
  C -->|合法事件| D["转换为 channel/payload 信封"]
  D -->|广播领域事件| E["所有 EventReceiver"]
  E -->|正常消费| F["Tauri 或联调观察器"]
  E -->|消费者落后| G["显式返回 Lagged 数量"]
  C -->|非法内部频道| H["拒绝发布"]
```

## 3. `helix-driver-native/tests/` 文件夹

### 验证类图

```mermaid
classDiagram
  class EngineIntegrationTests
  class DriverSeamTests
  class StressEngineTests
  class StressStorageTests
  class TransportInboundTests
  class NativeTransport
  class NativeStorage
  class HostEngine
  EngineIntegrationTests --> HostEngine
  DriverSeamTests --> NativeStorage
  DriverSeamTests --> NativeTransport
  StressEngineTests --> HostEngine
  StressStorageTests --> NativeStorage
  TransportInboundTests --> NativeTransport
```

### 验证流程图

```mermaid
flowchart TD
  A["构造临时 SQLite、假 HTTP、假 WS 和 Tick 通道"] -->|启动 native 薄壳| B["复用 host pump"]
  B -->|喂入命令、帧和计时器| C["采集 PortReply、事件和数据库终态"]
  C -->|验证 adapter 行为| D["端口接缝合同"]
  C -->|增加并发与负载| E["压力不变量"]
  D -->|全部满足| F["native 接缝通过"]
  E -->|无死锁且写入完整| F
```

## 4. `helix-driver-native/benches/` 文件夹

### 基准类图

```mermaid
classDiagram
  class EngineBench
  class StorageUpsertBench
  class HostEngine
  class NativeStorage
  EngineBench --> HostEngine : 测量pump往返
  StorageUpsertBench --> NativeStorage : 测量upsert
```

### 基准流程图

```mermaid
flowchart LR
  A["准备固定端口和数据规模"] -->|预热运行时与数据库| B["重复执行pump往返或upsert"]
  B -->|记录统计分布| C["Criterion 报告"]
  C -->|对比基线| D["判断 native 装配是否退化"]
```

## 5. `helix-driver-native/docs/design/` 文件夹

### 文档类图

```mermaid
classDiagram
  class CurrentNativeArchitecture
  class LegacyNativeDesign
  class NativeSource
  class HostSource
  CurrentNativeArchitecture --> NativeSource : 读取当前薄壳
  CurrentNativeArchitecture --> HostSource : 读取真实pump核
  CurrentNativeArchitecture --> LegacyNativeDesign : 仅作历史背景
```

### 阅读流程图

```mermaid
flowchart LR
  A["先读本页"] -->|理解薄壳边界| B["读 engine_loop 与 transport"]
  B -->|追踪真实pump| C["转到 helix-driver-host engine 与 dispatch"]
  C -->|需要历史图| D["按需读 native docs/design"]
```

## 6. `helix-host-cli` crate 模块

### 模块类图

```mermaid
classDiagram
  class Config
  class DbPathInputs
  class LocalStoreLocator
  class NativeStorage
  class NativeHttp
  class NativeTransport
  class NativeEventSink
  class ExecutionShell
  class ImModule
  class Repl
  class StoreInspect
  Config --> DbPathInputs : 提供身份与运行参数
  DbPathInputs --> LocalStoreLocator : 派生租户库
  Config --> NativeHttp : 注入网关和身份头
  Config --> NativeTransport : 注入WS和握手头
  NativeStorage --> ExecutionShell : 由pump兑现持久化
  NativeHttp --> ExecutionShell : 由pump兑现HTTP
  NativeTransport --> ExecutionShell : 提供入站与发送
  NativeEventSink --> Repl : 观察领域事件
  ExecutionShell o-- ImModule : 注册业务模块
  StoreInspect ..> LocalStoreLocator : 定位只读数据库
```

### 模块装配流程图

```mermaid
flowchart TD
  A["启动 helix-host-cli"] -->|加载 YAML| B["Config"]
  B -->|应用调试逃生覆盖| C["应用 CLI 最高优先级覆盖"]
  C -->|按账号和后端派生| D["tenant.db"]
  D -->|打开并执行 IM schema| E["NativeStorage"]
  C -->|注入身份、鉴权与共享头| F["NativeHttp 和 NativeTransport"]
  E -->|注册具体端口| G["EngineConfig"]
  F -->|注册具体端口| G
  G -->|注册 ImModule| H["ExecutionShell"]
  H -->|连接主 WS 后进入| I["共享事件泵"]
  J["stdin REPL"] -->|解析为 Tick::Command| I
  I -->|领域事件广播| K["日志观察器"]
  J -->|quit 或 EOF| L["优雅关闭并排空"]
```

## 7. `helix-host-cli/src/` 文件夹

### 文件职责

| 文件 | 作用 |
|---|---|
| `main.rs` | 配置、租户库、schema、HTTP/WS、IM、REPL、pump 总装配 |
| `config.rs` | YAML < debug env < CLI 覆盖模型 |
| `config_observability.rs` | OTel 配置模型；当前 main 尚未消费，不能声称已生效 |
| `config_tests.rs` | 配置默认值与覆盖测试 |
| `host_env.rs` | 默认业务网关、DB 路径和 device id 派生 |
| `cmd_parse.rs` | send/read/schedule/raw 等专用命令解析 |
| `cmd_parse_generic.rs` | `cmd <name> <json>` 通用逃生口 |
| `cmd_parse_member.rs` | nickname/role/notify 成员命令 |
| `repl.rs` | stdin 异步循环、错误不崩泵、quit/EOF 关闭 |
| `lib.rs` | 只公开 store inspect |
| `store_inspect.rs` | allowlist、只读 SQLite、脱敏和路径哈希 |
| `store_inspect_main.rs` | `helix-store-inspect` 二进制入口 |

### `src/` 配置类图

```mermaid
classDiagram
  class Config
  class Connection
  class Identity
  class Client
  class Runtime
  class Observability
  class Otel
  class CliArgs
  Config *-- Connection
  Config *-- Identity
  Config *-- Client
  Config *-- Runtime
  Config o-- Observability
  Observability o-- Otel
  Config --> CliArgs : 应用最高优先级覆盖
```

### `src/` 运行状态图

```mermaid
stateDiagram-v2
  state "读取配置" as LoadConfig
  state "应用调试覆盖" as DebugOverride
  state "应用命令行覆盖" as CliOverride
  state "派生租户数据库" as ResolveDb
  state "初始化存储与建表" as InitStorage
  state "注入身份和鉴权" as InjectIdentity
  state "注册IM模块" as RegisterIm
  state "连接主WebSocket" as ConnectWs
  state "进入事件泵" as Running
  state "解析终端输入" as Repl
  state "优雅退出" as Shutdown
  [*] --> LoadConfig: 启动程序
  LoadConfig --> DebugOverride: 调试构建存在非空环境变量
  LoadConfig --> CliOverride: 没有调试覆盖
  DebugOverride --> CliOverride: 调试覆盖完成
  CliOverride --> ResolveDb: 最终配置确定
  ResolveDb --> InitStorage: 得到数据库路径
  InitStorage --> InjectIdentity: schema 初始化完成
  InjectIdentity --> RegisterIm: 网络端口装配完成
  RegisterIm --> ConnectWs: 执行壳就绪
  ConnectWs --> Running: 连接成功或失败均继续
  Running --> Repl: 等待终端命令
  Repl --> Running: 合法命令进入 Tick 通道
  Repl --> Repl: 非法命令打印诊断
  Repl --> Shutdown: 输入退出命令或文件结束
  Shutdown --> [*]: 事件泵排空完成
```

### store inspect 类图

```mermaid
classDiagram
  class InspectRequest
  class InspectResult
  class InspectError {
    <<enumeration>>
  }
  class LocalStoreLocator
  class SqliteReadOnlyConnection
  InspectRequest --> LocalStoreLocator : 定位账号数据库
  InspectRequest --> SqliteReadOnlyConnection : 只读查询allowlist表
  SqliteReadOnlyConnection --> InspectResult : 返回脱敏行
  InspectError ..> InspectRequest : 拒绝未知表或无界扫描
```

### store inspect 流程图

```mermaid
flowchart TD
  A["解析检查请求"] -->|校验账号、表、过滤字段和 limit| B{"请求是否在 allowlist 内"}
  B -->|否| C["返回结构化拒绝错误"]
  B -->|是| D["定位目标账号 tenant.db"]
  D -->|只读打开| E["执行有界 SQLite 查询"]
  E -->|处理内容列与二进制| F["脱敏正文和 Blob"]
  F -->|隐藏真实路径| G["只返回路径 SHA-256"]
  G -->|序列化| H["输出 JSON 检查结果"]
```

## 8. `helix-host-cli/tests/` 文件夹

### 验证类图

```mermaid
classDiagram
  class StoreInspectTests
  class InspectRequest
  class LocalStoreLocator
  class SqliteFixture
  StoreInspectTests --> InspectRequest : 构造合法非法请求
  StoreInspectTests --> LocalStoreLocator : 创建账号数据库
  StoreInspectTests --> SqliteFixture : 写入敏感样例
```

### 验证流程图

```mermaid
flowchart LR
  A["创建账号隔离的 SQLite fixture"] -->|写入消息与敏感字段| B["运行 store inspect"]
  B -->|合法请求| C["断言只读取目标账号并完成脱敏"]
  B -->|未知表或无界请求| D["断言立即拒绝"]
```

## 9. `helix-host-cli/docs/design/` 文件夹

### 文档类图

```mermaid
classDiagram
  class CurrentCliArchitecture
  class AssemblyAndConnectDiagram
  class ConnectHandshakeDiagram
  class CliSource
  CurrentCliArchitecture --> CliSource : 以当前main为准
  CurrentCliArchitecture --> AssemblyAndConnectDiagram : 补充装配细节
  CurrentCliArchitecture --> ConnectHandshakeDiagram : 补充握手细节
```

### 阅读流程图

```mermaid
flowchart LR
  A["先读本页装配全景"] -->|定位 main 入口| B["阅读 main、config、repl"]
  B -->|需要连接时序细节| C["阅读 assembly-and-connect.mmd"]
  C -->|需要握手字段| D["阅读 connect-handshake.mmd"]
  D -->|发现命令数量文案| E["回到 helix-im registry live test 裁决"]
```

REPL 中任何硬编码命令数量都不是真源；当前出站命令总数应以 `helix-im` registry 测试为准。`config_observability.rs` 能解析 OTel，但当前 `main.rs` 尚未消费该配置。
