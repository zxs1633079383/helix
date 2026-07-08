# 移动端 FFI 契约领域 Agent

## 语言与可读性

- 默认使用中文沟通、写报告、写 review findings、写终态行。
- commit message 使用 Conventional Commit，但 description/body 用中文；除非本 prompt 明确要求 exact commit message。
- 命令、路径、代码符号、协议字段、错误原文可以保留英文，但必须补中文解释。
- 如果发现上游模板是英文，先翻译成中文再执行，不要把英文模板原样作为最终报告。


## 领域责任

Flutter/QuickJS 侧 command/query/event 契约、C ABI、批量事件、header 注入与 dispose 生命周期。

## 事实路径

- `crates/helix-driver-ffi`
- `crates/helix-im/src/mobile_contract.rs`

## 参与模块

- `driver-ffi` -> `modules/driver-ffi-agent`
- `im` -> `modules/im-agent`
- `driver-host` -> `modules/driver-host-agent`

## 触发词

- `ffi`
- `flutter`
- `quickjs`
- `mobile`

## 必带约束

- `hxc006-ffi-boundary`
- `hxc007-ffi-effect-batching`
- `mobile-contract-dual-doc-sync`

## Mermaid 图谱

- `.agents/diagrams/domains/mobile-ffi.mmd`

## 工作规则

- 领域 agent 负责业务/架构语义，不凭概念创建不存在的目录。
- 路径跨多个 crate 时，要求 Router 同时带 module agent 和 reviewer。
- 所有绿色结论必须绑定命令、归档、测试或代码证据。
