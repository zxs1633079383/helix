pub(crate) mod announcement_changed;
pub(crate) mod change_channel_approval;
pub(crate) mod channel_close;
pub(crate) mod channel_created;
pub(crate) mod channel_member_role_updated;
pub(crate) mod channel_member_update;
pub(crate) mod gate;
pub(crate) mod hello;
pub(crate) mod ignored;
pub(crate) mod increment_channel;
pub(crate) mod increment_channel_end;
pub(crate) mod pong;
pub(crate) mod post;
pub(crate) mod post_chain;
pub(crate) mod category_chain;
pub(crate) mod post_echo_settle;
pub(crate) mod post_pin;
pub(crate) mod post_read;
pub(crate) mod post_schedule_canceled;
pub(crate) mod post_schedule_created;
pub(crate) mod post_update;
pub(crate) mod posted;
pub(crate) mod posts_update;
pub(crate) mod quit_company;
pub(crate) mod update_channel;
pub(crate) mod update_channel_member_nickname;
pub(crate) mod update_channel_notice;

/// `inventory` 的 wasm ctor 只有在对应 rlib 成员被最终链接时才会存在。显式触碰每个 handler
/// 模块，避免 wasm-ld 把“仅靠 ctor 可达”的注册项静默裁掉。
#[cfg(target_arch = "wasm32")]
pub(crate) fn force_link_inventory() {
    announcement_changed::inventory_link_anchor();
    change_channel_approval::inventory_link_anchor();
    channel_close::inventory_link_anchor();
    channel_created::inventory_link_anchor();
    channel_member_role_updated::inventory_link_anchor();
    channel_member_update::inventory_link_anchor();
    hello::inventory_link_anchor();
    ignored::inventory_link_anchor();
    increment_channel::inventory_link_anchor();
    increment_channel_end::inventory_link_anchor();
    pong::inventory_link_anchor();
    post::inventory_link_anchor();
    post_chain::inventory_link_anchor();
    category_chain::inventory_link_anchor();
    post_pin::inventory_link_anchor();
    post_read::inventory_link_anchor();
    post_schedule_canceled::inventory_link_anchor();
    post_schedule_created::inventory_link_anchor();
    post_update::inventory_link_anchor();
    posted::inventory_link_anchor();
    posts_update::inventory_link_anchor();
    quit_company::inventory_link_anchor();
    update_channel::inventory_link_anchor();
    update_channel_member_nickname::inventory_link_anchor();
    update_channel_notice::inventory_link_anchor();
}
