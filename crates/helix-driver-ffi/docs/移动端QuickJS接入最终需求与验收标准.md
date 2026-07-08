# 移动端 QuickJS 接入最终需求与验收标准

## 1. 最终结论

移动端方案的完整链路是：

```text
Flutter UI
  -> QuickJS runtime
    -> JS UserCase 脚本调用 native im.*
      -> C++ MobileImSdk / CoreBridge
        -> helix-driver-ffi
          -> helix-core / helix-im
            -> Go Server
```

这里的关键结论是：Flutter 只负责 UI 渲染和用户交互入口，不沉淀业务逻辑；IM 业务规则、用例编排、状态收敛、协议适配边界全部下放到 Rust 侧。Rust 内核直接对接 Go Server，移动端不直接关心 Go 协议细节。

因此本阶段交付不能只理解为“编译出 hpp 就结束”。但仓库边界必须清楚：`helix` 仓负责把 Rust FFI ABI、公开 command / query / event 契约、capabilities、header / native library 和底层验收说明交付出来；C++ bridge、QuickJS 注册代码、JS wrapper、Android / iOS 平台编译产物和真机 smoke 应在移动端集成项目中创建和维护。

总流程图见：[移动端QuickJS接入总流程图.mmd](./移动端QuickJS接入总流程图.mmd)。

## 2. Rust 内核在架构里的定位

`helix-core` / `helix-im` 是跨端确定性业务内核。它负责承载 IM 的领域规则、应用用例、状态机、同步策略、事件投影和对 Go Server 的协议适配。

`helix-driver-ffi` 是移动端原生接入层。它不重新实现业务，只把 Rust 内核以稳定 C ABI 暴露给 C++ 适配层，并通过宿主虚表接收时间、事件回调、网络/平台能力等外部注入。

C++ 原生桥接是移动端运行时适配层。它负责持有 Rust handle、管理生命周期、把 C ABI 封装成 C++ 类、处理内存拷贝和线程约束，并把 `MobileImSdk` / `im.*` 这层能力注册给 QuickJS。

QuickJS 是 JS UserCase / 业务脚本运行容器。JS 不应该重新承载 IM 领域规则，也不再通过 `js/im_api.js` / `createImApi(core)` 包一层业务 facade；JS 只调用注入的 native `im.*` 方法、编写使用步骤和断言。

Flutter 是 UI 层。它只做页面渲染、输入收集、动效、导航、平台权限和最终展示，不直接写 IM 业务规则。

## 3. 职责边界

helix 仓负责到“可被移动端集成项目稳定消费”的 Rust FFI 边界：

- Rust 内核公开移动端所需的 command / query / event 契约。
- `helix-driver-ffi` 输出稳定 C ABI、hpp / 头文件、native library。
- 给出 C++ 适配层 / QuickJS native binding 必须遵守的创建、销毁、命令、查询、回调、错误和线程模型。
- 提供 Rust / FFI 层契约测试、ABI smoke、发送消息链路验收说明。
- 给出内存安全、生命周期、性能和错误处理验收脚本或验收说明。

移动端集成项目负责：

- 创建 C++ `CoreBridge` 类或等价封装。
- 注册 QuickJS 原生类和 native `im.*` API。
- 创建 C++ 直调 smoke、JS 直调 smoke、发送消息全链路 demo。
- 产出 Android / iOS 平台编译产物、加载脚本、真机验收报告。

移动端侧负责：

- Flutter UI 渲染和交互绑定。
- 集成 QuickJS 运行时、原生库、平台构建脚本和真机包。
- 把 UI 事件接到我们提供的 JS API。
- 在真实 Android / iOS 环境做最终 UI 验收、包体、权限、线程调度和崩溃采集。

双方共同确认：

- 移动端公开命令 / 查询 / 事件清单。
- JS API 命名、载荷 JSON schema、错误码和事件格式。
- Android / iOS 的原生库产物格式、架构矩阵和加载路径。
- 内存、性能、并发和稳定性验收阈值。

## 4. “所有方法”怎么理解

移动端说的“JS 调用所有方法，或者 C++ 调用所有方法内存和性能没问题”，这里的“所有方法”不应该理解为 Rust 内部所有函数，也不应该理解为每个业务用例都生成一个新的 C ABI 符号。

正确理解是：移动端公开契约里的所有命令 / 查询 / 事件都能被覆盖。

底层 C ABI 继续保持少量稳定函数，例如创建、销毁、设置 header、删除 header、执行命令、执行查询、释放查询结果。业务用例作为数据通过 `name + JSON payload` 进入 `helix_command` 或 `helix_query`，新增业务不新增 C ABI 符号。

移动端 UserCase 层面对 Flutter / 测试脚本的公开调用面可以是：

```js
var receipt = im.send({
  channelId: "c1",
  temporaryId: "local-001",
  text: "hello"
});

im.waitForEvent("im:post:sending", { temporaryId: receipt.temporaryId });
```

底层仍走统一命令 / 查询通道，但业务命名和字段归一由 C++ SDK / Rust 契约承接：

```text
im.send(input)
  -> MobileImSdk::send(input)
  -> CoreBridge.call("im_send", payload)
  -> helix_command("im_send", payload)
  -> helix-im 内部 alias / handler
```

禁止在 JS 中重新拼 Go 请求体、补身份字段、等待 HTTP 字符串或合成发送失败事件。

## 5. 交付物

### 5.1 Rust / FFI 产物

- `helix.hpp` 或等价 C/C++ 头文件。
- Android / iOS 可链接的原生库。
- 稳定 C ABI 说明：函数签名、参数所有权、返回码、错误语义。
- 命令 / 查询 / 事件契约文档：[`移动端Command-Query-Event契约SPEC.md`](./移动端Command-Query-Event契约SPEC.md)。
- 生命周期文档：create、dispose、callback、query result free。
- 版本 / 能力集查询能力，用于移动端启动时做兼容检查。

### 5.2 C++ 桥接产物（移动端集成项目）

- `CoreBridge` C++ 类或等价封装。
- C++ 直调所有公开命令 / 查询的冒烟测试。
- 明确线程约束：QuickJS 所在线程、Rust handle 调用线程、是否需要串行队列或锁。
- 明确内存策略：输入缓冲区拷贝、输出缓冲区释放、事件批在回调内 memcpy 稳定化。

### 5.3 QuickJS 产物（移动端集成项目）

- QuickJS 原生类注册代码。
- JS 侧 native `im.*` API。
- JS 直调所有公开命令 / 查询的冒烟脚本。
- 发送消息完整链路 demo。
- 错误路径 demo：未知命令、非法 JSON、空 handle、destroy 后调用、busy/backpressure。

## 6. 验收标准

### 6.1 编译与链接

- Rust 原生库能在目标平台编译通过。
- hpp/头文件能被 C++ 工程 include。
- 移动端集成项目中的 C++ demo 能链接真实 Rust 库，而不是 mock。
- 移动端集成项目中的 QuickJS demo 能加载 C++ 桥接并创建 JS 类。

### 6.2 C++ 直调验收

- C++ 能创建和销毁 Rust 内核 handle。
- C++ 能调用全部移动端公开命令。
- C++ 能调用全部移动端公开查询。
- C++ 能接收事件批回调。
- C++ 能处理所有定义过的错误码。

### 6.3 JS 直调验收

- JS 能使用注入的 native `im.*`。
- JS 能通过 native `im.*` 调用全部移动端公开命令。
- JS 能通过 native `im.*` 调用全部移动端公开查询。
- JS 能接收或拉取 Rust 侧投影结果。
- JS 脚本异常不会破坏 Rust handle 生命周期。

### 6.4 发送消息全链路验收

发送消息必须跑通：

```text
Flutter 点击发送
  -> JS im.send(...)
    -> QuickJS native im.* binding
      -> C++ MobileImSdk::send
        -> CoreBridge.call("im_send", payload)
          -> helix_command("im_send", payload)
          -> Rust IM usecase
            -> im:post:sending optimistic event
            -> Go Server
              -> WS post echo
                -> Rust temporaryId 对账
                  -> im:post:received projection event batch
                    -> C++ callback memcpy
                      -> JS / Flutter 更新 UI
```

具体流程图见：[发送消息完整链路流程图.mmd](./发送消息完整链路流程图.mmd)。

### 6.5 内存安全验收

- create/destroy 循环执行无泄漏。
- command/query 大量调用无泄漏。
- callback 中传出的借用指针只在 callback 生命周期内有效，C++ 必须立即 memcpy。
- 查询结果由明确的 free 函数释放，不能由 C++ 自行 `delete/free`。
- Rust panic 不跨越 `extern "C"` 边界。
- destroy 后再调用必须返回明确错误，不能崩溃。
- 重复 destroy、空指针、非法 UTF-8、非法 JSON 都有稳定错误行为。
- Android / iOS 至少各有一套可执行的泄漏 / asan / 平台内存检查方案。

### 6.6 性能验收

- JS 调用命令 / 查询的开销有基线数据。
- C++ 桥接不做无意义多次 JSON 解析 / 序列化。
- FFI 输入输出拷贝次数受控，事件批使用批量回调。
- 高频事件下没有无界队列增长。
- busy/backpressure 有明确错误码或排队策略。
- 发送消息、会话列表刷新、历史消息加载这些高频路径要有端到端耗时统计。

建议至少记录：

- 单次 `im.send(...)` / `im_send` 从 JS 调用到 Rust 命令接收的耗时。
- 从 Rust 投影批到 Flutter 可渲染数据的耗时。
- 连续 N 次命令 / 查询的平均耗时、P95、P99。
- create/destroy N 次后的内存增长。

### 6.7 错误与兼容验收

- 未知命令返回稳定错误。
- 载荷 schema 错误返回稳定错误。
- Go Server 网络错误能转成 Rust 领域事件或明确错误事件。
- JS 封装层能识别版本和能力集，不兼容时启动失败并给出明确原因。
- 移动端升级 JS 脚本时，不要求替换原生库，除非能力集明确不兼容。

## 7. 明确不做什么

- 不把 IM 业务逻辑写到 Flutter。
- 不把 Go Server 协议细节暴露给 Flutter。
- 不为每个业务方法新增一个 C ABI 符号。
- 不让 JS 成为新的业务内核；JS 只做调用入口和轻量编排。
- 不让移动端自己补齐 Rust core 缺失的业务逻辑。

## 8. 当前可用 demo 的意义

QuickJS 本地 demo 只用于验证这条形状是可行的，源码和编译产物应归属移动端集成项目：

```text
QuickJS
  -> JS script
    -> C++ CoreBridge class
      -> Rust FFI 形状
```

demo 里的 `rust_core_mock.cpp` 只是替身。真实接入时，应在移动端集成项目保留 C++ 桥接和 QuickJS 注册思路，把 mock 替换成 `helix-driver-ffi` 编译出的真实原生库与头文件。不要把移动端 C++/QuickJS demo 源码或编译产物继续放进 helix 仓。

## 9. 完成清单

- [ ] 移动端公开命令 / 查询 / 事件清单冻结。
- [ ] hpp/头文件 + 原生库产物可被 C++ 链接。
- [ ] helix 仓 Rust/FFI 契约测试覆盖公开命令 / 查询 / 事件 / 能力集。
- [ ] 移动端集成项目 C++ 直调冒烟覆盖所有公开命令 / 查询。
- [ ] 移动端集成项目 QuickJS 注册 `CoreBridge`。
- [ ] 移动端集成项目 JS 冒烟覆盖所有公开命令 / 查询。
- [ ] 移动端集成项目发送消息完整链路跑通。
- [ ] 事件批从 Rust 回流到 C++/JS/Flutter。
- [ ] create/destroy/query result/callback pointer 所有权验收通过。
- [ ] 内存泄漏和 destroy 后调用验收通过。
- [ ] 性能基线有数据，P95/P99 和内存增长可接受。
- [ ] Flutter 只做 UI 渲染，没有新增 IM 业务规则。
