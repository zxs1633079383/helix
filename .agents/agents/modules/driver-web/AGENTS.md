# helix-driver-web 模块边界

> 本文件是 `driver-web` 模块的目录级 agent 边界。角色协议见 `.agents/agents/modules/driver-web-agent.md`。

## 语言与可读性

- 默认使用中文沟通、写报告、写 review findings、写终态行。
- commit message 使用 Conventional Commit，但 description/body 用中文；除非本 prompt 明确要求 exact commit message。
- 每个 commit 必须包含结构化 body，禁止只有 subject；body 固定包含 `# 影响范围`、`# 改动影响面`、`# 功能改进/开发/新增`、`# Harness 更新`、`# 验证`。
- `# 影响范围` 和 `# 改动影响面` 必须先用 codebase-memory-mcp（search_graph/trace_path/get_code_snippet）或项目等价索引分析调用链/字段/接口影响面后再写；不足时用 rg/测试/日志补证。
- 提交时使用 subject + body 两段，例如 `git commit -m "<type>(<scope>): <中文描述>" -m "<结构化 body>"`，不要只用单个 `-m`。
- 命令、路径、代码符号、协议字段、错误原文可以保留英文，但必须补中文解释。
- 如果发现上游模板是英文，先翻译成中文再执行，不要把英文模板原样作为最终报告。

## 模块责任

正式 Web/WASM 平台壳，兑现 Browser Fetch、当前页面 session store、timer、外部物理 WebSocket bridge 和有界批量事件发布。

`browser/` glue 只负责浏览器 API、物理 socket 生命周期与不透明帧搬运，不提供产品 UI，不解释 IM 协议。业务 HTTP 请求调度、query/local-first/fallback、sync/cursor/echo 与 WS parser 继续由 Rust/`helix-im` 持有。

## 负责路径

- `crates/helix-driver-web`

## 对应领域

- `runtime-core` -> `.agents/agents/domains/runtime-core/AGENTS.md`
- `im-sync` -> `.agents/agents/domains/im-sync/AGENTS.md`

## 触发词

- `web`
- `wasm`
- `browser`
- `fetch`
- `websocket`
- `session-store`

## 必带约束

<!-- agent-constraints:start -->
- `hxc002-send-lifetime`
- `hxc003-web-dual-target`
- `hxc004-features-additive`
- `web-browser-glue-no-business`
<!-- agent-constraints:end -->

## Mermaid 图谱

- `.agents/diagrams/modules/driver-web.mmd`

## 工作规则

- 先读取 Router 给出的 required references，再读取本边界和角色协议。
- 只在本模块拥有的路径内改动；跨领域或跨 crate 时请求 Router 同时带对应 domain agent。
- JavaScript 不得绕过 Rust 门面直接实现业务 HTTP/query 或解析 WS 业务帧；`helix-core` 不得引入 Browser API/runtime。
- 浏览器输出队列必须有界，WebSocket 握手不得依赖浏览器不能设置的自定义 header。
- 报告必须分清已验证、未验证、风险和下一步。
