//! G01 普通文本发送的 Helix 侧完整 sans-IO 回放包。
//!
//! 测试只覆盖 Helix 拥有的边界：
//! `Tick::{Command, PortReply, Inbound} -> Effect -> Persist barrier -> InteractionFrameV2`。
//! Angular `SurfaceRuntime`、Tauri IPC、SQLite 执行器、Go/Pulsar/PG 与真实 WS 连接必须在各自
//! integration/E2E 层验证，不能由本包伪造为“已跑全链”。

#[path = "g01_full_flow/group_prerequisite.rs"]
mod group_prerequisite;
#[path = "g01_full_flow/http_terminal.rs"]
mod http_terminal;
#[path = "g01_full_flow/support.rs"]
mod support;
#[path = "g01_full_flow/ws_inbound.rs"]
mod ws_inbound;
