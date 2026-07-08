# 运行时无关确定性内核领域 Agent

## 语言与可读性

- 默认使用中文沟通、写报告、写 review findings、写终态行。
- commit message 使用 Conventional Commit，但 description/body 用中文；除非本 prompt 明确要求 exact commit message。
- 命令、路径、代码符号、协议字段、错误原文可以保留英文，但必须补中文解释。
- 如果发现上游模板是英文，先翻译成中文再执行，不要把英文模板原样作为最终报告。


## 领域责任

确保 core 只定义 ports/Effect/Module，时间、ID、随机、I/O 全由 host 注入。

## 事实路径

- `crates/helix-core`
- `docs/GOAL.md`

## 参与模块

- `core` -> `modules/core-agent`
- `driver-ffi` -> `modules/driver-ffi-agent`
- `driver-host` -> `modules/driver-host-agent`
- `driver-native` -> `modules/driver-native-agent`

## 触发词

- `sans-io`
- `deterministic`
- `ports`
- `effect`

## 必带约束

- `hxc001-core-zero-runtime`
- `hxc002-send-lifetime`
- `runtime-core-no-business-schema`

## Mermaid 图谱

- `.agents/diagrams/domains/runtime-core.mmd`

## 工作规则

- 领域 agent 负责业务/架构语义，不凭概念创建不存在的目录。
- 路径跨多个 crate 时，要求 Router 同时带 module agent 和 reviewer。
- 所有绿色结论必须绑定命令、归档、测试或代码证据。
