# Agent Task

## 语言与可读性

- 默认使用中文沟通、写报告、写 review findings、写终态行。
- commit message 使用 Conventional Commit，但 description/body 用中文；除非本 prompt 明确要求 exact commit message。
- 命令、路径、代码符号、协议字段、错误原文可以保留英文，但必须补中文解释。
- 如果发现上游模板是英文，先翻译成中文再执行，不要把英文模板原样作为最终报告。


## 路由结果

- Workflow: {{workflow}}
- Primary agents: {{primary_agents}}
- Reviewer agents: {{reviewer_agents}}
- Required references: {{required_references}}

## 执行规则

- 先读取 Router 给你的 required references；不要预加载无关文档。
- 只在 assigned boundary 内改动；跨边界时写清需要 Router 增派哪些 agent。
- Rust 任务遵守 Cargo 纪律：中途禁反复 cargo，完整模块完成前最多集中验证。
- 涉及实质任务时先写可证伪 EXPECT；收尾写 DONE 或 PARTIAL 终态行。
- 报告必须包含 `agent/status/changed_files/evidence/unverified/risks`。
