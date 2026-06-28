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
// posts 审批写族（post/approval/approval，{postId}）：补齐 posts 域唯一缺失写命令。零交集独立文件。
pub mod posts_approval;
// CP：posts/createPosts 批量转发（补齐唯一挂起 outbound，88→89）。独立文件零交集。
pub mod posts_relay;
// P3a posts 读族（G1 并行，独立 distributed_slice 注册，与 channel* 零交集）。
pub mod posts_read;
// posts 读族扩展（bookmark/announcement/post-read-list，守 posts_read.rs ≤300 行硬顶）。
pub mod posts_read_ext;
// 公告写族（save/delete/read，post/ 单数子路由，与 posts_read 公告读族同域）。
pub mod posts_announcement;
// P3b channel 写族（G1 并行，独立 distributed_slice 注册，与 posts_read 零交集）：
// createSpecifyOwner / change(info/source/picture/notice/top) / member(role/notify)。
pub mod channel_change;
// channel 写族补全（HTTP-coverage 补齐，与 channel_change.rs 零交集）：专用单字段写路由
// displayName/orient/purpose + add·remove/manger + enableApproval + channels/view（Go 各独立 handler）。
pub mod channel_change_dedicated;
pub mod channel_create;
// channel 读族补全（is_read 回灌）：load notice/postPinned/admin/incrementByChannelId + query +
// onlineStatus + channels/member/byIds + member/snapshot（HTTP 体即数据，无 WS 回声）。
pub mod channel_member_admin;
pub mod channel_read;
// 消息置顶族（postPinned add/remove）：set/unset message top。注意与 channel_change 的对话置顶区分。
pub mod channel_pinned;
pub mod quick_reply;
pub mod registry;
pub mod template_received;
pub mod urgent;
// P5a user-misc 域（G2 并行，独立 distributed_slice 注册，与 P5b bot_agent 零交集）：
// users(list/status) / teams(upsert/member add/quit) / modules/getAll / notification/loadSend /
// search 四件套（post/user/channel/do）。presence（D2 自建）= users/status/ids 接管。
pub mod user_misc;
// P5a build 层 helper 外提（user_misc.rs 行数基线收口·纯函数零行为变更）：
mod user_misc_helpers;
// P5b bot-agent 域（G2 并行，目录模块 6 子族独立 distributed_slice 注册，与 P5a user_misc 零交集）：
// gateway(createBot/botCallback) / bot_manage(CRUD 9) / config(config+channel 10) /
// calls(bot-token 调用 6，auth_kind=Bot) / agents(会话 5) / webhook(config 3)。
// 排除 timeline/cost(#33/34 D7 废弃) + cross-repo demo(#38-40 非业务)。
pub mod bot_agent;
// vote/score 第二网关族（spec06 方案 A，5 命令 gateway()=Default 走 ImConfig.default_api_base_url）：
// createVote/readVote/deleteVote + average/read·average/delete。独立文件零交集（仅 mod.rs append）。
pub mod vote_score;

// 对外单一入口：accepts_tick / module.rs dispatch / 集成测试都从此处取。
pub use registry::{
    canonical_command_name, handle_outbound, is_outbound, is_read, outbound_command_count,
};
