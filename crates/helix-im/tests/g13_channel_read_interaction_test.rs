use helix_core::effect::Effect;
use helix_core::Correlation;
use helix_im::commands::handle_outbound;
use helix_im::projector::unread_summary::{
    ChannelReadObservation, ChannelReadObservationDecision, ChannelReadObservationTracker,
};
use serde_json::{json, Value};

fn one_http(name: &str, payload: Value) -> (String, Value) {
    let effects = handle_outbound(
        name,
        &serde_json::to_vec(&payload).unwrap(),
        "http://x/api/cses",
        "http://x/api/cses",
        Some("conn-g13"),
        Correlation::from_raw(13),
    )
    .expect("canonical read command");
    match effects.as_slice() {
        [Effect::Http { req, .. }] => (
            req.url.clone(),
            serde_json::from_slice(req.body.as_ref().unwrap()).unwrap(),
        ),
        other => panic!("expected one HTTP effect, got {other:?}"),
    }
}

#[test]
fn stable_visibility_advances_once_per_monotonic_boundary() {
    let mut tracker = ChannelReadObservationTracker::default();
    let first = ChannelReadObservation::new(7, "window-1", 41, true);
    assert_eq!(
        tracker.observe(first.clone()),
        ChannelReadObservationDecision::AwaitingStability
    );
    assert_eq!(
        tracker.observe(first),
        ChannelReadObservationDecision::Advance { boundary: 41 }
    );
    assert_eq!(
        tracker.observe(ChannelReadObservation::new(7, "window-1", 41, true)),
        ChannelReadObservationDecision::Noop
    );
    assert_eq!(
        tracker.observe(ChannelReadObservation::new(7, "window-1", 40, true)),
        ChannelReadObservationDecision::Noop
    );
}

#[test]
fn epoch_or_window_change_requires_a_new_stability_pair() {
    let mut tracker = ChannelReadObservationTracker::default();
    tracker.observe(ChannelReadObservation::new(7, "window-1", 41, true));
    assert_eq!(
        tracker.observe(ChannelReadObservation::new(8, "window-1", 42, true)),
        ChannelReadObservationDecision::AwaitingStability
    );
    assert_eq!(
        tracker.observe(ChannelReadObservation::new(8, "window-2", 42, true)),
        ChannelReadObservationDecision::AwaitingStability
    );
    assert_eq!(
        tracker.observe(ChannelReadObservation::new(8, "window-2", 42, true)),
        ChannelReadObservationDecision::Advance { boundary: 42 }
    );
}

#[test]
fn canonical_channel_view_wire_does_not_smuggle_read_authority() {
    let (url, body) = one_http(
        "im_channels_view",
        json!({"channels":[{"id":"channel-13","isRoot":true}]}),
    );
    assert_eq!(url, "http://x/api/cses/channels/view");
    assert_eq!(
        body,
        json!({"channels":[{"id":"channel-13","isRoot":true}]})
    );
    assert!(body.get("unreadCount").is_none());
    assert!(body.get("userId").is_none());
}
