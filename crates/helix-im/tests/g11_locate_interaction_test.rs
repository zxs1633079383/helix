//! G11 focused contract: locate identity belongs to the VM, not directive payload.

use helix_im::render_ready_locate::{normalize_located_window, reveal_directive};

#[test]
/// 证明 target 身份属于 typed VM，而 directive 始终保持 payload-free。
fn locate_requires_target_in_matching_channel_and_uses_payload_free_directive() {
    let rows = normalize_located_window(
        br#"{"data":[
            {"id":"before","channelId":"channel-11","userId":"444"},
            {"id":"target","channelId":"channel-11","userId":"678"}
        ]}"#,
        "channel-11",
        "target",
        "444",
    )
    .unwrap();
    assert!(rows.iter().any(|row| row["msgId"] == "target"));
    assert!(normalize_located_window(
        br#"{"data":[{"id":"target","channelId":"other","userId":"678"}]}"#,
        "channel-11",
        "target",
        "444",
    )
    .is_err());
    assert_eq!(
        serde_json::to_value(reveal_directive("reveal-11")).unwrap(),
        serde_json::json!({"directiveId":"reveal-11","kind":"revealMessage"})
    );
}
