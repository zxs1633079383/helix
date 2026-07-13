# helix-im 模块 Agent

## 语言与可读性

- 默认使用中文沟通、写报告、写 review findings、写终态行。
- commit message 使用 Conventional Commit，但 description/body 用中文；除非本 prompt 明确要求 exact commit message。
- 命令、路径、代码符号、协议字段、错误原文可以保留英文，但必须补中文解释。
- 如果发现上游模板是英文，先翻译成中文再执行，不要把英文模板原样作为最终报告。


## 角色

首个业务模块，拥有 IM command/query/event、发送状态机、sync-v2、WS handler 与投影 payload 契约。

## 负责路径

- `crates/helix-im`

## 触发词

- `im`
- `message`
- `channel`
- `sync`
- `outbound`
- `ws`
- `projection`

## 对应领域

- `im-message` -> `domains/im-message-domain-agent`
- `im-channel` -> `domains/im-channel-domain-agent`
- `im-sync` -> `domains/im-sync-domain-agent`
- `projection-observability` -> `domains/projection-observability-domain-agent`
- `runtime-core` -> `domains/runtime-core-domain-agent`

## 必带约束

- `hxc010-deterministic-effect-order`
- `hxc012-expect-for-substantial-task`
- `projection-schema-lock`
- `csesapi-consumption-truth`

## 默认 Reviewer

- `reviewers/contract-reviewer-agent`
- `reviewers/evidence-reviewer-agent`
- `reviewers/test-reviewer-agent`

## Mermaid 图谱

- `.agents/diagrams/modules/im.mmd`

## 工作规则

- 先读取 Router 给出的 required references，再读本模块就近 AGENTS。
- 不把本模块便利实现推入错误层级；跨层变化必须显式请求对应 domain agent。
- 报告中分清已验证、未验证、风险和下一步。
