# helix-core 模块边界

> 本文件是 `core` 模块的目录级 agent 边界。角色协议见 `.agents/agents/modules/core-agent.md`。

## 语言与可读性

- 默认使用中文沟通、写报告、写 review findings、写终态行。
- commit message 使用 Conventional Commit，但 description/body 用中文；除非本 prompt 明确要求 exact commit message。
- 每个 commit 必须包含结构化 body，禁止只有 subject；body 固定包含 `# 影响范围`、`# 改动影响面`、`# 功能改进/开发/新增`、`# 验证`。
- `# 影响范围` 和 `# 改动影响面` 必须先用 codebase-memory-mcp（search_graph/trace_path/get_code_snippet）或项目等价索引分析调用链/字段/接口影响面后再写；不足时用 rg/测试/日志补证。
- 提交时使用 subject + body 两段，例如 `git commit -m "<type>(<scope>): <中文描述>" -m "<结构化 body>"`，不要只用单个 `-m`。
- 命令、路径、代码符号、协议字段、错误原文可以保留英文，但必须补中文解释。
- 如果发现上游模板是英文，先翻译成中文再执行，不要把英文模板原样作为最终报告。

## 模块责任

运行时无关的 Module 宿主、ports、Effect 与确定性执行壳；不得感知 IM 业务或具体 runtime。

## 负责路径

- `crates/helix-core`

## 对应领域

- `runtime-core` -> `.agents/agents/domains/runtime-core/AGENTS.md`

## 触发词

- `core`
- `ports`
- `effect`
- `module`
- `sans-io`

## 必带约束

<!-- agent-constraints:start -->
- `hxc001-core-zero-runtime`
- `hxc002-send-lifetime`
- `hxc004-features-additive`
- `hxc005-hot-path`
- `runtime-core-no-business-schema`
<!-- agent-constraints:end -->

## Mermaid 图谱

- `.agents/diagrams/modules/core.mmd`

## 工作规则

- 先读取 Router 给出的 required references，再读取本边界和角色协议。
- 只在本模块拥有的路径内改动；跨领域或跨 crate 时请求 Router 同时带对应 domain agent。
- 不把本模块便利实现推入错误层级；依赖方向必须服从 `driver -> core` 与 Helix 北极星。
- 报告必须分清已验证、未验证、风险和下一步。
