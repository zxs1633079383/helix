//! ID-only forward outbound boundary; no legacy body compatibility.
use helix_core::{Correlation, Effect};
use helix_im::commands::{handle_outbound, is_outbound};
use serde_json::{json, Value};
/// Run the actual registered outbound builder.
fn run(value: Value) -> Result<Vec<Effect>, helix_im::ImError> {
    handle_outbound(
        "im_create_posts",
        value.to_string().as_bytes(),
        "http://test",
        "http://test",
        None,
        Correlation::from_raw(1),
    )
}
/// Only the canonical ID tuple is serialized.
#[test]
fn source_ids_and_stable_command_are_the_only_creation_fields() {
    let effects = run(json!({"post_ids":["p"],"target_channel_ids":["chfixx00000000000000000001"],"mode":"merge","command_id":"c","req_id":"r"})).unwrap();
    let Effect::Http { req, .. } = &effects[0] else {
        panic!("HTTP")
    };
    assert!(req.url.ends_with("/posts/createPosts"));
    let body: Value = serde_json::from_slice(req.body.as_ref().unwrap()).unwrap();
    assert_eq!(
        body,
        json!({"postIds":["p"],"targetChannelIds":["chfixx00000000000000000001"],"mode":"merged","commandId":"c"})
    );
    assert!(is_outbound("im_create_posts") && is_outbound("im_forward_detail"));
}
/// Forged post bodies, duplicates and absent stable keys fail closed.
#[test]
fn malformed_or_legacy_creation_is_rejected() {
    let valid = json!({"post_ids":["p"],"target_channel_ids":["chfixx00000000000000000001"],"mode":"item","command_id":"c","req_id":"r"});
    for key in [
        "post_ids",
        "target_channel_ids",
        "mode",
        "command_id",
        "req_id",
    ] {
        let mut args = valid.clone();
        args.as_object_mut().unwrap().remove(key);
        assert!(run(args).is_err());
    }
    for (key, value) in [
        ("posts", json!([{}])),
        ("snapshot_id", json!("forged")),
        ("post_ids", json!(["p", "p"])),
        ("target_channel_ids", json!([])),
        ("mode", json!("merged")),
    ] {
        let mut args = valid.clone();
        args[key] = value;
        assert!(run(args).is_err());
    }
}
