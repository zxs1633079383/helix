//! 出站命令编排（HTTP outbound build）。
//!
//! 现网前端 service 层的 wire body 组装编排下沉到此（接管：前端退化为纯渲染 + 语义 invoke）。
//! P1 落地 `send_build`（posts/create body）；P2 落地 **registry 化**（`registry.rs` distributed_slice）
//! + posts 写族（urgent/quick_reply/post_props/template_received，revoke 已在 posts_existing）。
//!
//! 注册表 = 单一真源：`is_outbound` / `handle_outbound` 查 registry，各命令文件零交集
//! （P3a/P3b 各加独立注册文件即可并行，不再共改一处 match）。`mod.rs` 只 re-export。

pub mod send_build;

// P2 registry 骨架 + 既有 10 命令迁移 + posts 写族新增（每命令独立语义 + distributed_slice 注册）。
pub mod channel_existing;
pub mod post_props;
pub mod posts_existing;
pub mod quick_reply;
pub mod registry;
pub mod template_received;
pub mod urgent;

// 对外单一入口：accepts_tick / module.rs dispatch / 集成测试都从此处取。
pub use registry::{handle_outbound, is_outbound};
