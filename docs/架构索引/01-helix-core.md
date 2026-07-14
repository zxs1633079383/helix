# helix-core 架构

> 目录：`crates/helix-core`。它是运行时无关的确定性内核，不认识 IM、SQLite、tokio、Tauri 或 Flutter。

## 1. crate 模块

### 职责

`helix-core` 提供系统的通用语言：输入是 `Tick`，业务扩展点是 `Module`，输出是 `Effect`，执行聚合根是 `ExecutionShell`，外部能力以 ports trait 表达。

### crate 类图

```mermaid
classDiagram
  class ExecutionShell {
    +register(module)
    +start() Effect[]
    +step(tick, now_ms) Effect[]
    +stop() Effect[]
  }
  class ModuleHost {
    -Module[] modules
    +dispatch_inbound()
    +dispatch_reply()
  }
  class Module {
    <<interface>>
    +name()
    +accepts(tick)
    +handle(tick, now_ms, out)
    +on_start(out)
    +on_stop(out)
  }
  class CorrelationMap
  class TimerMap
  class EffectSink
  class Tick {
    <<enumeration>>
  }
  class Effect {
    <<enumeration>>
  }
  ExecutionShell *-- ModuleHost : 持有模块宿主
  ExecutionShell *-- CorrelationMap : 定向端口回复
  ExecutionShell *-- TimerMap : 定向计时器
  ExecutionShell *-- EffectSink : 复用输出缓冲
  ModuleHost o-- Module : 注册业务模块
  Module ..> Tick : 处理输入
  Module ..> EffectSink : 写入副作用
  EffectSink o-- Effect : 保持有序批次
```

### crate 流程图

```mermaid
flowchart TD
  A["宿主创建 ExecutionShell"] -->|注册业务模块| B["ModuleHost 保存模块"]
  B -->|启动时按注册顺序调用| C["Module::on_start"]
  C -->|写入初始副作用| D["EffectSink"]
  D -->|登记 correlation 与 timer| E["返回初始 Effect 批次"]
  E -->|宿主反复喂入 Tick| F["ExecutionShell::step"]
  F -->|定向或首匹配路由| G["Module::handle"]
  G -->|只表达副作用意图| D
  F -->|关闭时逆序调用| H["Module::on_stop"]
  H -->|返回清理副作用| I["宿主完成兑现"]
```

## 2. `src/` 文件夹

### 文件职责

| 文件 | 关键类型/入口 | 作用 |
|---|---|---|
| `lib.rs` | 公共 re-export | 统一导出 `ExecutionShell`、`Module`、`Tick`、`Effect`、错误和平台约束 |
| `auth.rs` | `AuthKind` | 只表达 Session/Bot 鉴权意图，不保存真实 token |
| `effect.rs` | `Effect`、`StorageOp`、各类 spec | 副作用与通用存储协议真源 |
| `tick.rs` | `Tick`、`AppCommand`、`PortOutcome` | 状态机输入 tagged union |
| `module_host.rs` | `Module`、`ModuleHost` | 模块注册、首匹配路由和定向回复 |
| `engine.rs` | `ExecutionShell` | 启停、step、corr/timer 注册与回收 |
| `error.rs` | `CoreError`、端口 `PortError` | 区分内核错误和端口实现错误 |
| `platform.rs` | `MaybeSend`、`MaybeSync` | 按目标平台约束 Send/Sync |
| `engine_tests.rs` | 测试模块 | 验证一次性 timer 与空壳行为 |

当前 `Effect` 有 10 个顶层变体：`Send`、`Persist`、`PersistFire`、`Http`、`UploadFile`、`HttpFire`、`Request`、`Emit`、`ScheduleTimer`、`CancelTimer`。当前 `StorageOp` 有 8 个变体：`BatchUpsert`、`MonotonicUpsert`、`BatchUpdate`、`GuardedBump`、`Get`、`Scan`、`BatchDelete`、`DeleteWhere`。

### `src/` 类图

```mermaid
classDiagram
  class Tick {
    <<enumeration>>
    Inbound
    PortReply
    Timer
    Command
    Connected
    Disconnected
  }
  class AppCommand {
    +name
    +payload
  }
  class Effect {
    <<enumeration>>
    Send
    Persist
    PersistFire
    Http
    UploadFile
    HttpFire
    Request
    Emit
    ScheduleTimer
    CancelTimer
  }
  class StorageOp {
    <<enumeration>>
    BatchUpsert
    MonotonicUpsert
    BatchUpdate
    GuardedBump
    Get
    Scan
    BatchDelete
    DeleteWhere
  }
  class Correlation
  class TimerId
  class TransportId
  class DomainEventBytes
  class AuthKind {
    <<enumeration>>
    Session
    Bot
  }
  Tick o-- AppCommand : 命令载荷
  Effect o-- StorageOp : 持久化操作
  Effect o-- Correlation : 关联回灌
  Effect o-- TimerId : 调度计时器
  Effect o-- TransportId : 选择连接
  Effect o-- DomainEventBytes : 发布领域事件
  Effect ..> AuthKind : 通过请求头表达意图
```

### `src/` 输入路由流程图

```mermaid
flowchart TD
  A["宿主喂入 Tick 和 now_ms"] -->|开始新一轮| B["清空并复用 Effect 缓冲"]
  B -->|判断输入类别| C{"输入属于哪一类"}
  C -->|入站帧、命令、已连接或已断开| D["按注册顺序扫描 accepts"]
  D -->|首个模块接受| E["调用模块 handle"]
  D -->|没有模块接受| F["记录调试信息并忽略"]
  C -->|端口回复| G["消费 correlation 映射"]
  G -->|找到发起模块| E
  G -->|未知 correlation| H["记录警告并忽略"]
  C -->|计时器触发| I["查询 timer 映射"]
  I -->|找到所属模块| E
  I -->|未知或已取消| F
  E -->|模块只写入 Effect| J["登记新 correlation 和 timer"]
  J -->|本轮处理完成| K["返回有序 Effect 切片"]
```

### 模块生命周期状态图

```mermaid
stateDiagram-v2
  state "未注册" as Unregistered
  state "已注册" as Registered
  state "已启动" as Started
  state "处理中" as Handling
  state "等待端口回复或计时器" as Waiting
  state "已停止" as Stopped
  [*] --> Unregistered: 创建执行壳
  Unregistered --> Registered: 注册模块
  Registered --> Started: 正序调用启动钩子
  Started --> Handling: 收到可处理输入
  Handling --> Waiting: 产生带关联键的副作用或计时器
  Waiting --> Handling: 收到端口回复或计时器触发
  Handling --> Started: 本轮没有待回灌副作用
  Started --> Stopped: 逆序调用停止钩子
  Stopped --> [*]: 宿主释放执行壳
```

### 路由复杂度

- `Inbound/Command`：遍历模块 `Vec`，O(m)。设计预期 m 小于 10，优先保持开闭原则。
- `PortReply`：`HashMap<Correlation, module_index>` 平均 O(1)，回复后消费并移除。
- Timer：`HashMap<TimerId, module_index>` 平均 O(1)，当前语义是一次性触发后移除；周期行为必须分配新 id 再调度。
- `EffectSink`：复用 `Vec` 容量，按产生顺序追加，单轮 O(k)。

## 3. `src/ports/` 文件夹

### 文件职责

| 文件 | trait | 契约 |
|---|---|---|
| `clock.rs` | `Clock` | 注入 `now_ms`；测试提供 `MockClock` |
| `event_sink.rs` | `EventSink` | 同步、非阻塞发布领域事件 |
| `http.rs` | `HttpRequester` | 执行 HTTP 请求并返回响应 |
| `id.rs` | `IdSource`、`Random` | 注入 ID 和随机数 |
| `storage.rs` | `Storage` | 兑现 8 类通用存储操作 |
| `transport.rs` | `Transport` | connect/send/recv/close WebSocket 类传输 |
| `upload.rs` | `FileUploader` | 按本地路径流式上传文件 |
| `mod.rs` | re-export | ports 唯一公共入口 |

### `src/ports/` 类图

```mermaid
classDiagram
  class Clock {
    <<interface>>
    +now_ms()
  }
  class EventSink {
    <<interface>>
    +emit(event)
  }
  class HttpRequester {
    <<interface>>
    +request(request)
  }
  class IdSource {
    <<interface>>
    +new_uuid()
  }
  class Random {
    <<interface>>
    +random_u64()
  }
  class Storage {
    <<interface>>
    +batch_upsert()
    +monotonic_upsert()
    +batch_update()
    +guarded_bump()
    +get()
    +scan()
    +batch_delete()
    +delete_where()
  }
  class Transport {
    <<interface>>
    +connect()
    +send()
    +recv()
    +close()
  }
  class FileUploader {
    <<interface>>
    +upload(request)
  }
  class Effect
  Effect ..> Storage : 驱动负责兑现
  Effect ..> Transport : 驱动负责兑现
  Effect ..> HttpRequester : 驱动负责兑现
  Effect ..> FileUploader : 驱动负责兑现
  Effect ..> EventSink : 驱动负责兑现
  Clock ..> Effect : 为状态机注入时间
  IdSource ..> Effect : 为业务注入标识
  Random ..> Effect : 为业务注入随机源
```

### `src/ports/` 兑现流程图

```mermaid
flowchart LR
  A["业务模块生成 Effect"] -->|返回纯数据| B["ExecutionShell"]
  B -->|把批次交给宿主| C["driver-host 分流"]
  C -->|持久化类| D["Storage"]
  C -->|HTTP 类| E["HttpRequester"]
  C -->|WebSocket 类| F["Transport"]
  C -->|上传类| G["FileUploader"]
  C -->|领域事件类| H["EventSink"]
  C -->|计时器类| I["宿主计时器"]
  D -->|带 correlation 回灌| J["Tick::PortReply"]
  E -->|带 correlation 回灌| J
  G -->|带 correlation 回灌| J
  I -->|触发一次性回灌| K["Tick::Timer"]
  J -->|定向回原模块| A
  K -->|定向回原模块| A
```

## 4. `tests/` 文件夹

### 验证对象类图

```mermaid
classDiagram
  class MultiModuleCorrRoutingTest
  class RequestReplyReplayTest
  class ExecutionShell
  class ModuleA
  class ModuleB
  MultiModuleCorrRoutingTest --> ExecutionShell : 注册两个模块
  MultiModuleCorrRoutingTest --> ModuleA : 验证回复定向
  MultiModuleCorrRoutingTest --> ModuleB : 验证回复定向
  RequestReplyReplayTest --> ExecutionShell : 回放相同输入
```

### 验证流程图

```mermaid
flowchart TD
  A["构造固定模块与输入序列"] -->|注册到执行壳| B["启动并收集 Request"]
  B -->|按不同 correlation 回灌| C["验证回复进入各自模块"]
  C -->|再次回放同一 Tick 序列| D["比较 Effect 序列"]
  D -->|完全一致| E["确定性与多模块路由通过"]
  D -->|出现差异| F["测试失败并定位状态漂移"]
```

## 5. `docs/design/` 文件夹

### 文档关系类图

```mermaid
classDiagram
  class CurrentArchitectureIndex
  class LegacyCoreDesignDiagrams
  class CoreSource
  CurrentArchitectureIndex --> CoreSource : 以当前代码为真源
  CurrentArchitectureIndex --> LegacyCoreDesignDiagrams : 仅作历史细节参考
  LegacyCoreDesignDiagrams ..> CoreSource : 存在计数与计时器语义漂移
```

### 阅读流程图

```mermaid
flowchart LR
  A["先读本页当前快照"] -->|理解执行语言| B["再读 engine 和 module_host 源码"]
  B -->|需要历史设计背景| C["按需读 docs/design"]
  C -->|发现 Effect 或 StorageOp 数量冲突| D["回到当前枚举源码裁决"]
  C -->|发现周期计时器描述| E["以一次性触发后移除为准"]
```

现有旧图仍写 Effect 9 类、StorageOp 7 类，并包含旧 timer 描述；当前真值分别是 10、8，timer 是 one-shot-on-fire。旧图不可单独作为当前架构依据。
