# 投影与可观测性领域 Agent

## 语言与可读性

- 默认使用中文沟通、写报告、写 review findings、写终态行。
- commit message 使用 Conventional Commit，但 description/body 用中文；除非本 prompt 明确要求 exact commit message。
- 命令、路径、代码符号、协议字段、错误原文可以保留英文，但必须补中文解释。
- 如果发现上游模板是英文，先翻译成中文再执行，不要把英文模板原样作为最终报告。


## 领域责任

im:__bus__ 投影契约、RecordingSink/BatchSink 接缝、trace sidecar 与 hop coverage。

## 事实路径

- `crates/helix-im/docs/projection-schema.md`
- `crates/helix-driver-host/src/instrument.rs`
- `crates/helix-driver-host/src/trace.rs`

## 参与模块

- `im` -> `modules/im-agent`
- `driver-host` -> `modules/driver-host-agent`
- `driver-native` -> `modules/driver-native-agent`
- `governance` -> `modules/governance-agent`

## 触发词

- `projection`
- `RecordingSink`
- `trace`
- `bus-event`

## 必带约束

- `recording-sink-batch-boundary`
- `trace-sidecar-no-business-payload`
- `hxc015-task-notification-no-polling`

## Mermaid 图谱

- `.agents/diagrams/domains/projection-observability.mmd`

## 工作规则

- 领域 agent 负责业务/架构语义，不凭概念创建不存在的目录。
- 路径跨多个 crate 时，要求 Router 同时带 module agent 和 reviewer。
- 所有绿色结论必须绑定命令、归档、测试或代码证据。
