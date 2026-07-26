use helix_im::interaction_frame::{
    ChannelAvatarRef, ChannelLifecycleVm, ChannelTitleRef, PreviewVm, SurfaceLoadState,
    TopicListVm, TopicRowVm, WindowPageVm,
};
use helix_im::projector::topic_list::TopicListProjector;

#[path = "support/g15_scenario.rs"]
mod scenario;

fn row(id: &str, root: &str) -> TopicRowVm {
    TopicRowVm {
        topic_channel_id: id.into(),
        root_message_id: root.into(),
        title: ChannelTitleRef {
            text: "Topic".into(),
        },
        root_message_preview: PreviewVm::System {
            text: "created".into(),
        },
        last_activity_at: 15,
        message_count: 1,
        avatar: ChannelAvatarRef {
            resource: None,
            user_ids: Vec::new(),
            fallback_text: "T".into(),
        },
        lifecycle: ChannelLifecycleVm {
            state: "active".into(),
            reason: None,
        },
        actions: Vec::new(),
    }
}

fn view(items: Vec<TopicRowVm>, topic_count: u32) -> TopicListVm {
    TopicListVm {
        parent_channel_id: "parent-15".into(),
        items,
        page: WindowPageVm {
            window_token: "cursor-15".into(),
            has_older: false,
            has_newer: false,
            has_more: false,
        },
        topic_count,
        state: SurfaceLoadState::Ready,
    }
}

#[test]
fn topic_count_cannot_be_less_than_visible_rows() {
    let mut projector = TopicListProjector::default();
    let error = projector
        .snapshot(view(vec![row("topic-1", "root-1")], 0))
        .unwrap_err();
    assert!(error.to_string().contains("topicCount"));
}

#[test]
fn duplicate_topic_channel_ids_are_rejected() {
    let mut projector = TopicListProjector::default();
    let error = projector
        .snapshot(view(
            vec![row("topic-1", "root-1"), row("topic-1", "root-2")],
            2,
        ))
        .unwrap_err();
    assert!(error.to_string().contains("duplicate topic channel id"));
}

#[test]
/// 证明 G-15 terminal Frame 的 causation 与 topic/root identity 来自生产 correlation。
fn production_terminal_frame_preserves_topic_correlation() {
    let trace = scenario::G15ScenarioRunner::run_http_created();
    scenario::assert_thread_contract(&trace.thread);
}

#[test]
/// 证明 matching relation PersistOk 之前没有 typed terminal Frame。
fn production_relation_persist_is_a_hard_barrier() {
    assert_eq!(scenario::G15ScenarioRunner::run_persist_failure(), 0);
}

#[test]
/// 证明 cold recovery 使用 durable topic row 而非 raw replies reducer。
fn production_recovery_rebuilds_summary_without_raw_event() {
    let trace = scenario::G15ScenarioRunner::run_recovery();
    scenario::assert_summary_contract(&trace.summary);
}
