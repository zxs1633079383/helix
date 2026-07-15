# helix-driver-ffi 架构

> 目录：`crates/helix-driver-ffi`。这是 Flutter/C++ 宿主的 C ABI 薄壳。Rust 自持 SQLite、HTTP 和 WebSocket；C++ 只提供壁钟和批量领域事件回调。

## 1. crate 模块

### C ABI 面

当前导出 7 个冻结函数：`helix_create_with_network`、`helix_destroy`、`helix_set_header`、`helix_remove_header`、`helix_command`、`helix_query`、`helix_query_free`。W3C trace carrier 通过 `helix_command` payload 的保留 `__trace` metadata 传入，driver 入队前剥离，不增加 ABI 符号，也不进入业务 payload。

### crate 类图

```mermaid
classDiagram
  class HelixHandle
  class PumpArgs
  class QueryDeps
  class CppHostVtable
  class FfiEventSink
  class FfiHttpRequester
  class FfiClock
  class HostStorage
  class SharedWsClient
  class ExecutionShell
  class ImModule
  HelixHandle *-- QueryDeps : 同步查询依赖
  HelixHandle --> PumpArgs : 创建专用线程
  PumpArgs *-- CppHostVtable : 冻结回调表
  FfiEventSink *-- CppHostVtable : 批量事件回调
  FfiHttpRequester --> FfiEventSink : 横切错误共用出口
  FfiClock --> CppHostVtable : 优先读取宿主时间
  QueryDeps --> HostStorage : 本地读取
  PumpArgs --> SharedWsClient : Rust自持连接
  PumpArgs --> ExecutionShell : 运行共享内核
  ExecutionShell o-- ImModule : 注册IM模块
```

### crate 生命周期状态图

```mermaid
stateDiagram-v2
  state "正在创建" as Creating
  state "建立专用运行时" as Runtime
  state "初始化存储" as Storage
  state "装配端口" as Ports
  state "引擎就绪" as Ready
  state "处理命令" as Command
  state "执行状态机" as Step
  state "批量回传事件" as Batch
  state "优雅排空" as Drain
  state "已销毁" as Destroyed
  [*] --> Creating: 调用创建函数
  Creating --> Runtime: 创建有界输入通道和泵线程
  Runtime --> Storage: 建立单线程 Tokio runtime
  Storage --> Ports: 打开 SQLite 并执行 IM 建表迁移
  Ports --> Ready: 创建事件出口、HTTP、WS、时钟并注册模块
  Ready --> Command: C++ 提交命令或带追踪命令
  Command --> Step: 命令进入 Tick 通道
  Step --> Batch: 一个步骤内事件统一编码
  Batch --> Ready: C++ 回调返回
  Ready --> Drain: 调用销毁并关闭通道
  Drain --> Destroyed: 泵线程退出并完成 join
  Destroyed --> [*]: C++ 可以释放回调上下文
```

## 2. `src/` 文件夹

### 文件职责

| 文件 | 关键类型/入口 | 作用 |
|---|---|---|
| `lib.rs` | facade | 声明 C++ 直连模型和模块导出 |
| `api.rs` | `HelixHandle`、8 个 C ABI | 指针复制、返回码、线程和内存生命周期 |
| `pump.rs` | `PumpArgs`、`QueryDeps`、`run` | current-thread runtime、schema、端口、WS、重连和共享 pump |
| `vtable.rs` | `CppHostVtable` | `on_domain_events_batch`、`now_ms` 和 `ctx` |
| `event_sink.rs` | `FfiEventSink` | emit 只攒批，flush 才跨边界 |
| `batch.rs` | batch codec | 小端 `count + (len,payload)*` 编解码 |
| `clock.rs` | `FfiClock` | 优先 C++ `now_ms`，缺失才后退系统时间 |
| `http.rs` | `FfiHttpRequester` | 固定使用 crosscut shared HTTP |
| `api_tests.rs` | cfg test | 参数、UTF-8、busy/null、trace 和生命周期 |
| `api_e2e_tests.rs` | cfg e2e | 命令到批回调、建表到 query |

### `src/` 类图

```mermaid
classDiagram
  class HelixHandle {
    +headers
    +auth
    +command_traces
    +tick_tx
    +shutdown_tx
    +pump_thread
    +query
  }
  class CppHostVtable {
    +ctx
    +on_domain_events_batch
    +now_ms
  }
  class FfiEventSink {
    -events
    -scratch
    +emit()
    +flush()
  }
  class FfiClock
  class FfiHttpRequester
  class PumpArgs
  class QueryDeps
  HelixHandle o-- QueryDeps : 查询路径
  HelixHandle --> PumpArgs : 启动泵
  PumpArgs *-- CppHostVtable
  FfiEventSink *-- CppHostVtable
  FfiClock *-- CppHostVtable
  FfiHttpRequester o-- FfiEventSink : 错误事件出口
```

### create 流程图

```mermaid
flowchart TD
  A["C++ 传入 API base、WS URL 和 vtable"] -->|立即复制 UTF-8 参数| B{"参数是否合法"}
  B -->|否| C["返回空指针"]
  B -->|是| D["创建 shared config、headers、auth 和 trace queue"]
  D -->|建立容量256输入通道| E["创建 PumpArgs"]
  E -->|启动 helix-ffi-pump 线程| F["pump 打开数据库并装配端口"]
  F -->|装配失败| G["回传 None、join 并返回空指针"]
  F -->|装配成功| H["回传 QueryDeps"]
  H -->|转为不透明指针| I["返回 HelixHandle 不透明指针"]
```

### command 流程图

```mermaid
flowchart LR
  A["C++ 调用 helix_command"] -->|复制 name 和 payload| B["公开命令别名与载荷归一"]
  B -->|可选侧车 trace 一一入队| C["构造 Tick::Command"]
  C -->|非阻塞 try_send| D{"输入通道状态"}
  D -->|可写| E["pump 执行状态机"]
  D -->|已满| F["返回 FFI_BUSY 由宿主稍后重试"]
  D -->|已关闭| G["返回 FFI_NULL"]
  E -->|本轮产生事件| H["FfiEventSink 攒批"]
  H -->|step 派发完成| I["一次 flush 跨 C ABI"]
```

### query 流程图

```mermaid
flowchart TD
  A["C++ 调用 helix_query"] -->|识别查询名称| B{"查询类型"}
  B -->|capabilities 或 version| C["直接构造元信息 JSON"]
  B -->|本地 IM 查询| D["mobile_query::plan 生成 ScanSpec"]
  D -->|在 pump runtime 上执行| E["HostStorage.scan"]
  E -->|返回 rows| F["shape_json 整形成契约事件"]
  B -->|未知查询| G["返回未实现错误码"]
  C -->|Box 字节切片| H["返回 ptr 和 len"]
  F -->|Box 字节切片| H
  H -->|宿主消费完成| I["helix_query_free 单次释放"]
```

### destroy 流程图

```mermaid
flowchart TD
  A["C++ 调用 helix_destroy"] -->|空指针直接返回| B{"handle 是否有效"}
  B -->|有效| C["Box::from_raw 恢复唯一所有权"]
  C -->|关闭输入通道| D["pump 进入 graceful drain"]
  C -->|发送冗余 shutdown| D
  D -->|持久化排空、网络有界结束| E["pump 线程退出"]
  E -->|join 完成| F["最后一批回调已经返回"]
  F -->|释放 handle| G["C++ 可以安全释放 vtable.ctx"]
```

## 3. `tests/` 文件夹

### 验证类图

```mermaid
classDiagram
  class ApiSmokeTests
  class EgressParityTests
  class HttpCrosscutTests
  class LifecycleTests
  class MobileContractTests
  class LifecycleLeakTests
  class TraceCommandTests
  class HelixHandle
  class CppHostVtable
  ApiSmokeTests --> HelixHandle
  ApiSmokeTests --> CppHostVtable
  EgressParityTests --> CppHostVtable
  HttpCrosscutTests --> HelixHandle
  LifecycleTests --> HelixHandle
  MobileContractTests --> HelixHandle
  LifecycleLeakTests --> HelixHandle
  TraceCommandTests --> HelixHandle
```

### 验证流程图

```mermaid
flowchart TD
  A["构造测试 vtable、临时库和假网络"] -->|通过真实 C ABI 调用| B["create"]
  B -->|提交 command、query、header 或 trace| C["运行 pump 与共享内核"]
  C -->|采集批回调和返回码| D["验证 wire、parity 与 busy 语义"]
  C -->|反复创建销毁或网络不可达| E["验证有界生命周期和无泄漏"]
  D -->|满足合同| F["调用 destroy 和 query_free"]
  E -->|满足合同| F
  F -->|join 完成且无迟到回调| G["FFI 边界通过"]
```

## 4. `smoke/` 文件夹

### C++ 烟测类图

```mermaid
classDiagram
  class CppSmokeMain
  class GeneratedHelixHeader
  class CppHostVtable
  class RustCdylib
  CppSmokeMain --> GeneratedHelixHeader : 使用C ABI声明
  CppSmokeMain --> CppHostVtable : 提供回调函数
  GeneratedHelixHeader --> RustCdylib : 链接导出符号
```

### C++ 烟测流程图

```mermaid
flowchart LR
  A["CMake 编译 smoke.cpp"] -->|链接 helix cdylib| B["创建 C++ vtable"]
  B -->|调用 create、command、query| C["验证 C ABI 可达"]
  C -->|消费返回缓冲| D["调用 query_free"]
  D -->|关闭实例| E["调用 destroy 并等待退出"]
  E -->|进程无崩溃且回调合法| F["C++ 最后一公里通过"]
```

`smoke/build/` 是生成目录，不属于源码架构，不纳入文档维护范围。

## 5. `docs/design/` 文件夹

### 文档类图

```mermaid
classDiagram
  class CurrentFfiArchitecture
  class CAbiBoundaryDiagram
  class FfiMemorySafetyDiagram
  class CppDirectDocs
  class FfiSource
  CurrentFfiArchitecture --> FfiSource : 以当前main为准
  CurrentFfiArchitecture --> CAbiBoundaryDiagram : 补充边界细节
  CurrentFfiArchitecture --> FfiMemorySafetyDiagram : 补充所有权规则
  CurrentFfiArchitecture --> CppDirectDocs : 补充移动宿主设计
```

### 阅读流程图

```mermaid
flowchart LR
  A["先读本页生命周期与ABI"] -->|理解当前实现| B["阅读 api、pump、vtable、event_sink"]
  B -->|需要所有权细节| C["阅读 FFI memory safety 图"]
  B -->|需要 C++ 直连背景| D["阅读 design/cpp-direct"]
  D -->|遇到 Dart-host 旧描述| E["以 Rust 自持网络和 C++ 批回调为准"]
```

## 6. `docs/design/cpp-direct/` 文件夹

### 边界类图

```mermaid
classDiagram
  class FlutterUi
  class CppCoreBridge
  class HelixCAbi
  class RustPump
  class SharedHostIo
  FlutterUi --> CppCoreBridge : 调用命令并接收事件
  CppCoreBridge --> HelixCAbi : 管理不透明句柄
  HelixCAbi --> RustPump : 投递Tick和生命周期
  RustPump --> SharedHostIo : 自持HTTP、WS和SQLite
```

### C++ 直连流程图

```mermaid
flowchart LR
  A["Flutter UI"] -->|经平台桥提交命令| B["C++ CoreBridge"]
  B -->|调用 C ABI| C["Rust FFI pump"]
  C -->|执行业务与真实I/O| D["shared host driver"]
  D -->|产生领域事件批| C
  C -->|一次 C++ 回调| B
  B -->|转发结构化事件| A
```

边界铁律：不允许 unwind 跨 `extern "C"`；`Box::into_raw/from_raw` 各一次；`destroy` 返回前 vtable 和 `ctx` 必须仍有效；事件按 step 批量跨边界。
