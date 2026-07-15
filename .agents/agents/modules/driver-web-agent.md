# helix-driver-web 模块 Agent

## 语言与可读性

- 默认使用中文沟通、写报告、写 review findings、写终态行。
- commit message 使用 Conventional Commit，但 description/body 用中文；除非本 prompt 明确要求 exact commit message。
- 命令、路径、代码符号、协议字段、错误原文可以保留英文，但必须补中文解释。
- 如果发现上游模板是英文，先翻译成中文再执行，不要把英文模板原样作为最终报告。

## 角色

正式 Web/WASM 平台壳，负责 Browser Fetch、page-session store、timer、外部物理 WebSocket bridge 与批量事件发布；浏览器 glue 无 UI、无业务语义。

## 负责路径

- `crates/helix-driver-web`

## 触发词

- `web`
- `wasm`
- `browser`
- `fetch`
- `websocket`
- `session-store`

## 对应领域

- `runtime-core` -> `domains/runtime-core-domain-agent`
- `im-sync` -> `domains/im-sync-domain-agent`

## 必带约束

- `hxc002-send-lifetime`
- `hxc003-web-dual-target`
- `hxc004-features-additive`
- `web-browser-glue-no-business`

## 默认 Reviewer

- `reviewers/contract-reviewer-agent`
- `reviewers/test-reviewer-agent`
- `reviewers/harness-reviewer-agent`

## Mermaid 图谱

- `.agents/diagrams/modules/driver-web.mmd`

## 工作规则

- 先读取 Router 给出的 required references，再读本模块就近 AGENTS。
- Browser glue 只持有物理 WebSocket 与浏览器生命周期；不得包含产品 UI，也不得复制业务 HTTP/query、sync/cursor/echo 或 WS parser。
- Rust 侧继续持有业务 HTTP 调度、query/local-first/fallback 与 WS 帧解析；不把本模块便利实现推入 `helix-core` 或 JavaScript。
- 跨层变化必须显式请求对应 domain agent；报告中分清已验证、未验证、风险和下一步。
