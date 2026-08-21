# 学习使命：读懂 Helix 的消息发送链

## Why

用户希望以 Rust 小白能跟上的方式，沿着当前源码从 `Tick::Command` 读懂一条消息如何先进入 `sending`，再经过 HTTP、WebSocket、持久化和 MessageV3 事件，最后收敛为 `sent`。

## Success looks like

- 能从 `Tick::Command` 找到 `im_send_message` 的业务 handler。
- 能解释为什么 HTTP 2xx 仍然只是 `sending`，以及什么证据才允许 `sent`。
- 能用 `temporaryId`、`corr`、`PersistOk` 和 `im:post:received` 在相邻边界对账。
- 能读懂相关 Rust 的 `enum`、`match`、`Result`、借用和 `EffectSink::push`。

## Constraints

- 默认使用中文讲解，先讲运行时故事，再解释 Rust 语法。
- 每次只学习一条短链，优先使用当前 checkout 的源码与测试证据。
- 不把静态源码、focused 测试或 HTTP 200 说成当前真实运行 PASS。

## Out of scope

- 本阶段不展开 Angular DOM、Flutter FFI 细节或 Go 服务端实现内部。
- 不修改发送业务代码；先建立阅读和诊断能力。
