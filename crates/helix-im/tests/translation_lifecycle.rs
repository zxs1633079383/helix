//! 翻译公开 Tick seam 生命周期回归。

mod common;

use bytes::Bytes;
use common::{enveloped, port_reply_with_body, timer_tick, Harness};
use helix_core::effect::{Correlation, TimerId};
use helix_core::{Effect, Module, Tick};
use serde_json::{json, Value};

const TRANSLATION_EVENT: &str = "im:translation:result";

fn translation_payload(req_id: &str) -> Value {
    json!({
        "req_id": req_id,
        "channel_id": common::cid(1),
        "message_key": common::sid(1),
        "source_revision": "sha256:body-1",
        "from": "zh",
        "to": "en",
        "text": "你好"
    })
}

fn translate_command(req_id: &str) -> helix_core::tick::AppCommand {
    helix_im::client_api::build_command(
        "im_translate_message",
        serde_json::to_vec(&translation_payload(req_id))
            .expect("translation payload must serialize")
            .as_slice(),
    )
    .expect("translation public command must build")
}

fn cancel_command(req_id: &str) -> helix_core::tick::AppCommand {
    helix_im::client_api::build_command(
        "im_cancel_translation",
        serde_json::to_vec(&json!({ "req_id": req_id }))
            .expect("cancel payload must serialize")
            .as_slice(),
    )
    .expect("translation cancel command must build")
}

fn emitted(h: &Harness) -> Vec<Value> {
    h.sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => Some(
                serde_json::from_slice(event.0.as_ref())
                    .expect("translation event must be a JSON envelope"),
            ),
            _ => None,
        })
        .collect()
}

fn start_translation(h: &mut Harness, req_id: &str) -> (Correlation, TimerId) {
    let tick = Tick::Command(translate_command(req_id));
    assert!(
        h.module.accepts(&tick),
        "translation command must be reachable"
    );
    h.step(tick, 100);
    let corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("translation command must emit one HTTP effect");
    let timer = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::ScheduleTimer { id, .. } => Some(*id),
            _ => None,
        })
        .expect("translation command must schedule a timeout");
    (corr, timer)
}

fn translation_harness() -> Harness {
    Harness::new_with_identity(&[1], "user-1", "company-1", "用户", "公司", "部门")
}

#[test]
fn public_translation_command_routes_to_isolated_service_without_business_side_effects() {
    assert_eq!(
        helix_im::client_api::command_alias("im_translate_message"),
        Some("im_translate_message")
    );
    assert_eq!(
        helix_im::client_api::command_alias("im_cancel_translation"),
        Some("im_cancel_translation")
    );

    let mut h = Harness::new_with_identity(&[], "user-1", "company-1", "用户", "公司", "部门");
    let command = Tick::Command(translate_command("translation-1"));
    assert!(h.module.accepts(&command));
    h.step(command, 100);

    let request = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } => Some(req),
            _ => None,
        })
        .expect("translation command must emit HTTP");
    assert_eq!(request.method, "POST");
    assert_eq!(request.url, "service:translation");
    let body: Value = serde_json::from_slice(
        request
            .body
            .as_ref()
            .expect("translation request must have body")
            .as_ref(),
    )
    .expect("translation request body must be JSON");
    assert_eq!(body, translation_payload("translation-1"));
    assert!(request.headers.iter().all(|(name, _)| {
        !matches!(
            name.to_ascii_lowercase().as_str(),
            "authorization"
                | "cookie"
                | "cookieid"
                | "userid"
                | "user-id"
                | "connectionid"
                | "connection-id"
        )
    }));
    assert!(!h.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Persist { .. } | Effect::PersistAtomic { .. } | Effect::Send { .. }
    )));
    assert!(emitted(&h).is_empty());
}

#[test]
fn translation_request_requires_distinct_supported_languages_and_non_empty_text() {
    let cases = [
        ("fr", "en", "bonjour"),
        ("zh", "zh", "你好"),
        ("zh", "en", ""),
    ];
    for (from, to, text) in cases {
        let mut h = translation_harness();
        let mut payload = translation_payload("translation-invalid");
        let object = payload
            .as_object_mut()
            .expect("translation payload is an object");
        object.insert("from".into(), Value::String(from.into()));
        object.insert("to".into(), Value::String(to.into()));
        object.insert("text".into(), Value::String(text.into()));
        let command = Tick::Command(helix_core::tick::AppCommand::new(
            "im_translate_message",
            Bytes::from(serde_json::to_vec(&payload).expect("invalid translation serializes")),
        ));
        let result = h.module.handle(&command, 100, &mut h.sink);
        assert!(result.is_err(), "invalid language/text must fail closed");
        assert!(!h
            .sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Http { .. })));
    }
}

#[test]
fn translation_success_is_a_single_non_durable_terminal_event_and_late_duplicate_is_ignored() {
    let mut h = translation_harness();
    let (corr, _) = start_translation(&mut h, "translation-success");
    h.step(
        port_reply_with_body(
            corr.raw(),
            enveloped(Bytes::from_static(
                br#"{"text":"Hello","from":"zh","to":"en"}"#,
            )),
        ),
        101,
    );
    let events = emitted(&h);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], TRANSLATION_EVENT);
    assert_eq!(
        events[0]["data"],
        json!({
            "req_id": "translation-success",
            "channel_id": common::cid(1),
            "message_key": common::sid(1),
            "source_revision": "sha256:body-1",
            "from": "zh",
            "to": "en",
            "status": "success",
            "translated_text": "Hello"
        })
    );
    assert!(h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::CancelTimer { .. })));

    h.step(
        port_reply_with_body(
            corr.raw(),
            enveloped(Bytes::from_static(
                br#"{"text":"Late","from":"zh","to":"en"}"#,
            )),
        ),
        102,
    );
    assert!(
        emitted(&h).is_empty(),
        "late duplicate must not emit a second terminal event"
    );
}

#[test]
fn translation_timeout_emits_error_and_late_reply_is_ignored() {
    let mut h = translation_harness();
    let (corr, timer) = start_translation(&mut h, "translation-timeout");
    h.step(timer_tick(timer.raw()), 40_000);
    let events = emitted(&h);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], TRANSLATION_EVENT);
    assert_eq!(events[0]["data"]["status"], "error");
    assert!(events[0]["data"].get("error_code").is_some());

    h.step(
        port_reply_with_body(
            corr.raw(),
            enveloped(Bytes::from_static(
                br#"{"text":"Late","from":"zh","to":"en"}"#,
            )),
        ),
        40_001,
    );
    assert!(
        emitted(&h).is_empty(),
        "timed out request must ignore its late reply"
    );
}

#[test]
fn translation_non_success_http_envelope_is_terminal_error_without_persist() {
    let mut h = translation_harness();
    let (corr, _) = start_translation(&mut h, "translation-http-error");
    h.step(
        port_reply_with_body(
            corr.raw(),
            Bytes::from_static(br#"{"status":401,"headers":{},"body":"dW5hdXRob3JpemVk"}"#),
        ),
        101,
    );
    let events = emitted(&h);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["status"], "error");
    assert_eq!(events[0]["data"]["error_code"], "TRANSLATION_HTTP_ERROR");
    assert!(!h.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Persist { .. } | Effect::PersistAtomic { .. } | Effect::Send { .. }
    )));
}

#[test]
fn translation_cancel_emits_cancelled_cancels_timer_and_ignores_late_reply() {
    let mut h = translation_harness();
    let (corr, timer) = start_translation(&mut h, "translation-cancel");
    let cancel = Tick::Command(cancel_command("translation-cancel"));
    assert!(
        h.module.accepts(&cancel),
        "translation cancel must be reachable"
    );
    h.step(cancel, 101);
    let events = emitted(&h);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], TRANSLATION_EVENT);
    assert_eq!(events[0]["data"]["status"], "cancelled");
    assert!(h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::CancelTimer { id } if *id == timer)));

    h.step(
        port_reply_with_body(
            corr.raw(),
            enveloped(Bytes::from_static(
                br#"{"text":"Late","from":"zh","to":"en"}"#,
            )),
        ),
        102,
    );
    assert!(
        emitted(&h).is_empty(),
        "cancelled request must ignore its late reply"
    );
}

#[test]
fn translation_duplicate_is_idempotent_and_conflict_is_error() {
    let mut h = translation_harness();
    let command = translate_command("translation-duplicate");
    h.step(Tick::Command(command.clone()), 100);
    let first_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("first translation command emits HTTP");
    h.step(Tick::Command(command), 101);
    assert!(h
        .sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Http { .. })));
    h.step(
        Tick::Command(helix_core::tick::AppCommand::new(
            "im_translate_message",
            serde_json::to_vec(&json!({
                "req_id": "translation-duplicate",
                "channel_id": common::cid(1),
                "message_key": common::sid(1),
                "source_revision": "sha256:different",
                "from": "zh",
                "to": "en",
                "text": "你好"
            }))
            .expect("conflicting translation serializes"),
        )),
        102,
    );
    let events = emitted(&h);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["status"], "error");
    assert_eq!(
        events[0]["data"]["error_code"],
        "TRANSLATION_REQUEST_CONFLICT"
    );
    h.step(
        port_reply_with_body(
            first_corr.raw(),
            enveloped(Bytes::from_static(
                br#"{"text":"Hello","from":"zh","to":"en"}"#,
            )),
        ),
        103,
    );
    assert_eq!(emitted(&h).len(), 1);
}

#[test]
fn translation_pending_cap_emits_busy_without_extra_http() {
    let mut h = translation_harness();
    // Keep this assertion tied to the public contract without exposing an implementation
    // constant from the domain crate solely for an integration test.
    for index in 0..32usize {
        h.step(
            Tick::Command(translate_command(&format!("translation-cap-{index}"))),
            100 + index as u64,
        );
        assert!(h
            .sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Http { .. })));
    }
    h.step(
        Tick::Command(translate_command("translation-cap-overflow")),
        200,
    );
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Http { .. })));
    let events = emitted(&h);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["error_code"], "TRANSLATION_BUSY");
}

#[test]
fn translation_identity_switch_cancels_old_generation_without_late_emit() {
    let mut h = translation_harness();
    let (corr, timer) = start_translation(&mut h, "translation-identity");
    let identity = Tick::Command(helix_core::tick::AppCommand::new(
        "__helix_runtime_identity",
        serde_json::to_vec(&json!({
            "auth_user_id": "user-2",
            "company_id": "company-2"
        }))
        .expect("identity patch serializes"),
    ));
    h.step(identity, 101);
    assert!(h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::CancelTimer { id } if *id == timer)));
    h.step(
        port_reply_with_body(
            corr.raw(),
            enveloped(Bytes::from_static(
                br#"{"text":"Late","from":"zh","to":"en"}"#,
            )),
        ),
        102,
    );
    assert!(emitted(&h).is_empty());
}

#[test]
fn translation_stop_drops_pending_reply_without_emitting_after_lifecycle_end() {
    let mut h = translation_harness();
    let (corr, _) = start_translation(&mut h, "translation-stop");
    h.sink.clear();
    h.module
        .on_stop(&mut h.sink)
        .expect("module stop must succeed");
    h.step(
        port_reply_with_body(
            corr.raw(),
            enveloped(Bytes::from_static(
                br#"{"text":"Late","from":"zh","to":"en"}"#,
            )),
        ),
        102,
    );
    assert!(
        emitted(&h).is_empty(),
        "stopped module must not emit old translation replies"
    );
}
