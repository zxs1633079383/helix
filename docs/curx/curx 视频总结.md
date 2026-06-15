# Crux 发布演讲总结 —— Stuart Harris @ Rust Nation UK《iOS, Android and Web applications that share a single Rust core》

> 来源 = Rust Nation UK 演讲（59:43），Crux 首次公开发布演讲。本文是 [`../过会方案/helix-crux-参照对照.md`](../过会方案/helix-crux-参照对照.md) 与 [架构设计 §7.0](../过会方案/helix-core-im-架构设计.md) 的素材来源。

视频字幕已拿到，整理完毕。这是 Stuart Harris（Red Badger 创始人/首席科学家）在 Rust Nation UK 的演讲：《iOS, Android and Web applications that share a single Rust core》（59:43）——正是当前 crux 仓库的首次公开发布演讲。以下按视频段落总结：

## 按段落总结

### 1. 开场介绍（00:00–02:30）

主持人介绍 Stuart Harris：Rust London 社区元老，其公司 Red Badger 曾把 Rust 引入 Nando's 餐厅的会员积分系统后端。并预告会后在 Red Badger 办公室举办"Rust 进企业"主题早午餐会。

### 2. 什么是 Headless App（02:30–07:10）

Stu 说他本想把演讲改名为 "Headless Apps"。核心理念：

- 把应用的**行为（behavior）封装在跨平台共享的纯函数核心（pure core）**里，所有副作用推到边缘（shell）。
- 这就是 90 年代的老模式 "Functional Core, Imperative Shell"。
- UI 是平台最擅长的事，没必要重新发明——iOS 有 SwiftUI、Android 有 Jetpack Compose、Web 有 React，都是声明式 UI 框架，直接用。
- UI 本身也是副作用，同样推到边缘。这一切的根本目的是可测试性，采用 Ports & Adapters（六边形架构）模式。

### 3. 动机（07:10–08:30）

JS/TS 生态层层叠叠互相补坑，难以驾驭，且没有 Ports & Adapters 的实现。目标是把"app 质量左移"——就像 Rust 编译器把整类 bug 提前到编译期一样，应用级 bug 也应该在最开始就暴露，而不是让用户去发现；测试应该极其容易，并且不必为 iOS/Android/Web 写三遍。

### 4. 现有跨平台方案盘点（08:30–13:25）

逐一点评四条路线（都是合理选择，各有取舍）：

- 平台原生（Swift UI + Kotlin/Compose + Web 各写一套）：完全可控但零共享，三个团队重复造轮子。
- React Native：原生控件 + 共享 JS 逻辑，但大型项目仍躲不开 iOS/Android 工程师，组件越来越大、hooks 里发 API 调用导致测试要 mock/stub，外侧测试脆弱又慢。
- Flutter：全栈重写 UI 控件（Dart），适合追求双端一致体验的产品（如 Spotify），但不是平台原生 idiomatic 体验。
- Kotlin Multiplatform Mobile：方向正确（UI 留给平台、其余共享），但共享层是 Kotlin。
- 混合方案（Capacitor/Ionic/Tauri Mobile）：在变好，值得关注。

### 5. 介绍 Crux（13:25–16:05）

Crux：开发了 4 个月的实验性开源库，本场观众是全世界第一批听众。它没发明新东西，只是把成熟架构模式组合起来：

- 平台原生 UX（和 KMM 一样不管 UI），但共享层是 Rust；
- 通过 Capability 插件让纯核心"请求" shell 执行副作用（HTTP、获取时间等）；
- 在"对业务友好（代码共享多）vs 对用户友好（原生体验好）"的张力图上，Crux 想冲右上角。

### 6. Ports & Adapters 理论根基（16:05–17:35）

引用 Alistair Cockburn 2005 年的六边形架构论文：应用应能被"用户、程序、自动化测试、批处理脚本"同等驱动，并能脱离最终的运行时设备与数据库独立开发测试——"这正是 Crux 的定义"。论文里还提到应用可以以 headless 模式部署，正好印证命名。

### 7. 五种接入方式与技术栈（17:35–20:40）

- iOS：静态库（libshared.a）+ Mozilla UniFFI 生成 Swift 绑定；
- Android：动态库 .so + JNA + 同一套 UniFFI 绑定；
- Web：编译成 WASM + wasm-bindgen，任何 JS/TS 框架可调用;
- Rust UI 框架（Yew/Dioxus 等）：直接把 crate 编译进去；
- CLI：同样直接编译——用 CLI 包一层核心是绝佳的外侧测试方式（想象银行 app 的存款命令行测试）。
- 类型通过 serde-generate 序列化（BCS）跨边界传递，类型安全跨语言传播；未来可能用类型擦除减少序列化开销。

### 8. Crux 架构与 Capability（20:40–24:35）

- 绿色"纯路径"：UI 事件 → core 的 update 函数 → 更新 model → 通过内置 Render capability 通知 shell → shell 调 view 拿 ViewModel 绑定 UI。
- Capability 三种形态：fire-and-forget（render）、请求-响应（crux_http，API 抄了 surf 客户端；expect_json 借 Rust 类型推断把回调事件串起来）、流式（SSE 示例，一次请求多次响应）。
- Capability 内可用 async/await，core 里有内置 executor 负责排队收发消息。内置/示例 capability 有 HTTP、KV 存储、时间、平台信息等，自定义 capability 很简单。

### 9. Crux App 长什么样 + 测试（24:35–26:55）

一个 Crux app 就是个零尺寸 struct，带 4 个关联类型：Event（枚举）、Model（内部状态）、ViewModel（给 UI 的视图）、Capabilities。核心是 update 函数：收到 Increment 就加计数、然后请求 render。测试用 AppTester：发一个事件进去，断言"产生了哪些 effect + view 是否符合预期"——纯函数、无 mock、飞快。

### 10. 现场 Demo：Counter（26:55–32:30）

计数器示例（连 fly.io 上的 API，含 SSE 流式推送）。同一个 Rust core 跑出五个客户端：Yew web 版、Next.js 版、CLI（cargo run 增减并 watch）、Android、iOS——在一个端点击 increment，所有端同步更新。代码层面演示了乐观更新、HTTP capability 的 get/post 与回调事件。

### 11. Demo：Notes 协同编辑器（32:30–38:15）

更复杂的示例（Victor 主写）：类 Google Docs 的协同文本编辑，用 CRDT 保证多实例编辑同步。亮点是测试：

- 不开任何 UI/模拟器，直接在测试里实例化两个 core——Alice 和 Bob，Alice 插入文本→把 edits 发给 Bob→断言两人 view 相同（单向同步）；双向同步同理，最后断言得到 "hello world"。
- 整个测试套件用 cargo-nextest 跑完只要 20–30 毫秒。"无副作用、不用 mock、不用 Detox/Appium/Playwright——让我离开那个世界吧。"

### 12. Crux 的本质与 Roadmap（38:15–43:55）

- Crux 是库也算轻量运行时（托管 app、管理与 shell 间消息的异步性）；shell 很薄，只有 UI 布局 + 每个 capability 的一小段平台适配代码。
- App 可组合：像微服务一样按领域拆分、组成层级或并列。
- 下一步：建社区、用它构建大型应用验证、改进人体工学、写完 mdBook 文档（含各平台搭建走查）、做 crux doctor 检查工具、为 capability 提供 shell 端现成实现。
- 总结：共享行为 + 可穷尽且极快的测试反馈 = 更高生产力。致谢共同作者 Victor 和 Graham。

### 13. Q&A（44:05–59:40）

- CLI/shell 端代码什么样？——展示 CLI 的事件循环：收到 core 的 effect（render→调 view；HTTP→实际发请求），承认目前有序列化"包袱"，有 UI 框架的 shell 反而更简单。
- App 层级化的用途？——比如 notes 应用里"加载/保存文档（含定时器）"和"协同编辑"是两个独立关注点，可拆成两个子 app，由父 app 路由消息。
- Capability 用什么写的？SQLite 这类怎么抽象？——capability 是 Rust 写的；现场展示 SSE 和 Time capability 源码（spawn async 上下文、stream_from_shell、逐 chunk 更新 app）。
- 生产可用吗？——"你得相当勇敢"。但 core↔shell 的桥只有三个接口（process_event / handle_response / view），这层 API 基本稳定。
- 部署形态？——就是一个多 target 的 Rust 库：静态库 / 动态库 / WASM，Xcode 配置在书里有完整走查。
- 滚动收缩工具栏这类高频 UI 交互怎么办？——原则是尽量多的逻辑进 Rust、shell 尽量薄；具体场景还没验证，"build something and see what happens"。
- 设备能力差异（如 iPad 有 Pencil）？——shell 可以选择忽略不适用的消息，但穷尽模式匹配 + 跨边界类型流动保证：core 行为一变，所有 shell 编译失败（Kotlin 缺 when 分支、Swift 缺 case），强迫每个平台显式处理或显式忽略——这正是想要的。

---

**一句话 TLDR**：Crux 的首次公开亮相——把 app 行为做成跨 iOS/Android/Web/CLI 共享的纯 Rust 核心（Functional Core, Imperative Shell + Ports & Adapters），UI 留给各平台原生声明式框架，副作用通过 capability 协议交给 shell 执行，换来毫秒级、零 mock 的行为测试。这个视频是理解当前仓库设计哲学的最佳源头（注意：视频是早期版本，仓库现已演进到 0.19，capability 机制已向 Command/effect routing 演进）。
