# helix-driver-wasm 归档架构

> 目录：`crates/helix-driver-wasm`。ADR-006 已将 Web/wasm 暂停维护；它不是 native 与 FFI 之外的第三个可交付端。

## 1. crate 模块

### 当前完成度

- 非 `wasm32` 目标直接 `compile_error!`。
- `JsClock` 基本可用。
- Storage、Transport、HTTP 和 engine loop 仍是 TODO 或骨架。
- EventSink 当前丢弃事件，没有接 JS callback。

### crate 类图

```mermaid
classDiagram
  class Clock {
    <<interface>>
  }
  class Storage {
    <<interface>>
  }
  class Transport {
    <<interface>>
  }
  class HttpRequester {
    <<interface>>
  }
  class EventSink {
    <<interface>>
  }
  class JsClock
  class WasmStorage
  class WasmTransport
  class WasmHttp
  class WasmEventSink
  class WasmEngineLoop
  Clock <|.. JsClock : 基本实现
  Storage <|.. WasmStorage : 未完成
  Transport <|.. WasmTransport : 未完成
  HttpRequester <|.. WasmHttp : 未完成
  EventSink <|.. WasmEventSink : 当前丢弃
  WasmEngineLoop --> WasmStorage : 计划装配
  WasmEngineLoop --> WasmTransport : 计划装配
  WasmEngineLoop --> WasmHttp : 计划装配
  WasmEngineLoop --> WasmEventSink : 计划装配
```

### 归档状态图

```mermaid
stateDiagram-v2
  state "归档保留" as Archived
  state "校验编译目标" as CheckTarget
  state "拒绝构建" as RejectBuild
  state "检查工作线程" as CheckWorker
  state "拒绝初始化" as RejectInit
  state "实验骨架" as Skeleton
  state "尚未可运行" as Incomplete
  [*] --> Archived: 加载归档模块
  Archived --> CheckTarget: 尝试构建
  CheckTarget --> RejectBuild: 目标不是 wasm32
  CheckTarget --> CheckWorker: 目标是 wasm32
  CheckWorker --> RejectInit: 不在专用工作线程
  CheckWorker --> Skeleton: 位于专用工作线程
  Skeleton --> Incomplete: 存储、网络、事件泵仍未实现
  RejectBuild --> [*]: 保持归档状态
  RejectInit --> [*]: 保持归档状态
  Incomplete --> [*]: 不进入交付验收
```

## 2. `src/` 文件夹

### 文件职责

| 文件 | 当前事实 |
|---|---|
| `lib.rs` | 目标守卫和模块声明 |
| `clock.rs` | `Date.now()` 实现 `Clock` |
| `storage.rs` | 做 Worker 环境检查，8 个 Storage 方法仍未实现 |
| `transport.rs` | 保存 URL，connect/send/recv/close 未实现 |
| `http.rs` | request 未实现 |
| `event_sink.rs` | emit 丢弃事件 |
| `engine_loop.rs` | `spawn_local` 外壳存在，循环未实现 |

### `src/` 类图

```mermaid
classDiagram
  class JsClock {
    +now_ms()
  }
  class WasmStorage {
    +assert_worker_context()
    +storage_methods()
  }
  class WasmTransport {
    +url
    +connect()
    +send()
    +recv()
    +close()
  }
  class WasmHttp {
    +request()
  }
  class WasmEventSink {
    +emit()
  }
  class WasmEngineLoop {
    +run_engine_loop()
  }
  WasmEngineLoop --> JsClock : 计划注入时间
  WasmEngineLoop --> WasmStorage : 计划兑现存储
  WasmEngineLoop --> WasmTransport : 计划兑现传输
  WasmEngineLoop --> WasmHttp : 计划兑现请求
  WasmEngineLoop --> WasmEventSink : 计划发布事件
```

### `src/` 实际流程图

```mermaid
flowchart TD
  A["wasm32 调用实验入口"] -->|启动本地异步任务| B["进入 engine loop 外壳"]
  B -->|尝试装配存储| C["WasmStorage"]
  B -->|尝试装配 HTTP| D["WasmHttp"]
  B -->|尝试装配传输| E["WasmTransport"]
  B -->|尝试装配事件出口| F["WasmEventSink"]
  C -->|调用未完成方法| G["停止于 TODO"]
  D -->|调用未完成方法| G
  E -->|调用未完成方法| G
  F -->|事件被丢弃| H["没有 JS 可见投影"]
```

## 3. `docs/design/` 文件夹

### 文档类图

```mermaid
classDiagram
  class WasmArchiveReadme
  class ArchivedWebArchitecture
  class Adr006
  class CurrentProductArchitecture
  WasmArchiveReadme --> Adr006 : 解释归档决策
  ArchivedWebArchitecture --> WasmArchiveReadme : 保留历史方案
  CurrentProductArchitecture --> Adr006 : 只维护native与FFI
```

### 阅读流程图

```mermaid
flowchart LR
  A["先读 ADR-006 和本页"] -->|确认非交付端| B["按需读 archived-web-architecture.mmd"]
  B -->|了解历史设想| C["不要推导当前可用能力"]
  C -->|若未来重启 Web| D["先新增 ADR、恢复 CI 和重新设计存储"]
```

## 4. 未来重新激活的前置条件

1. 新 ADR 明确 Web 产品目标和维护人。
2. 恢复 wasm CI 和当前已归档的编译护栏。
3. 完成 Worker 环境下 Storage、HTTP、WS、EventSink 和 pump。
4. 用和 native/FFI 同一 Tick 剧本做 byte-level 或语义 parity 验收。
