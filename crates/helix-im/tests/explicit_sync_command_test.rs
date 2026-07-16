mod common;

use common::{channel_id, cid, hello_inbound, Harness};
use helix_core::{Effect, Module, Tick};

fn sync_command(channel_id: &str) -> Tick {
    Tick::Command(
        helix_im::client_api::build_command(
            "im_sync_channels",
            serde_json::json!({
                "channelId": channel_id,
                "reason": "offline_revoke_replay",
                "eventSeq": 999_999
            })
            .to_string()
            .as_bytes(),
        )
        .expect("public sync command must normalize"),
    )
}

#[test]
fn public_sync_command_uses_helix_cursor_and_sync_notify_not_hydration() {
    let mut harness = Harness::new(&[]);
    harness.module.register_channel(channel_id(5), 41);
    harness.step(hello_inbound("conn-explicit-sync"), 1_000);
    let command = sync_command(&cid(5));
    assert!(harness.module.accepts(&command));

    harness.step(command, 1_001);

    let (url, body) = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } => Some((req.url.as_str(), req.body.as_ref().unwrap())),
            _ => None,
        })
        .expect("explicit sync must emit one HTTP effect");
    assert!(url.ends_with("/channel/sync/notify"), "url={url}");
    assert!(!url.contains("incrementByChannelId"));
    let body: serde_json::Value = serde_json::from_slice(body.as_ref()).unwrap();
    assert_eq!(body["cursors"][0]["channelId"], cid(5));
    assert_eq!(body["cursors"][0]["fromSeq"], 41);
    assert_ne!(body["cursors"][0]["fromSeq"], 999_999);
}

#[test]
fn explicit_sync_registers_unknown_channel_at_zero_for_authoritative_pull() {
    let mut harness = Harness::new(&[]);
    harness.step(hello_inbound("conn-explicit-unknown"), 1_000);

    harness.step(sync_command(&cid(6)), 1_001);

    let body = harness.last_http_body().expect("sync HTTP body");
    assert_eq!(body["cursors"][0]["channelId"], cid(6));
    assert_eq!(body["cursors"][0]["fromSeq"], 0);
    assert_eq!(harness.module.cursor_for(channel_id(6)), Some(0));
}
