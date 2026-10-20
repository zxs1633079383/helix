//! One admission vocabulary, derived from the same command registries used for dispatch.
use super::command_alias;

/// Platform-neutral command requirement; local-first fallback is guarded at the I/O seam.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandNetworkMode { LocalOnly, LocalFirst, RequiresNetwork }

/// Unknown commands fail closed; aliases are classified only after canonical resolution.
pub fn command_network_mode(name: &str) -> Option<CommandNetworkMode> {
    let name = command_alias(name).unwrap_or(name);
    if crate::outbound::is_outbound(name) || crate::chain::is_command(name)
        || crate::category_chain::is_command(name) { return Some(CommandNetworkMode::RequiresNetwork); }
    if crate::draft::is_draft_command(name) || name == crate::recent_history::COMMAND
        || name == crate::module::RUNTIME_IDENTITY_COMMAND {
        return Some(CommandNetworkMode::LocalOnly);
    }
    if crate::query::is_query(name) {
        return Some(match name {
            "im_query_dialog_list" | "im_query_channel_view_snapshot" | "im_query_subtopics"
                | "im_query_pinned_projection" | "im_delete_all_dialogs" | "im_complete_channel_sync" => CommandNetworkMode::LocalOnly,
            "im_query_messages_by_channel" | "im_load_older_context" | "im_load_newer_context"
                | "im_locate_message" | "im_locate_context" | "im_query_channel_sync_page" => CommandNetworkMode::LocalFirst,
            _ => CommandNetworkMode::RequiresNetwork,
        });
    }
    Some(match name {
        "im_presence_observe" => CommandNetworkMode::LocalFirst,
        "im_cancel_translation" | "im_cancel_forward_detail" | "im_reconnect" => CommandNetworkMode::LocalOnly,
        "im_send_message" | "im_retry_send" | "im_retry_upload" | "im_translate_message"
            | "im_sync_channels" | "im_forward_detail" => CommandNetworkMode::RequiresNetwork,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Inventory remains derived and every entry explicitly declares its execution mode.
    #[test]
    fn inventory_is_complete_and_explicit() {
        let inventory = command_network_inventory();
        let modes = inventory.as_object().unwrap();
        for alias in super::super::COMMAND_ALIASES { assert!(modes.contains_key(alias.public)); }
        assert_eq!(modes["im_bookmark_load"], "requires_network");
        for name in crate::draft::DRAFT_COMMANDS { assert_eq!(modes[*name], "local_only"); }
        assert_eq!(modes[crate::recent_history::COMMAND], "local_only");
        println!("NETWORK_INVENTORY={}", inventory);
    }
    /// Every outbound is remote even when marked read; unknown aliases cannot become local.
    #[test]
    fn outbound_and_public_alias_coverage_is_fail_closed() {
        for name in crate::outbound::outbound_command_names() {
            assert_eq!(command_network_mode(name), Some(CommandNetworkMode::RequiresNetwork), "{name}");
        }
        for alias in super::super::COMMAND_ALIASES {
            assert_eq!(command_network_mode(alias.public), command_network_mode(alias.core));
        }
        assert_eq!(command_network_mode("im_bookmark_load"), Some(CommandNetworkMode::RequiresNetwork));
        assert_eq!(command_network_mode("im_query_dialog_list"), Some(CommandNetworkMode::LocalOnly));
        assert_eq!(command_network_mode("im_query_messages_by_channel"), Some(CommandNetworkMode::LocalFirst));
        assert_eq!(command_network_mode("unclassified_command"), None);
    }
}

/// Discovery uses existing registries; unsupported historical aliases are explicitly rejected.
pub fn command_network_inventory() -> serde_json::Value {
    let mut names=crate::outbound::outbound_command_names();
    names.extend_from_slice(crate::query::query_command_names());
    names.extend_from_slice(crate::draft::DRAFT_COMMANDS);
    names.extend([crate::recent_history::COMMAND, crate::module::RUNTIME_IDENTITY_COMMAND]);
    names.extend_from_slice(crate::chain::COMMANDS);
    names.extend_from_slice(crate::category_chain::wire::COMMANDS);
    names.extend(["im_send_message","im_retry_send","im_retry_upload","im_translate_message","im_cancel_translation","im_presence_observe","im_forward_detail","im_cancel_forward_detail","im_reconnect","im_sync_channels"]);
    names.extend(super::COMMAND_ALIASES.iter().map(|alias|alias.public)); names.sort_unstable(); names.dedup();
    serde_json::Value::Object(names.into_iter().map(|name| {
        let mode=match command_network_mode(name) {Some(CommandNetworkMode::LocalOnly)=>"local_only",Some(CommandNetworkMode::LocalFirst)=>"local_first",Some(CommandNetworkMode::RequiresNetwork)=>"requires_network",None=>"rejected_unclassified"};
        (name.into(),serde_json::Value::String(mode.into()))
    }).collect())
}
