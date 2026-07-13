# helix-driver-native 模块 Agent

## 语言与可读性

- 默认使用中文沟通、写报告、写 review findings、写终态行。
- commit message 使用 Conventional Commit，但 description/body 用中文；除非本 prompt 明确要求 exact commit message。
- 命令、路径、代码符号、协议字段、错误原文可以保留英文，但必须补中文解释。
- 如果发现上游模板是英文，先翻译成中文再执行，不要把英文模板原样作为最终报告。


## 角色

PC/Tauri 薄壳，负责 native runtime 接缝和 host driver 组装，不拥有业务命令真相。

## 负责路径

- `crates/helix-driver-native`

## 触发词

- `native`
- `tauri`
- `pc`
- `event-sink`

## 对应领域

- `native-tauri` -> `domains/native-tauri-domain-agent`
- `host-io` -> `domains/host-io-domain-agent`
- `projection-observability` -> `domains/projection-observability-domain-agent`

## 必带约束

- `native-thin-shell`
- `recording-sink-batch-boundary`
- `tauri-four-seams-p0`

## 默认 Reviewer

- `reviewers/contract-reviewer-agent`
- `reviewers/evidence-reviewer-agent`

## Mermaid 图谱

- `.agents/diagrams/modules/driver-native.mmd`

## 工作规则

- 先读取 Router 给出的 required references，再读本模块就近 AGENTS。
- 不把本模块便利实现推入错误层级；跨层变化必须显式请求对应 domain agent。
- 报告中分清已验证、未验证、风险和下一步。
