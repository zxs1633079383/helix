# helix-driver-host 架构

> 目录：`crates/helix-driver-host`。这是 native 与 FFI 共用的宿主 I/O 和事件泵单一实现。

## 1. crate 模块

### 职责

该 crate 接收 `ExecutionShell` 产出的 Effect，将借用数据转换为 owned 数据，再兑现为 SQLite、HTTP、上传、WebSocket、计时器或事件出口，并把带 correlation 的结果回灌为 `Tick::PortReply`。

### crate 类图

```mermaid
classDiagram
  class EngineDeps
  class ExecutionShell
  class OwnedEffect {
    <<enumeration>>
  }
  class BoundedSpawner
  class Job
  class TimerRegistry
  class TransportTable
  class HostStorage
  class SharedHttpClient
  class SharedWsClient
  class SharedFileUploader
  class BatchSink {
    <<interface>>
  }
  class TraceHooks
  EngineDeps o-- HostStorage : 注入存储
  EngineDeps o-- SharedHttpClient : 注入请求器
  EngineDeps o-- SharedFileUploader : 注入上传器
  EngineDeps o-- BatchSink : 注入事件出口
  EngineDeps o-- TraceHooks : 注入逐跳探针
  ExecutionShell ..> OwnedEffect : 转换为自有对象
  BoundedSpawner o-- Job : 有界调度
  TransportTable o-- SharedWsClient : 按通道标识路由
  TimerRegistry ..> ExecutionShell : 触发计时器输入
```

### crate 主流程图

```mermaid
flowchart TD
  A["构造 EngineDeps 与传输表"] -->|初始化宿主能力| B["建立回灌通道和工作池"]
  B -->|触发业务启动| C["调用内核 start"]
  C -->|解除 scratch 借用| D["把 Effect 转为 OwnedEffect"]
  D -->|分流兑现| E["派发初始副作用"]
  E -->|合并事件出口| F["按步骤刷新事件批"]
  F -->|进入主选择循环| G{"等待哪类输入"}
  G -->|端口回复优先| H["取 PortReply"]
  G -->|传输连接注册| I["写入传输路由表"]
  I -->|继续等待| G
  G -->|收到关闭信号| N["进入平滑关闭"]
  G -->|收到普通 Tick| H
  H -->|建立逐跳作用域| J["读取当前时间"]
  J -->|同步执行纯状态机| K["调用 shell.step"]
  K -->|转换并分流| L["兑现本轮副作用"]
  L -->|本轮非空| M["刷新一次事件批"]
  M -->|继续处理| G
  N -->|保证本地写入| O["无限等待持久化队列排空"]
  O -->|限制网络关闭时间| P["上传与必达 HTTP 最多等待三秒"]
  P -->|限时内完成| Q["清空迟到回复并退出"]
  P -->|超过上限| R["中止剩余网络任务"]
  R -->|完成有界退出| Q
```

## 2. `src/` 文件夹

### 文件职责

| 文件 | 关键类型/入口 | 作用 |
|---|---|---|
| `lib.rs` | 公共 facade | 导出 20 个模块和队列容量常量 |
| `engine.rs` | `EngineDeps`、`run_engine_loop` | pump 主循环、优先级和关闭排空 |
| `owned_effect.rs` | `OwnedEffect`、`consume_effects` | 解除 core scratch 借用，允许后续 async |
| `dispatch.rs` | `dispatch_effects` | Effect 分流总入口 |
| `spawner.rs` | `BoundedSpawner`、`Overflow` | 有界 worker、Block/DropNewest 和关闭策略 |
| `pools.rs` | pool 工厂 | 创建 Persist、HTTP、上传 worker |
| `ops.rs` | `execute_storage_ops` | 顺序执行 StorageOp、结果编码和错误分类 |
| `batch_sink.rs` | `BatchSink` | 给 EventSink 增加每 step 一次 `flush` |
| `instrument.rs` | `RecordingSink` | observe-first 的事件出口装饰器 |
| `timer.rs` | `TimerRegistry` | 调度、取消、惰性回收一次性 timer |
| `codec.rs` | rows codec | SQLite row 与 PortReply JSON 互转 |
| `http_envelope.rs` | `run_http_envelope` | HTTP 响应编码为 status/headers/base64 body |
| `upsert_sql.rs` | SQL shape cache | 按表/列/conflict key 缓存 upsert SQL |
| `storage.rs` | `HostStorage`、`ReadPool` | 1 写 + 4 读、WAL、spawn_blocking、8 类操作 |
| `local_store.rs` | `LocalStoreLocator` | 多租户稳定 store key、tenant.db、manifest |
| `network_util.rs` | 私有纯函数 | URL/header 校验、合并和错误转换 |
| `network.rs` | shared HTTP/WS | 共享头、请求、push/pull WS 和状态机 |
| `http_cross.rs` | `CrossCuttingHttp` | 鉴权意图、trace、401/离线事件装饰 |
| `upload.rs` | `SharedFileUploader` | 64 KiB 流式 PUT 上传 |
| `reconnect.rs` | reconnect supervisor | 指数退避和重新注册 transport |
| `trace.rs` | `TraceCarrier`、`TraceHooks` | W3C carrier、6 个 hop、command sidecar |
| `otel.rs` | `HostOtelRuntime` | span 快照、OTLP worker 和重试 |
| `dispatch_tests.rs` | cfg test 子模块 | 分流、上传、trace 和 reply JSON |
| `spawner_tests.rs` | cfg test 子模块 | 背压、保序、超时中止与回灌 |

### `src/` 总类图

```mermaid
classDiagram
  class Storage {
    <<interface>>
  }
  class HostStorage
  class ReadPool
  class StorageTraceContext
  class HttpRequester {
    <<interface>>
  }
  class SharedHttpClient
  class CrossCuttingHttp
  class AuthTokenRegistry
  class EventSink {
    <<interface>>
  }
  class Transport {
    <<interface>>
  }
  class SharedWsClient
  class HostHeaderRegistry
  class HostNetworkConfig
  HostStorage ..|> Storage
  HostStorage *-- ReadPool : 文件库四读连接
  HostStorage ..> StorageTraceContext : 传播存储链路
  SharedHttpClient ..|> HttpRequester
  CrossCuttingHttp ..|> HttpRequester
  CrossCuttingHttp o-- SharedHttpClient : 装饰真实请求器
  CrossCuttingHttp o-- AuthTokenRegistry : 按意图取凭据
  CrossCuttingHttp o-- EventSink : 发布错误事件
  SharedWsClient ..|> Transport
  SharedHttpClient o-- HostHeaderRegistry : 读取共享头
  SharedWsClient o-- HostHeaderRegistry : 读取握手头
  SharedHttpClient o-- HostNetworkConfig
  SharedWsClient o-- HostNetworkConfig
```

### Effect 分流流程图

```mermaid
flowchart TD
  A["收到一批 OwnedEffect"] -->|逐条判断类型| B{"副作用类型"}
  B -->|需回复或免回复持久化| C["按表集合选择单消费者写队列"]
  C -->|队列已满| D["阻塞并把背压传回输入端"]
  C -->|执行完成且带 correlation| E["经无界通道回灌 PortReply"]
  B -->|必达 HTTP| F["提交到 Block HTTP 池"]
  B -->|文件上传| G["提交到 Block 上传池"]
  B -->|可丢 HTTP| H["提交到 DropNewest 池"]
  H -->|队列已满| I["丢弃本条最新请求"]
  B -->|WebSocket 发送| J["查传输表并内联发送"]
  B -->|领域事件| K["同步 emit 并等待批次刷新"]
  B -->|调度或取消计时器| L["更新 TimerRegistry"]
  B -->|宿主 Request| M["记录未接线警告且不回灌"]
```

`Effect::Request` 当前仍是 deferred stub，只记录警告，不产生 `PortReply`。任何文档都不应把这条链描述为已闭合。

## 3. `src/storage.rs` 所属存储子域

### 类图

```mermaid
classDiagram
  class HostStorage {
    -writer
    -readers
    +batch_upsert()
    +scan()
    +delete_where()
  }
  class ReadPool {
    -connections
    -semaphore
  }
  class UpsertShapeKey
  class StorageTraceContext
  class StorageOp {
    <<enumeration>>
  }
  HostStorage *-- ReadPool : 文件库使用
  HostStorage --> UpsertShapeKey : 缓存 SQL 形状
  HostStorage --> StorageTraceContext : 带入阻塞线程
  HostStorage ..> StorageOp : 兑现操作
```

### 存储流程图

```mermaid
flowchart TD
  A["接收 SQLite URL"] -->|规范化目标| B["判断能否共享读取"]
  B -->|所有形态| C["在阻塞线程打开一个写连接"]
  B -->|文件库或共享缓存内存库| D["打开四个只读连接"]
  B -->|独占内存库| E["读取回退到写连接"]
  C -->|初始化| F["配置 WAL、NORMAL、busy_timeout 和基础迁移"]
  D -->|连接就绪| F
  E -->|连接就绪| F
  F -->|开始处理操作| G{"存储操作类别"}
  G -->|写操作| H["锁定单写连接并在阻塞线程执行"]
  G -->|读操作| I["异步取得读池许可"]
  I -->|借出连接| J["在阻塞线程查询"]
  J -->|查询结束| K["归还连接并释放许可"]
  H -->|提交或回滚| L["返回成功或分类错误"]
  K -->|编码行结果| L
```

### 存储状态图

```mermaid
stateDiagram-v2
  state "尚未打开" as Closed
  state "写连接已建立" as WriterReady
  state "读池已建立" as ReadersReady
  state "迁移已完成" as Migrated
  state "可服务" as Serving
  state "读取中" as Reading
  state "写入中" as Writing
  [*] --> Closed: 打开存储组件
  Closed --> WriterReady: 打开写连接
  WriterReady --> ReadersReady: 目标支持共享读取
  WriterReady --> Migrated: 目标不支持共享读取
  ReadersReady --> Migrated: 建立四个只读连接
  Migrated --> Serving: 完成数据库配置和迁移
  Serving --> Reading: 收到读取操作
  Reading --> Serving: 归还只读连接
  Serving --> Writing: 收到写入操作
  Writing --> Serving: 提交或回滚
```

存储复杂度：主键/唯一键 upsert 依赖 SQLite 索引，通常 O(log n)；一批 k 行为 O(k log n)；Scan 由 spec 限定条件/排序并设置 10000 硬上限；同 table-set 写入由 N=1 worker 保序。

## 4. `src/network.rs` 与 `src/http_cross.rs` 网络子域

### 类图

```mermaid
classDiagram
  class HostNetworkConfig
  class HostHeaderRegistry
  class SharedHttpClient
  class SharedWsClient
  class CrossCuttingHttp
  class AuthTokenRegistry
  class ReconnectPolicy
  class ReconnectTraceSink
  SharedHttpClient *-- HostNetworkConfig
  SharedHttpClient o-- HostHeaderRegistry
  SharedWsClient *-- HostNetworkConfig
  SharedWsClient o-- HostHeaderRegistry
  CrossCuttingHttp o-- SharedHttpClient : 装饰
  CrossCuttingHttp o-- AuthTokenRegistry : 热更新凭据
  ReconnectPolicy --> SharedWsClient : 驱动重连
  ReconnectTraceSink ..> SharedWsClient : 记录尝试结果
```

### HTTP 横切流程图

```mermaid
flowchart TD
  A["接收 HttpRequest"] -->|解析内部意图| B["查找并移除鉴权意图头"]
  B -->|识别意图且凭据存在| C["覆盖注入真实鉴权头"]
  B -->|意图缺失、未知或无凭据| D["不注入凭据"]
  C -->|保持链路| E["保留或注入 traceparent"]
  D -->|保持链路| E
  E -->|请求级头覆盖共享头| F["合并 headers"]
  F -->|解析相对 URL| G["用 reqwest 发送"]
  G -->|收到 401| H["发布鉴权失效事件并返回响应"]
  G -->|收到其他响应| I["原样返回状态、头和正文"]
  G -->|网络或超时错误| J["发布离线事件并上抛错误"]
  G -->|其他错误| K["只上抛错误"]
```

### WebSocket 与重连状态图

```mermaid
stateDiagram-v2
  state "未连接" as Disconnected
  state "正在连接" as Connecting
  state "已连接" as Connected
  state "读取帧" as Reading
  state "等待重连" as Backoff
  [*] --> Disconnected: 创建网络客户端
  Disconnected --> Connecting: 调用连接
  Connecting --> Connected: 握手成功并拆分读写半边
  Connecting --> Disconnected: 握手失败
  Connected --> Reading: 启动独立读取任务
  Reading --> Reading: 收到帧并转换为入站输入
  Reading --> Disconnected: 收到关闭帧或读取错误
  Connected --> Disconnected: 主动关闭
  Disconnected --> Backoff: 监督器收到断开事件
  Backoff --> Connecting: 退避时间到并创建新连接
  Connecting --> Backoff: 重连失败且仍可继续尝试
  Connecting --> Connected: 重连成功并注册到传输表
```

## 5. `src/trace.rs` 与 `src/otel.rs` 可观测子域

### 类图

```mermaid
classDiagram
  class TraceCarrier
  class CommandTraceQueue
  class TraceHooks
  class TraceHooksImpl {
    <<interface>>
  }
  class TraceScope
  class HostOtelRuntime
  class HostSpanScope
  class OtlpExporter
  TraceHooks o-- TraceHooksImpl : 具体探针
  TraceHooks o-- TraceCarrier : 当前链路载体
  TraceHooks o-- CommandTraceQueue : 命令一一对位
  TraceHooks --> TraceScope : 建立逐跳作用域
  HostOtelRuntime --> HostSpanScope : 创建 span
  HostSpanScope --> OtlpExporter : 销毁时导出
```

### Trace 流程图

```mermaid
flowchart TD
  A["命令侧车或请求头携带 trace 信息"] -->|校验长度与 W3C 格式| B{"carrier 是否可信"}
  B -->|校验失败| C["拒绝或丢弃不可信字段"]
  B -->|校验通过| D["建立当前 Tick 活动 carrier"]
  D -->|开始状态机步骤| E["创建 core step 与入站 span"]
  E -->|判断跳点类型| F{"副作用跳点"}
  F -->|存储| G["建立 task-local 上下文并记录 SQL span"]
  F -->|HTTP 请求| H["补 traceparent 并记录请求响应"]
  F -->|WebSocket 发送| I["记录出站 span"]
  F -->|领域事件| J["记录事件出站 span"]
  F -->|上传| K["记录上传出站 span"]
  G -->|作用域结束| L["导出 span"]
  H -->|作用域结束| L
  I -->|作用域结束| L
  J -->|作用域结束| L
  K -->|作用域结束| L
  L -->|导出失败且未超过八次| M["等待后重试"]
  M -->|重试导出| L
```

## 6. `src/local_store.rs` 多租户路径子域

### 类图

```mermaid
classDiagram
  class LocalStoreIdentity
  class LocalStoreLocator
  class LocalStorePaths
  class LocalStoreManifest
  class BackendScope
  LocalStoreLocator --> LocalStoreIdentity : 接收身份
  LocalStoreIdentity o-- BackendScope : 归一后端范围
  LocalStoreLocator --> LocalStorePaths : 计算目录与数据库
  LocalStoreLocator --> LocalStoreManifest : 维护无凭据清单
```

### 路径派生流程图

```mermaid
flowchart TD
  A["接收本地库身份"] -->|校验应用、环境、网关、公司、账号和 schema| B{"字段是否完整"}
  B -->|字段缺失| C["返回 MissingField"]
  B -->|校验通过| D["生成长度前缀 canonical identity"]
  D -->|计算稳定哈希| E["生成 FNV-1a store key"]
  E -->|建立租户目录| F["创建 data/store-key"]
  F -->|确定持久化文件| G["得到 tenant.db 与 manifest.json"]
  G -->|维护时间信息| H["保留首次创建时间并刷新最近打开时间"]
  H -->|排除 token 和 cookie| I["写入无凭据 manifest"]
  I -->|完成定位| J["返回 db_path 和 SQLite URL"]
```

## 7. `tests/` 文件夹

### 验证对象类图

```mermaid
classDiagram
  class StorageConformanceTests
  class NetworkSharedTests
  class HttpCrosscutTests
  class ReconnectTests
  class OtelBoundaryTests
  class ImPersistTests
  class HostStorage
  class SharedHttpClient
  class SharedWsClient
  class CrossCuttingHttp
  class TraceHooks
  StorageConformanceTests --> HostStorage
  NetworkSharedTests --> SharedHttpClient
  NetworkSharedTests --> SharedWsClient
  HttpCrosscutTests --> CrossCuttingHttp
  ReconnectTests --> SharedWsClient
  OtelBoundaryTests --> TraceHooks
  ImPersistTests --> HostStorage
```

### 验证流程图

```mermaid
flowchart TD
  A["构造临时数据库、假网络或记录探针"] -->|执行目标端口合同| B["采集回灌、SQL 终态与 trace"]
  B -->|验证单调 cursor 和写入顺序| C["存储合同"]
  B -->|验证共享头、断线和重连| D["网络合同"]
  B -->|验证 401、离线和 carrier| E["横切与 trace 合同"]
  C -->|全部满足| F["host 共享语义通过"]
  D -->|全部满足| F
  E -->|全部满足| F
  C -->|任一不满足| G["测试失败"]
  D -->|任一不满足| G
  E -->|任一不满足| G
```

## 8. `benches/` 文件夹

### 基准关系类图

```mermaid
classDiagram
  class StorageBench
  class RuntimeCompareArchive
  class HostStorage
  StorageBench --> HostStorage : 测量存储热路径
  RuntimeCompareArchive ..> HostStorage : 保留历史决策
```

### 基准流程图

```mermaid
flowchart LR
  A["准备固定数据规模"] -->|预热 SQLite 和 SQL shape cache| B["重复执行存储操作"]
  B -->|记录吞吐与延迟| C["Criterion 报告"]
  C -->|和既有基线比较| D["判断是否性能退化"]
```

`storage_bench` 是 Cargo 注册的 active bench；`runtime_compare.rs` 当前未注册，只保留历史实现/决策参考。

## 9. `docs/design/` 文件夹

### 文档关系类图

```mermaid
classDiagram
  class CurrentHostArchitecture
  class LegacyHostDesign
  class HostSource
  CurrentHostArchitecture --> HostSource : 按当前代码生成
  CurrentHostArchitecture --> LegacyHostDesign : 仅引用历史意图
  LegacyHostDesign ..> HostSource : 存在线程池和关闭语义漂移
```

### 阅读流程图

```mermaid
flowchart LR
  A["先读本页 current snapshot"] -->|定位真实入口| B["阅读 engine、dispatch、storage、network"]
  B -->|需要历史性能设计| C["按需阅读 docs/design"]
  C -->|出现三池描述| D["以当前一写四读为准"]
  C -->|出现必达 HTTP 无限排空| E["以当前三秒封顶可中止为准"]
```
