//! 出站命令编排（HTTP outbound build）。
//!
//! 现网前端 service 层的 wire body 组装编排下沉到此（接管：前端退化为纯渲染 + 语义 invoke）。
//! P1 落地 `send_build`（posts/create body）；P2 落地 **registry 化**（`registry.rs` distributed_slice）
//! + posts 写族（urgent/quick_reply/post_props/template_received，revoke 已在 posts_existing）。
//!
//! 注册表 = 单一真源：`is_outbound` / `handle_outbound` 查 registry，各命令文件零交集
//! （P3a/P3b 各加独立注册文件即可并行，不再共改一处 match）。`mod.rs` 只 re-export。

pub mod send_build;

pub mod channel_admin;
pub use channel_admin::change as channel_change;
pub use channel_admin::change_dedicated as channel_change_dedicated;
pub use channel_admin::create as channel_create;
pub use channel_admin::existing as channel_existing;
pub use channel_admin::member_admin as channel_member_admin;
pub use channel_admin::pinned as channel_pinned;
pub use channel_admin::read as channel_read;
pub mod posts;
pub use posts::announcement as posts_announcement;
pub use posts::approval as posts_approval;
pub use posts::existing as posts_existing;
pub use posts::props as post_props;
pub use posts::read as posts_read;
pub use posts::read_ext as posts_read_ext;
pub use posts::relay as posts_relay;
pub mod engagement;
pub use engagement::quick_reply;
pub use engagement::template_received;
pub use engagement::urgent;
pub use engagement::vote_score;
pub mod registry;
// P5a user-misc 域（G2 并行，独立 distributed_slice 注册，与 P5b bot_agent 零交集）：
// users(list/status) / teams(upsert/member add/quit) / modules/getAll / notification/loadSend /
// search 四件套（post/user/channel/do）。presence（D2 自建）= users/status/ids 接管。
pub mod user_misc;
// P5b bot-agent 域（G2 并行，目录模块 6 子族独立 distributed_slice 注册，与 P5a user_misc 零交集）：
// gateway(createBot/botCallback) / bot_manage(CRUD 9) / config(config+channel 10) /
// calls(bot-token 调用 6，auth_kind=Bot) / agents(会话 5) / webhook(config 3)。
// 排除 timeline/cost(#33/34 D7 废弃) + cross-repo demo(#38-40 非业务)。
pub mod bot_agent;
// 对外单一入口：accepts_tick / module.rs dispatch / 集成测试都从此处取。
pub use registry::{
    canonical_command_name, handle_outbound, is_outbound, is_read, outbound_command_count,
};
