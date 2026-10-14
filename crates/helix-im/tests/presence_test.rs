#![allow(dead_code)]
mod common;
use helix_core::effect::{TimerId, TransportId};
use helix_core::tick::{AppCommand, InboundBytes, PortError, PortOutcome, ReplyBytes};
use helix_core::{Correlation, Effect, EffectSink, Module, Tick};
use helix_im::{module::ImConfig, ImModule};
use serde_json::{json, Value};

// Every test drives the public module seam; HTTP replies use the driver's real envelope format.
fn module() -> ImModule {
    ImModule::new(ImConfig {
        api_base_url: "http://test/api/cses".into(),
        ..ImConfig::default()
    })
}
// Capture one deterministic step.
fn step(m: &mut ImModule, tick: Tick, now: u64) -> Vec<Effect> {
    let mut sink = EffectSink::new();
    m.handle(&tick, now, &mut sink).unwrap();
    {
        let mut effects = Vec::new();
        sink.visit_mut_from(0, |effect| {
            effects.push(std::mem::replace(
                effect,
                Effect::CancelTimer {
                    id: TimerId::from_raw(0),
                },
            ))
        });
        effects
    }
}
// Scope replacement is the only UI command.
fn observe(
    m: &mut ImModule,
    scope: &str,
    users: Vec<String>,
    focus: Vec<String>,
    now: u64,
) -> Vec<Effect> {
    let tick = Tick::Command(AppCommand::new(
        "im_presence_observe",
        json!({"scope_id":scope,"user_ids":users,"focused_user_ids":focus}).to_string(),
    ));
    assert!(m.accepts(&tick));
    step(m, tick, now)
}
// Find only presence events so unrelated hello effects cannot satisfy assertions.
fn statuses(effects: &[Effect]) -> Value {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Emit { event } => serde_json::from_slice::<Value>(&event.0).ok(),
            _ => None,
        })
        .filter(|v| v["event"] == "im:presence:update")
        .last()
        .map(|v| v["data"]["statuses"].clone())
        .unwrap_or(Value::Null)
}
// Retrieve the query correlation assigned by production code.
fn corr(e: &[Effect]) -> Correlation {
    e.iter()
        .find_map(|e| match e {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .unwrap()
}
// Retrieve the presence timer from a scope command.
fn timer(e: &[Effect]) -> TimerId {
    e.iter()
        .find_map(|e| match e {
            Effect::ScheduleTimer { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap()
}
// Feed a fully enveloped remote observation.
fn reply(m: &mut ImModule, c: Correlation, data: Value, now: u64) -> Vec<Effect> {
    let data = if data.get("status").is_some_and(|s| s != "SUCCESS") {
        data
    } else {
        let statuses = data.get("data").cloned().unwrap_or(data);
        json!({"status":"SUCCESS","data":{"statuses":statuses,"checkedAt":now}})
    };
    raw_reply(m, c, data, now)
}
// Deliver metadata with an explicit server sampling time to exercise cross-transport ordering.
fn raw_reply(m: &mut ImModule, c: Correlation, data: Value, now: u64) -> Vec<Effect> {
    step(
        m,
        Tick::PortReply {
            corr: c,
            outcome: PortOutcome::Ok(ReplyBytes(common::enveloped(data.to_string().into()))),
        },
        now,
    )
}
// A handshake establishes the connection before watch frames may be sent.
fn hello(m: &mut ImModule, id: &str, now: u64) -> Vec<Effect> {
    inbound(m, json!({"action":"hello","data":{"connectionId":id}}), now)
}
// Raw server frames pass the same parser as native/FFI/web traffic.
fn inbound(m: &mut ImModule, v: Value, now: u64) -> Vec<Effect> {
    step(m, Tick::Inbound(InboundBytes(v.to_string().into())), now)
}
// Extract watch generation from actual outbound frame.
fn generation(e: &[Effect]) -> u64 {
    e.iter()
        .filter_map(|e| match e {
            Effect::Send { frame, .. } => serde_json::from_slice::<Value>(frame).ok(),
            _ => None,
        })
        .find(|v| v["action"] == "presence_watch")
        .unwrap()["data"]["generation"]
        .as_u64()
        .unwrap()
}

// Missing/duplicate replies must neither extend freshness nor generate successful observations.
#[test]
fn query_success_failure_timeout_and_release() {
    let mut m = module();
    let effects = observe(&mut m, "list", vec!["u1".into(), "u1".into()], vec![], 100);
    assert_eq!(statuses(&effects)["u1"], "unknown");
    let c = corr(&effects);
    let t = timer(&effects);
    assert_eq!(
        statuses(&reply(
            &mut m,
            c,
            json!({"status":"SUCCESS","data":{"u1":"online"}}),
            200
        ))["u1"],
        "online"
    );
    assert!(statuses(&raw_reply(
        &mut m,
        c,
        json!({"data":{"statuses":{"u1":"offline"},"checkedAt":100}}),
        300
    ))
    .is_null());
    let effects = step(&mut m, Tick::Timer(t), 30_100);
    let c = corr(&effects);
    let t = timer(&effects);
    let effects = step(
        &mut m,
        Tick::PortReply {
            corr: c,
            outcome: PortOutcome::Err(PortError::Network),
        },
        30_200,
    );
    assert!(statuses(&effects).is_null());
    let effects = step(&mut m, Tick::Timer(t), 45_200);
    assert_eq!(statuses(&effects)["u1"], "unknown");
    let effects = observe(&mut m, "list", vec![], vec![], 45_300);
    assert_eq!(statuses(&effects), json!({}));
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::CancelTimer { .. })));
}

// Focus snapshots are generation-scoped, fully replace authorization results and expire independently.
#[test]
fn focus_push_revoke_expiry_disconnect_and_reconnect() {
    let mut m = module();
    hello(&mut m, "c1", 0);
    let effects = observe(&mut m, "card", vec![], vec!["u1".into()], 100);
    let g = generation(&effects);
    let c = corr(&effects);
    let t = timer(&effects);
    reply(&mut m, c, json!({"u1":"offline"}), 100);
    assert!(effects.iter().any(|e| matches!(e, Effect::Http { .. })));
    let push = |g, checked, statuses| json!({"action":"presence","data":{"generation":g,"checkedAt":checked,"statuses":statuses}});
    assert_eq!(
        statuses(&inbound(&mut m, push(g, 200, json!({"u1":"online"})), 200))["u1"],
        "online"
    );
    assert!(statuses(&inbound(&mut m, push(g, 199, json!({"u1":"offline"})), 300)).is_null());
    assert_eq!(
        statuses(&inbound(&mut m, push(g, 400, json!({})), 400))["u1"],
        "unknown"
    );
    inbound(&mut m, push(g, 500, json!({"u1":"online"})), 500);
    assert_eq!(
        statuses(&step(&mut m, Tick::Timer(t), 15_500))["u1"],
        "unknown"
    );
    step(&mut m, Tick::Disconnected(TransportId::from_raw(0)), 16_000);
    assert!(statuses(&inbound(
        &mut m,
        push(g, 2000, json!({"u1":"online"})),
        16_100
    ))
    .is_null());
    let effects = hello(&mut m, "c2", 17_000);
    assert!(generation(&effects) > g);
}

// Shared scopes retain users until the final observer releases; aggregate IO remains bounded.
#[test]
fn reference_counts_caps_and_overflow_unknown() {
    let mut m = module();
    let ids: Vec<_> = (0..200).map(|i| format!("u{i:03}")).collect();
    let effects = observe(&mut m, "a", ids, vec![], 0);
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::Http { .. }))
            .count(),
        2
    );
    let effects = observe(&mut m, "b", vec!["z".into()], vec![], 1);
    assert_eq!(statuses(&effects)["z"], "unknown");
    let effects = observe(&mut m, "a", vec![], vec![], 2);
    assert_eq!(statuses(&effects), json!({"z":"unknown"}));
    let c = corr(&effects);
    reply(&mut m, c, json!({"z":"offline"}), 3);
    let effects = observe(&mut m, "c", vec!["z".into()], vec![], 4);
    assert_eq!(statuses(&effects)["z"], "offline");
    let effects = observe(&mut m, "b", vec![], vec![], 5);
    assert_eq!(statuses(&effects)["z"], "offline");
    assert!(!effects.iter().any(|e| matches!(e, Effect::Http { .. })));
}

// Validation is atomic and rejected input never starts HTTP or timers.
#[test]
fn invalid_input_fails_closed() {
    for payload in [
        json!({}),
        json!({"scope_id":"","user_ids":[],"focused_user_ids":[]}),
        json!({"scope_id":"x","user_ids":[""],"focused_user_ids":[]}),
    ] {
        let mut m = module();
        let mut sink = EffectSink::new();
        assert!(m
            .handle(
                &Tick::Command(AppCommand::new("im_presence_observe", payload.to_string())),
                0,
                &mut sink
            )
            .is_err());
        assert!(!sink.as_slice().iter().any(|e| matches!(
            e,
            Effect::Http { .. } | Effect::Send { .. } | Effect::ScheduleTimer { .. }
        )));
    }
}

// An old request cannot repopulate a replaced view; failure-shaped payloads never imply offline.
#[test]
fn late_query_and_failed_business_reply_are_not_applied() {
    let mut m = module();
    let effects = observe(&mut m, "a", vec!["u1".into()], vec![], 0);
    let old = corr(&effects);
    let effects = observe(&mut m, "a", vec!["u2".into()], vec![], 1);
    let current = corr(&effects);
    assert!(statuses(&reply(&mut m, old, json!({"u1":"online"}), 2)).is_null());
    assert!(statuses(&reply(
        &mut m,
        current,
        json!({"status":"DENIED","data":{"u2":"offline"}}),
        3
    ))
    .is_null());
    let effects = observe(&mut m, "a", vec!["u2".into()], vec![], 4);
    assert_eq!(statuses(&effects)["u2"], "unknown");
}

// Timeouts, rejected HTTP status and identity transitions cannot resurrect cached users.
#[test]
fn query_timeout_and_identity_change_drop_late_results() {
    let mut m = module();
    let e = observe(&mut m, "a", vec!["u1".into()], vec![], 0);
    let c = corr(&e);
    let t = timer(&e);
    step(&mut m, Tick::Timer(t), 15_000);
    assert!(statuses(&reply(&mut m, c, json!({"u1":"online"}), 15_001)).is_null());
    let e = observe(&mut m, "a", vec!["u2".into()], vec![], 15_002);
    let c = corr(&e);
    reply(&mut m, c, json!({"u2":"online"}), 15_002);
    let identity = Tick::Command(AppCommand::new(
        "__helix_runtime_identity",
        json!({"auth_user_id":"new-user","company_id":"new-company"}).to_string(),
    ));
    let e = step(&mut m, identity, 15_003);
    assert_eq!(statuses(&e)["u2"], "unknown");
    assert!(statuses(&reply(&mut m, c, json!({"u2":"online"}), 15_004)).is_null());
    let e = observe(&mut m, "b", vec!["u2".into()], vec![], 15_005);
    let c = corr(&e);
    let bad = Tick::PortReply {
        corr: c,
        outcome: PortOutcome::Ok(ReplyBytes(
            json!({"status":403,"body":"e30="}).to_string().into(),
        )),
    };
    assert!(statuses(&step(&mut m, bad, 15_006)).is_null());
}

// Only ten focused users are subscribed; overflow retains the ordinary bounded query path.
#[test]
fn focus_overflow_queries_and_watch_renews() {
    let mut m = module();
    hello(&mut m, "c1", 0);
    let e = observe(
        &mut m,
        "a",
        vec![],
        (0..11).map(|i| format!("u{i:02}")).collect(),
        1,
    );
    let g = generation(&e);
    let t = timer(&e);
    assert_eq!(
        e.iter()
            .filter(|e| matches!(e, Effect::Http { .. }))
            .count(),
        1
    );
    let e = step(&mut m, Tick::Timer(t), 20_001);
    assert_eq!(generation(&e), g);
    let e = observe(&mut m, "a", vec![], vec![], 20_002);
    assert!(generation(&e) > g);
    assert!(!e.iter().any(|e| matches!(e, Effect::ScheduleTimer { .. })));
}

// All platforms discover the identical command/event through the shared public ABI.
#[test]
fn public_api_advertises_presence() {
    let caps: Value = serde_json::from_slice(&helix_im::client_api::capabilities_json()).unwrap();
    assert!(caps["commands"]
        .as_array()
        .unwrap()
        .contains(&json!("im_presence_observe")));
    assert!(caps["events"]
        .as_array()
        .unwrap()
        .contains(&json!("im:presence:update")));
    assert!(helix_im::client_api::command_alias("im_presence_observe").is_some());
}

// A delayed initial HTTP snapshot cannot overwrite a fresher focused WS sample.
#[test]
fn initial_focus_http_is_immediate_but_push_wins() {
    let mut m = module();
    hello(&mut m, "c1", 0);
    let calibration = observe(&mut m, "clock", vec!["clock".into()], vec![], 0);
    reply(&mut m, corr(&calibration), json!({"clock":"offline"}), 0);
    observe(&mut m, "clock", vec![], vec![], 1);
    let e = observe(&mut m, "card", vec![], vec!["u1".into()], 100);
    let c = corr(&e);
    let g = generation(&e);
    inbound(
        &mut m,
        json!({"action":"presence","data":{"generation":g,"checkedAt":200,"statuses":{"u1":"online"}}}),
        200,
    );
    assert!(statuses(&raw_reply(
        &mut m,
        c,
        json!({"data":{"statuses":{"u1":"offline"},"checkedAt":100}}),
        300
    ))
    .is_null());
    let e = observe(&mut m, "card", vec![], vec!["u1".into()], 301);
    assert_eq!(statuses(&e)["u1"], "online");
}

// The freshness budget starts when the sample is requested, not when a slow network delivers it.
#[test]
fn delayed_query_does_not_extend_freshness_budget() {
    let mut m = module();
    let e = observe(&mut m, "list", vec!["u1".into()], vec![], 100);
    let c = corr(&e);
    let t = timer(&e);
    assert_eq!(
        statuses(&reply(&mut m, c, json!({"u1":"online"}), 14_000))["u1"],
        "online"
    );
    assert_eq!(
        statuses(&step(&mut m, Tick::Timer(t), 45_100))["u1"],
        "unknown"
    );
}

// Unused presence must not introduce unrelated events or background work into other domains.
#[test]
fn no_observers_leave_handshake_and_identity_presence_silent() {
    let mut m = module();
    let e = hello(&mut m, "c1", 0);
    assert!(statuses(&e).is_null());
    assert!(!e.iter().any(|e| matches!(e, Effect::Send { .. })));
    let e = step(
        &mut m,
        Tick::Command(AppCommand::new(
            "__helix_runtime_identity",
            json!({"auth_user_id":"next"}).to_string(),
        )),
        1,
    );
    assert!(statuses(&e).is_null());
    assert!(!e.iter().any(|e| matches!(e, Effect::Send { .. })));
}

// Scrolling one row while HTTP is slow keeps the 49-user intersection in flight.
#[test]
fn scrolling_one_of_fifty_only_queries_the_new_user() {
    let mut m = module();
    let ids: Vec<_> = (0..50).map(|i| format!("u{i:02}")).collect();
    let first = observe(&mut m, "list", ids.clone(), vec![], 0);
    let c = corr(&first);
    let t = timer(&first);
    let ids: Vec<_> = (1..51).map(|i| format!("u{i:02}")).collect();
    let second = observe(&mut m, "list", ids.clone(), vec![], 200);
    let request = second
        .iter()
        .find_map(|e| match e {
            Effect::Http { req, .. } => req
                .body
                .as_ref()
                .and_then(|body| serde_json::from_slice::<Value>(body).ok()),
            _ => None,
        })
        .unwrap();
    assert_eq!(request["userIds"], json!(["u50"]));
    let data: serde_json::Map<_, _> = (0..50)
        .map(|i| (format!("u{i:02}"), json!("online")))
        .collect();
    let e = reply(&mut m, c, Value::Object(data), 500);
    assert_eq!(statuses(&e)["u01"], "online");
    assert!(statuses(&e).get("u00").is_none());
    let mut ids = ids;
    ids.remove(0);
    ids.push("u51".into());
    let third = observe(&mut m, "list", ids, vec![], 600);
    let request = third
        .iter()
        .find_map(|e| match e {
            Effect::Http { req, .. } => req
                .body
                .as_ref()
                .and_then(|body| serde_json::from_slice::<Value>(body).ok()),
            _ => None,
        })
        .unwrap();
    assert_eq!(request["userIds"], json!(["u51"]));
    let e = step(&mut m, Tick::Timer(t), 30_000);
    assert!(
        e.iter().any(|e| matches!(e, Effect::Http { .. })),
        "scroll must not postpone the original refresh deadline"
    );
}

// Removed-and-readded users are isolated from an earlier batch even if some batch peers remain observed.
#[test]
fn removed_then_readded_user_does_not_accept_old_batch_result() {
    let mut m = module();
    let e = observe(&mut m, "list", vec!["a".into(), "b".into()], vec![], 0);
    let old = corr(&e);
    observe(&mut m, "list", vec!["b".into()], vec![], 100);
    let e = observe(&mut m, "list", vec!["a".into(), "b".into()], vec![], 200);
    let new = corr(&e);
    let e = reply(&mut m, old, json!({"a":"online","b":"online"}), 300);
    assert_eq!(statuses(&e)["a"], "unknown");
    assert_eq!(statuses(&e)["b"], "online");
    let e = reply(&mut m, new, json!({"a":"offline"}), 400);
    assert_eq!(statuses(&e)["a"], "offline");
}

// Server timestamps order both transports; delayed WS cannot resurrect expired samples despite clock skew.
#[test]
fn late_ws_cannot_overwrite_new_http_or_extend_sample_lifetime() {
    let mut m = module();
    hello(&mut m, "c1", 0);
    let e = observe(&mut m, "card", vec![], vec!["u1".into()], 100);
    let c = corr(&e);
    let g = generation(&e);
    let t = timer(&e);
    raw_reply(
        &mut m,
        c,
        json!({"data":{"statuses":{"u1":"online"},"checkedAt":1_000_100}}),
        110,
    );
    let push = |checked, state| json!({"action":"presence","data":{"generation":g,"checkedAt":checked,"statuses":{"u1":state}}});
    assert!(statuses(&inbound(&mut m, push(1_000_050, "offline"), 300)).is_null());
    assert_eq!(
        statuses(&step(&mut m, Tick::Timer(t), 15_100))["u1"],
        "unknown"
    );
    assert!(statuses(&inbound(&mut m, push(1_000_120, "online"), 16_000)).is_null());
    assert_eq!(
        statuses(&inbound(&mut m, push(1_016_010, "online"), 16_020))["u1"],
        "online"
    );
}

// Failed samples are retried at the bounded refresh cadence, not once per scroll event.
#[test]
fn failed_query_is_not_retried_for_every_scroll() {
    let mut m = module();
    let e = observe(&mut m, "list", vec!["a".into()], vec![], 0);
    let c = corr(&e);
    step(
        &mut m,
        Tick::PortReply {
            corr: c,
            outcome: PortOutcome::Err(PortError::Network),
        },
        100,
    );
    let e = observe(&mut m, "list", vec!["a".into(), "b".into()], vec![], 200);
    let request = e
        .iter()
        .find_map(|e| match e {
            Effect::Http { req, .. } => req
                .body
                .as_ref()
                .and_then(|b| serde_json::from_slice::<Value>(b).ok()),
            _ => None,
        })
        .unwrap();
    assert_eq!(request["userIds"], json!(["b"]));
    assert_eq!(request["includeMetadata"], true);
}

// Ordinary list scrolling never changes a focused subscription's generation or starves its in-flight sample.
#[test]
fn ordinary_scroll_keeps_focused_subscription_generation() {
    let mut m = module();
    hello(&mut m, "c1", 0);
    let e = observe(&mut m, "card", vec![], vec!["friend".into()], 100);
    let g = generation(&e);
    reply(&mut m, corr(&e), json!({"friend":"offline"}), 100);
    for i in 0..4 {
        let e = observe(
            &mut m,
            "list",
            vec![format!("row-{i}")],
            vec![],
            200 + i * 200,
        );
        assert!(
            !e.iter().any(|e| matches!(e, Effect::Send { .. })),
            "ordinary scroll must not renew or replace watch"
        );
    }
    let e = inbound(
        &mut m,
        json!({"action":"presence","data":{"generation":g,"checkedAt":1000,"statuses":{"friend":"online"}}}),
        1000,
    );
    assert_eq!(statuses(&e)["friend"], "online");
}

// A server clock jump cannot extend the local focused freshness budget beyond fifteen seconds.
#[test]
fn server_clock_jump_is_capped_by_local_freshness() {
    let mut m = module();
    hello(&mut m, "c1", 0);
    let e = observe(&mut m, "card", vec![], vec!["u1".into()], 100);
    let g = generation(&e);
    let t = timer(&e);
    reply(&mut m, corr(&e), json!({"u1":"offline"}), 100);
    inbound(
        &mut m,
        json!({"action":"presence","data":{"generation":g,"checkedAt":9_000_000,"statuses":{"u1":"online"}}}),
        200,
    );
    assert_eq!(
        statuses(&step(&mut m, Tick::Timer(t), 15_200))["u1"],
        "unknown"
    );
}

// Opening/closing a focused surface must not turn an unchanged valid sample into unknown.
#[test]
fn focus_switch_preserves_status_while_refreshing() {
    for initial in ["online", "offline"] {
        let mut m = module();
        hello(&mut m, "c1", 0);
        let list = observe(&mut m, "list", vec!["u1".into()], vec![], 100);
        reply(&mut m, corr(&list), json!({"u1":initial}), 200);
        let focused = observe(&mut m, "card", vec![], vec!["u1".into()], 300);
        assert_eq!(
            statuses(&focused)["u1"],
            initial,
            "promotion must preserve the visible status"
        );
        let refresh = corr(&focused);
        assert!(statuses(&reply(&mut m, refresh, json!({"u1":initial}), 400)).is_null());
        let ordinary = observe(&mut m, "card", vec![], vec![], 500);
        assert_eq!(
            statuses(&ordinary)["u1"],
            initial,
            "demotion must preserve the visible status"
        );
        assert!(statuses(&reply(&mut m, corr(&ordinary), json!({"u1":initial}), 600)).is_null());
        let focused = observe(&mut m, "card", vec![], vec!["u1".into()], 700);
        assert_eq!(statuses(&focused)["u1"], initial);
        assert_eq!(
            statuses(&reply(&mut m, corr(&focused), json!({"u1":"offline"}), 800))["u1"],
            if initial == "online" {
                json!("offline")
            } else {
                Value::Null
            }
        );
        let g = generation(&focused);
        let revoked = inbound(
            &mut m,
            json!({"action":"presence","data":{"generation":g,"checkedAt":900,"statuses":{}}}),
            900,
        );
        assert_eq!(
            statuses(&revoked)["u1"],
            "unknown",
            "authoritative omission still revokes the sample"
        );
    }
}

// Mode changes and failed refreshes cannot keep a stale green dot alive indefinitely.
#[test]
fn focus_switch_failed_refresh_keeps_but_never_extends_expiry() {
    for focus_at in [20_100, 44_100, 46_100] {
        let mut m = module();
        let list = observe(&mut m, "list", vec!["u1".into()], vec![], 100);
        let t = timer(&list);
        reply(&mut m, corr(&list), json!({"u1":"online"}), 200);
        let focused = observe(&mut m, "card", vec![], vec!["u1".into()], focus_at);
        let expected = if focus_at < 45_100 {
            "online"
        } else {
            "unknown"
        };
        assert_eq!(statuses(&focused)["u1"], expected);
        let failed = step(
            &mut m,
            Tick::PortReply {
                corr: corr(&focused),
                outcome: PortOutcome::Err(PortError::Network),
            },
            focus_at + 1,
        );
        assert!(statuses(&failed).is_null());
        if expected == "unknown" {
            continue;
        }
        let demoted = observe(&mut m, "card", vec![], vec![], focus_at + 2);
        assert_eq!(statuses(&demoted)["u1"], "online");
        step(
            &mut m,
            Tick::PortReply {
                corr: corr(&demoted),
                outcome: PortOutcome::Err(PortError::Network),
            },
            focus_at + 3,
        );
        let promoted = observe(&mut m, "card", vec![], vec!["u1".into()], focus_at + 4);
        assert_eq!(statuses(&promoted)["u1"], "online");
        let expires = (focus_at + 15_000).min(45_100);
        let before = step(&mut m, Tick::Timer(t), expires - 1);
        assert!(statuses(&before).is_null());
        let expired = step(&mut m, Tick::Timer(timer(&before)), expires);
        assert_eq!(statuses(&expired)["u1"], "unknown");
    }
}
