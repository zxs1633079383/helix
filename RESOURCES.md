# Helix 消息发送链资源

## Knowledge

- [当前发送状态机：`crates/helix-im/src/send/flow.rs`](crates/helix-im/src/send/flow.rs)
  本仓库发送命令的业务入口；用于理解 optimistic persist、HTTP 和 timeout 的顺序。
- [当前发送对账：`crates/helix-im/src/ws/handlers/post_echo_settle.rs`](crates/helix-im/src/ws/handlers/post_echo_settle.rs)
  本端 WebSocket `post` echo 如何按 `temporaryId` 收敛为 `sent`。
- [MessageV3 post 提交：`crates/helix-im/src/port_reply/message_v3_post.rs`](crates/helix-im/src/port_reply/message_v3_post.rs)
  `PersistAtomic → viewer read-back → im:post:received` 的权威终态链。
- [发送生命周期回放测试：`crates/helix-im/tests/message_v3/post/mv3_g01a/main.rs`](crates/helix-im/tests/message_v3/post/mv3_g01a/main.rs)
  用可证伪断言区分 sending、HTTP admission、WS echo 和终态事件。
- [The Rust Programming Language — Enums and Pattern Matching](https://doc.rust-lang.org/book/ch06-00-enums.html)
  用于本课中的 `Tick`、`PortOutcome`、`SendStatus` 和 `match`。
- [The Rust Programming Language — Error Handling](https://doc.rust-lang.org/book/ch09-00-en-error-handling.html)
  用于本课中的 `Result`、`Err` 和 `?`。
- [The Rust Programming Language — Writing Automated Tests](https://doc.rust-lang.org/book/ch11-00-testing.html)
  用于理解 `#[test]`、集成测试、断言与失败输出。
- [Cargo Book — cargo test](https://doc.rust-lang.org/cargo/commands/cargo-test.html)
  用于把 Cargo package、integration target、过滤器和 `--nocapture` 参数连起来。

## Wisdom (Communities)

- 本阶段不引入外部社区；先以本仓库的源码、测试和真实边界证据为学习反馈。

## Gaps

- 当前课程还没有覆盖真实 Tauri `app.emit("im:__bus__", ...)` 到 Angular DOM 的运行态练习；需要另一个具备认证和宿主前置的课程。
