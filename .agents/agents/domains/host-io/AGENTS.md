# Host I/O 领域边界

> 本文件是 `host-io` 领域的目录级 agent 边界。角色协议见 `.agents/agents/domains/host-io-domain-agent.md`；本文件与 `CLAUDE.md` 必须保持完全一致。

## 语言与可读性

- 默认使用中文沟通、写报告、写 review findings、写终态行。
- commit message 使用 Conventional Commit，但 description/body 用中文；除非本 prompt 明确要求 exact commit message。
- 每个 commit 必须包含结构化 body，禁止只有 subject；body 固定包含 `# 影响范围`、`# 改动影响面`、`# 功能改进/开发/新增`、`# 验证`。
- `# 影响范围` 和 `# 改动影响面` 必须先用 codebase-memory-mcp（search_graph/trace_path/get_code_snippet）或项目等价索引分析调用链/字段/接口影响面后再写；不足时用 rg/测试/日志补证。
- 提交时使用 subject + body 两段，例如 `git commit -m "<type>(<scope>): <中文描述>" -m "<结构化 body>"`，不要只用单个 `-m`。
- 命令、路径、代码符号、协议字段、错误原文可以保留英文，但必须补中文解释。
- 如果发现上游模板是英文，先翻译成中文再执行，不要把英文模板原样作为最终报告。

## 领域责任

维护共享 SQLite storage、HTTP/WS transport、host trace/instrument 以及本地 store identity。

## 事实路径

- `crates/helix-driver-host`

## 参与模块

- `driver-host` -> `.agents/agents/modules/driver-host/AGENTS.md`
- `driver-ffi` -> `.agents/agents/modules/driver-ffi/AGENTS.md`
- `driver-native` -> `.agents/agents/modules/driver-native/AGENTS.md`
- `host-cli` -> `.agents/agents/modules/host-cli/AGENTS.md`

## 必带约束

<!-- agent-constraints:start -->
- `hxc005-hot-path`
- `local-store-identity-boundary`
- `trace-sidecar-no-business-payload`
<!-- agent-constraints:end -->

## Mermaid 图谱

- `.agents/diagrams/domains/host-io.mmd`

## 工作规则

- 领域 agent 负责业务/架构语义；涉及真实代码目录时必须同时带 owning module agent。
- 概念性领域不要凭空创建业务目录；边界事实以本文件、manifest 和 required references 为准。
- 路径跨多个 crate 时，要求 Router 同时带 module agent 和 reviewer。
- 所有绿色结论必须绑定命令、归档、测试或代码证据。
