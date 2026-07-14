use serde_json::json;

mod normalize;

pub use normalize::normalize_command_payload;

pub const CONTRACT_VERSION: u32 = 1;
pub const BUS_CHANNEL: &str = "im:__bus__";

pub struct CommandAlias {
    pub public: &'static str,
    pub core: &'static str,
}

pub const COMMAND_ALIASES: &[CommandAlias] = &[
    CommandAlias {
        public: "im_send",
        core: "im_send_message",
    },
    CommandAlias {
        public: "im_relay_messages",
        core: "im_create_posts",
    },
    CommandAlias {
        public: "im_read_channel",
        core: "im_channels_view",
    },
    CommandAlias {
        public: "im_sync_channels",
        core: "im_channel_load_increment_by_channel_id",
    },
    CommandAlias {
        public: "im_ensure_channel_loaded",
        core: "im_channel_load_increment_by_channel_id",
    },
    CommandAlias {
        public: "im_locate_post",
        core: "im_get_posts",
    },
    CommandAlias {
        public: "im_channel_create",
        core: "im_create_channel",
    },
    CommandAlias {
        public: "im_member_leave",
        core: "im_channel_leave",
    },
    CommandAlias {
        public: "im_channel_settings",
        core: "im_channel_change_info",
    },
    CommandAlias {
        public: "im_post_pin",
        core: "im_set_message_top",
    },
    CommandAlias {
        public: "im_announcement_write",
        core: "im_announcement_save",
    },
    CommandAlias {
        public: "im_online_status",
        core: "im_channel_online_status",
    },
    CommandAlias {
        public: "im_members_by_ids",
        core: "im_channels_members_by_ids",
    },
    CommandAlias {
        public: "im_list_candidate_users",
        core: "im_user_candidates",
    },
    CommandAlias {
        public: "im_channel_admin_change",
        core: "im_channel_member_role",
    },
    CommandAlias {
        public: "im_member_nickname_change",
        core: "im_update_member_nickname",
    },
    CommandAlias {
        public: "im_todo_query",
        core: "im_query_todo_list",
    },
    CommandAlias {
        public: "im_system_notice_query",
        core: "im_load_target_notifications",
    },
    CommandAlias {
        public: "im_module_read",
        core: "im_get_all_modules",
    },
    CommandAlias {
        public: "im_module_read_all",
        core: "im_get_all_modules",
    },
    CommandAlias {
        public: "im_company_group_upsert",
        core: "im_team_upsert",
    },
    CommandAlias {
        public: "im_company_group_maintain",
        core: "im_team_member_add",
    },
    CommandAlias {
        public: "im_company_exit",
        core: "im_team_quit",
    },
];

pub const PUBLIC_EVENTS: &[&str] = &[
    "im:post:sending",
    "im:post:send-failed",
    "im:post:received",
    "im:post:updated",
    "im:posts:batch-result",
    "im:post:read",
    "im:post:readers",
    "im:channel:read_echo",
    "im:post:deleted",
    "im:post:batch-updated",
    "im:messages:query_result",
    "im:messages:older_loaded",
    "im:channels:projection",
    "im:channels:loaded",
    "im:channel:increment",
    "im:channel:update",
    "im:channel:update-by-post",
    "im:channel:created",
    "im:channel:closed",
    "im:channel:schedule-created",
    "im:channel:schedule-canceled",
    "im:channel:member-updated",
    "im:channel:settings-updated",
    "im:channel:left",
    "im:channel:memberNickname",
    "im:channel:members",
    "im:channel:replies",
    "im:post:located",
    "im:user:candidates",
    "im:read:result",
    "im:todo:updated",
    "im:health:result",
    "im:net:reconnect_requested",
    "im:http:unauthorized",
    "im:net:offline",
];

pub const META_QUERIES: &[&str] = &["helix_capabilities", "helix_version"];

pub fn command_alias(public: &str) -> Option<&'static str> {
    COMMAND_ALIASES
        .iter()
        .find(|alias| alias.public == public)
        .map(|alias| alias.core)
}

pub fn capabilities_json() -> Vec<u8> {
    let mut commands = crate::outbound::outbound_command_names();
    commands.push("im_send");
    commands.push("im_retry_send");
    commands.push("im_reconnect");
    commands.sort_unstable();
    commands.dedup();

    let mut queries = META_QUERIES.to_vec();
    queries.extend_from_slice(crate::query::query_command_names());
    queries.sort_unstable();
    queries.dedup();

    let value = json!({
        "abi": CONTRACT_VERSION,
        "bus": BUS_CHANNEL,
        "commands": commands,
        "queries": queries,
        "events": PUBLIC_EVENTS,
        "features": {
            "pendingSendTimeoutUnsend": true,
            "sendFailedProjection": true,
            "batchEventCallback": true,
            "readResultReqId": true,
            "noPerBusinessAbiSymbol": true
        }
    });
    serde_json::to_vec(&value).expect("static capabilities JSON must serialize")
}

#[cfg(test)]
#[path = "mobile_contract_tests.rs"]
mod tests;
