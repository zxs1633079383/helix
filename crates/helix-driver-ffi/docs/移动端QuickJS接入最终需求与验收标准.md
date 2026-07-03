# 移动端 QuickJS 接入最终需求与验收标准

## 1. 最终结论

移动端方案的完整链路是：

```text
Flutter UI
  -> QuickJS runtime
    -> JS 脚本 / JS API
      -> C++ native bridge
        -> helix-driver-ffi
          -> helix-core / helix-im
            -> Go Server
```

这里的关键结论是：Flutter 只负责 UI 渲染和用户交互入口，不沉淀业务逻辑；IM 业务规则、用例编排、状态收敛、协议适配边界全部下放到 Rust 侧。Rust core 直接对接 Go Server，移动端不直接关心 Go 协议细节。

因此本阶段交付不能只理解为“编译出 hpp 就结束”。如果要减少移动端对接成本，`driver-ffi` 这一侧需要把链路推进到：C++ 可以稳定调用 Rust，QuickJS 可以注册并调用 C++ bridge，JS 脚本可以覆盖移动端需要的公开命令和查询。

总流程图见：[移动端QuickJS接入总流程图.mmd](./移动端QuickJS接入总流程图.mmd)。

## 2. Rust Core 在架构里的定位

`helix-core` / `helix-im` 是跨端确定性业务内核。它负责承载 IM 的领域规则、应用用例、状态机、同步策略、事件投影和对 Go Server 的协议适配。

`helix-driver-ffi` 是移动端 native 接入层。它不重新实现业务，只把 Rust 内核以稳定 C ABI 暴露给 C++ shim，并通过 host vtable 接收时间、事件回调、网络/平台能力等外部注入。

C++ native bridge 是移动端 runtime 适配层。它负责持有 Rust handle、管理生命周期、把 C ABI 封装成 C++ class、处理内存拷贝和线程约束，并把这层能力注册给 QuickJS。

QuickJS 是 JS 业务脚本运行容器。移动端可以通过 JS 脚本热更新/热替换业务编排入口，但 JS 不应该重新承载 IM 领域规则；JS 只是调用 Rust core 的公开命令、查询和订阅结果。

Flutter 是 UI 层。它只做页面渲染、输入收集、动效、导航、平台权限和最终展示，不直接写 IM 业务规则。

## 3. 职责边界

我们侧建议负责到“可被 JS 全量调用”的边界：

- Rust 内核公开移动端所需的 command / query / event contract。
- `helix-driver-ffi` 输出稳定 C ABI、hpp/header、native library。
- C++ shim 封装 Rust handle，明确创建、销毁、命令、查询、回调、错误和线程模型。
- QuickJS 注册 `CoreBridge` 或等价 JS class，让 JS 能调用所有移动端公开方法。
- 提供 C++ 直调 smoke、JS 直调 smoke、发送消息全链路样例。
- 给出内存安全、生命周期、性能和错误处理验收脚本或验收说明。

移动端侧负责：

- Flutter UI 渲染和交互绑定。
- 集成 QuickJS runtime、native library、平台构建脚本和真机包。
- 把 UI 事件接到我们提供的 JS API。
- 在真实 Android / iOS 环境做最终 UI 验收、包体、权限、线程调度和崩溃采集。

双方共同确认：

- 移动端公开 command / query / event 清单。
- JS API 命名、payload JSON schema、错误码和事件格式。
- Android / iOS 的 native library 产物格式、架构矩阵和加载路径。
- 内存、性能、并发和稳定性验收阈值。

## 4. “所有方法”怎么理解

移动端说的“JS 调用所有方法，或者 C++ 调用所有方法内存和性能没问题”，这里的“所有方法”不应该理解为 Rust 内部所有函数，也不应该理解为每个业务用例都生成一个新的 C ABI symbol。

正确理解是：移动端公开契约里的所有 command / query / event 都能被覆盖。

底层 C ABI 继续保持少量稳定函数，例如创建、销毁、设置 header、删除 header、执行 command、执行 query、释放 query 结果。业务用例作为数据通过 `name + JSON payload` 进入 `helix_command` 或 `helix_query`，新增业务不新增 C ABI symbol。

JS 层可以提供更友好的包装：

```js
const core = new CoreBridge();

core.call("im_send_message", JSON.stringify({
  conversation_id: "c1",
  client_msg_id: "local-001",
  text: "hello"
}));

const snapshot = core.query("im_conversation_snapshot", JSON.stringify({
  conversation_id: "c1"
}));

core.dispose();
```

如果移动端希望“像调用 class 方法一样调用”，可以在 JS wrapper 上继续生成糖衣方法：

```js
await im.sendMessage({ conversationId: "c1", text: "hello" });
await im.loadConversation({ conversationId: "c1" });
```

但 native 层仍建议保持统一 command/query 通道，避免 C ABI 随业务增长而膨胀。

## 5. 交付物

### 5.1 Rust / FFI 产物

- `helix.hpp` 或等价 C/C++ header。
- Android / iOS 可链接的 native library。
- 稳定 C ABI 说明：函数签名、参数所有权、返回码、错误语义。
- command / query / event contract 文档。
- lifecycle 文档：create、dispose、callback、query result free。
- version / capabilities 查询能力，用于移动端启动时做兼容检查。

### 5.2 C++ Bridge 产物

- `CoreBridge` C++ class 或等价封装。
- C++ 直调所有公开 command/query 的 smoke 测试。
- 明确线程约束：QuickJS 所在线程、Rust handle 调用线程、是否需要串行队列或锁。
- 明确内存策略：输入 buffer 拷贝、输出 buffer 释放、事件 batch 在 callback 内 memcpy 稳定化。

### 5.3 QuickJS 产物

- QuickJS native class 注册代码。
- JS 侧 `CoreBridge` API。
- JS 直调所有公开 command/query 的 smoke 脚本。
- 发送消息完整链路 demo。
- 错误路径 demo：未知 command、非法 JSON、空 handle、destroy 后调用、busy/backpressure。

## 6. 验收标准

### 6.1 编译与链接

- Rust native library 能在目标平台编译通过。
- hpp/header 能被 C++ 工程 include。
- C++ demo 能链接真实 Rust library，而不是 mock。
- QuickJS demo 能加载 C++ bridge 并创建 JS class。

### 6.2 C++ 直调验收

- C++ 能创建和销毁 Rust core handle。
- C++ 能调用全部移动端公开 command。
- C++ 能调用全部移动端公开 query。
- C++ 能接收事件 batch callback。
- C++ 能处理所有定义过的错误码。

### 6.3 JS 直调验收

- JS 能创建 `CoreBridge`。
- JS 能调用全部移动端公开 command。
- JS 能调用全部移动端公开 query。
- JS 能接收或拉取 Rust 侧投影结果。
- JS 脚本异常不会破坏 Rust handle 生命周期。

### 6.4 发送消息全链路验收

发送消息必须跑通：

```text
Flutter 点击发送
  -> JS sendMessage
    -> QuickJS CoreBridge.call("im_send_message", payload)
      -> C++ bridge
        -> helix_command
          -> Rust IM usecase
            -> Go Server
              -> Rust 收敛 ack / ws echo
                -> projection event batch
                  -> C++ callback memcpy
                    -> JS / Flutter 更新 UI
```

具体流程图见：[发送消息完整链路流程图.mmd](./发送消息完整链路流程图.mmd)。

### 6.5 内存安全验收

- create/destroy 循环执行无泄漏。
- command/query 大量调用无泄漏。
- callback 中传出的 borrowed pointer 只在 callback 生命周期内有效，C++ 必须立即 memcpy。
- query result 由明确的 free 函数释放，不能由 C++ 自行 `delete/free`。
- Rust panic 不跨越 `extern "C"` 边界。
- destroy 后再调用必须返回明确错误，不能崩溃。
- 重复 destroy、空指针、非法 UTF-8、非法 JSON 都有稳定错误行为。
- Android / iOS 至少各有一套可执行的 leak/asan/平台内存检查方案。

### 6.6 性能验收

- JS 调用 command/query 的开销有基线数据。
- C++ bridge 不做无意义多次 JSON parse/stringify。
- FFI 输入输出拷贝次数受控，事件 batch 使用批量回调。
- 高频事件下没有无界队列增长。
- busy/backpressure 有明确错误码或排队策略。
- 发送消息、会话列表刷新、历史消息加载这些高频路径要有端到端耗时统计。

建议至少记录：

- 单次 `im_send_message` 从 JS call 到 Rust command accepted 的耗时。
- 从 Rust projection batch 到 Flutter 可渲染数据的耗时。
- 连续 N 次 command/query 的平均耗时、P95、P99。
- create/destroy N 次后的内存增长。

### 6.7 错误与兼容验收

- 未知 command 返回稳定错误。
- payload schema 错误返回稳定错误。
- Go Server 网络错误能转成 Rust domain event 或明确错误事件。
- JS wrapper 能识别版本和 capabilities，不兼容时启动失败并给出明确原因。
- 移动端升级 JS 脚本时，不要求替换 native library，除非 capabilities 明确不兼容。

## 7. 明确不做什么

- 不把 IM 业务逻辑写到 Flutter。
- 不把 Go Server 协议细节暴露给 Flutter。
- 不为每个业务方法新增一个 C ABI symbol。
- 不让 JS 成为新的业务内核；JS 只做调用入口和轻量编排。
- 不让移动端自己补齐 Rust core 缺失的业务逻辑。

## 8. 当前可用 demo 的意义

QuickJS 本地 demo 已验证这条形状是可行的：

```text
QuickJS
  -> JS script
    -> C++ CoreBridge class
      -> Rust FFI 形状
```

demo 里的 `rust_core_mock.cpp` 只是替身。真实接入时，应保留 C++ bridge 和 QuickJS 注册思路，把 mock 替换成 `helix-driver-ffi` 编译出的真实 native library 与 header。

## 9. Done Checklist

- [ ] 移动端公开 command/query/event 清单冻结。
- [ ] hpp/header + native library 产物可被 C++ 链接。
- [ ] C++ direct smoke 覆盖所有公开 command/query。
- [ ] QuickJS 注册 `CoreBridge`。
- [ ] JS smoke 覆盖所有公开 command/query。
- [ ] 发送消息完整链路跑通。
- [ ] 事件 batch 从 Rust 回流到 C++/JS/Flutter。
- [ ] create/destroy/query result/callback pointer 所有权验收通过。
- [ ] 内存泄漏和 destroy 后调用验收通过。
- [ ] 性能基线有数据，P95/P99 和内存增长可接受。
- [ ] Flutter 只做 UI 渲染，没有新增 IM 业务规则。
