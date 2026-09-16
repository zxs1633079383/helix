//! 回归锚：6aaa06ccba9349dc11743ae6；公开命令、按层分页与取消生命周期。

mod common;

use bytes::Bytes;
use common::{enveloped, Harness};
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::{Correlation, Effect, Module, Tick};
use serde_json::{json, Value};

/// 公开 API 构造命令，保留真实 accepts 与 handle 两道入口。
fn command(h: &mut Harness, name: &str, args: Value) {
    let tick = Tick::Command(
        helix_im::client_api::build_command(name, args.to_string().as_bytes()).unwrap(),
    );
    assert!(h.module.accepts(&tick), "command must be reachable: {name}");
    h.step(tick, 100);
}

/// 读取唯一 HTTP 意图，禁止来源 Get 或隐式本地正文装配。
fn http(h: &Harness) -> (Correlation, Value) {
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Persist { .. } | Effect::PersistAtomic { .. })));
    h.sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { corr, req } => Some((
                *corr,
                serde_json::from_slice(req.body.as_ref().unwrap()).unwrap(),
            )),
            _ => None,
        })
        .expect("HTTP effect")
}

/// 回灌真实 driver 信封，而非手工拼读事件。
fn reply(h: &mut Harness, corr: Correlation, body: Value) {
    h.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(enveloped(Bytes::from(body.to_string())))),
        },
        101,
    );
}

/// 仅解码生产 Effect::Emit。
fn events(h: &Harness) -> Vec<Value> {
    h.sink
        .as_slice()
        .iter()
        .filter_map(|e| match e {
            Effect::Emit { event } => Some(serde_json::from_slice(&event.0).unwrap()),
            _ => None,
        })
        .collect()
}

/// 转发不读取来源行，稳定命令键独立于本轮 request ID。
#[test]
fn relay_sends_only_source_ids_and_stable_command_key() {
    let mut h = Harness::new(&[]);
    command(
        &mut h,
        "im_relay_messages",
        json!({
            "post_ids":[common::sid(1)],"target_channel_ids":[common::cid(2)],
            "mode":"merge","command_id":"stable-1","req_id":"attempt-1"
        }),
    );
    let (_, body) = http(&h);
    assert_eq!(
        body,
        json!({"postIds":[common::sid(1)],"targetChannelIds":[common::cid(2)],"mode":"merged","commandId":"stable-1"})
    );
    assert!(events(&h).is_empty());
}

/// 首层响应必须成为可直接渲染的分页消息，子卡不携带正文树。
#[test]
fn detail_projects_one_page_and_rejects_duplicate_late_reply() {
    let mut h = Harness::new(&[]);
    command(
        &mut h,
        "im_forward_detail",
        json!({"root_post_id":"root","path":[],"offset":0,"req_id":"detail-1"}),
    );
    let (corr, body) = http(&h);
    assert_eq!(body, json!({"rootPostId":"root","path":[],"offset":0}));
    let response = json!({"status":"SUCCESS","data":{
        "rootPostId":"root","path":[],"snapshotId":"snapshot","title":"记录","itemCount":2,"nextOffset":null,
        "items":[
            {"kind":"message","itemKey":"one","userId":"a","userSnapshot":{"userName":"甲"},"createAt":1,"type":"TEXT","message":"完整正文","simpleMessage":"摘要","props":{}},
            {"kind":"forward","itemKey":"two","userId":"b","userSnapshot":{"userName":"乙"},"createAt":2,"snapshotId":"child","title":"子记录","itemCount":100,"preview":[{"author":"丙","text":"首条"}]}
        ]
    }});
    reply(&mut h, corr, response.clone());
    let emitted = events(&h);
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted[0]["event"], "im:read:result");
    let page = &emitted[0]["data"]["body"];
    assert_eq!(page["items"][0]["message"], "完整正文");
    assert_eq!(page["items"][0]["forwardItemKey"], "one");
    assert_eq!(page["items"][1]["type"], "MULTIPLY");
    assert_eq!(page["items"][1]["props"]["forward"]["snapshotId"], "child");
    assert!(page["items"][1].get("forwardDetail").is_none());
    assert!(h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::CancelTimer { .. })));
    reply(&mut h, corr, response);
    assert!(events(&h).is_empty());
}

/// 逻辑取消和超时均应释放读等待，后到 authority 不能复活结果。
#[test]
fn cancelled_or_timed_out_detail_drops_late_reply() {
    for cancel in [true, false] {
        let mut h = Harness::new(&[]);
        command(
            &mut h,
            "im_forward_detail",
            json!({"root_post_id":"root","path":[],"offset":0,"req_id":"detail-1"}),
        );
        let (corr, _) = http(&h);
        let timer = h
            .sink
            .as_slice()
            .iter()
            .find_map(|e| match e {
                Effect::ScheduleTimer { id, .. } => Some(*id),
                _ => None,
            })
            .unwrap();
        if cancel {
            command(
                &mut h,
                "im_cancel_forward_detail",
                json!({"req_id":"detail-1"}),
            );
        } else {
            h.step(Tick::Timer(timer), 40_000);
        }
        assert_eq!(
            events(&h)[0]["data"]["error"],
            if cancel {
                "CANCELLED"
            } else {
                "FORWARD_DETAIL_TIMEOUT"
            }
        );
        reply(&mut h, corr, json!({"status":"SUCCESS","data":{}}));
        assert!(events(&h).is_empty());
    }
}

/// A hundred messages arrive as five non-overlapping pages; media fields survive projection.
#[test]
fn hundred_items_keep_body_media_identity_and_pagination() {
    let mut h = Harness::new(&[]);
    let mut ids = std::collections::BTreeSet::new();
    for offset in (0..100).step_by(20) {
        command(
            &mut h,
            "im_forward_detail",
            json!({"root_post_id":"root","path":["child"],"offset":offset,"req_id":format!("page-{offset}")}),
        );
        let (corr, _) = http(&h);
        let items = (offset..offset+20).map(|i| json!({
            "kind":"message","itemKey":format!("item-{i}"),"userId":"a","userSnapshot":{"userName":"甲"},"createAt":i,
            "type":"RICH","message":"完整正文".repeat(100),"simpleMessage":"摘要",
            "props":{"files":[{"id":format!("file-{i}"),"fileName":"原图.png","url":"https://media.test/stable","width":640,"height":480}],"imagePosition":"bottom"}
        })).collect::<Vec<_>>();
        reply(
            &mut h,
            corr,
            json!({"status":"SUCCESS","data":{"rootPostId":"root","path":["child"],"snapshotId":"snapshot",
            "title":"记录","itemCount":100,"items":items,"nextOffset":if offset == 80 { Value::Null } else { json!(offset+20) }}}),
        );
        let emitted = events(&h);
        let rows = emitted[0]["data"]["body"]["items"].as_array().unwrap();
        assert_eq!(rows.len(), 20);
        for row in rows {
            assert!(ids.insert(row["id"].as_str().unwrap().to_owned()));
            assert_eq!(row["message"], "完整正文".repeat(100));
            assert_eq!(row["props"]["files"][0]["name"], "原图.png");
            assert_eq!(row["props"]["imagePosition"], "bottom");
        }
    }
    assert_eq!(ids.len(), 100);
    command(
        &mut h,
        "im_forward_detail",
        json!({"root_post_id":"root","path":["child"],"offset":100,"req_id":"end"}),
    );
    let (corr, _) = http(&h);
    reply(
        &mut h,
        corr,
        json!({"status":"SUCCESS","data":{"rootPostId":"root","path":["child"],"snapshotId":"snapshot","title":"记录","itemCount":100,"items":[],"nextOffset":null}}),
    );
    assert_eq!(events(&h)[0]["data"]["body"]["items"], json!([]));
}

/// Invalid boundaries, wrong roots and recursive replies never become renderable success.
#[test]
fn detail_rejects_corrupt_pages_and_releases_request() {
    let good = json!({"rootPostId":"root","path":[],"snapshotId":"s","title":"记录","itemCount":1,"nextOffset":null,
        "items":[{"kind":"message","itemKey":"one","userId":"a","userSnapshot":{},"createAt":1,"type":"TEXT","message":"ok","props":{}}]});
    let mut cases = vec![
        json!({"status":"FAILED","data":null}),
        json!({"status":"SUCCESS","data":{}}),
    ];
    for (key, value) in [
        ("rootPostId", json!("wrong")),
        ("path", json!(["wrong"])),
        ("nextOffset", json!(1)),
        ("items", json!([])),
    ] {
        let mut page = good.clone();
        page[key] = value;
        cases.push(json!({"status":"SUCCESS","data":page}));
    }
    let mut recursive = good.clone();
    recursive["items"][0]["props"] = json!({"mergeMessage":[]});
    cases.push(json!({"status":"SUCCESS","data":recursive}));
    let mut undeclared = good.clone();
    undeclared["items"][0]["viewers"] = json!(["private-viewer"]);
    cases.push(json!({"status":"SUCCESS","data":undeclared}));
    let mut page_budget = good.clone();
    let items = (0..6)
        .map(|i| {
            let mut item = good["items"][0].clone();
            item["itemKey"] = json!(format!("{i}"));
            item["message"] = json!("x".repeat(190_000));
            item
        })
        .collect::<Vec<_>>();
    page_budget["items"] = json!(items);
    page_budget["itemCount"] = json!(6);
    cases.push(json!({"status":"SUCCESS","data":page_budget}));
    let mut oversized = good.clone();
    oversized["items"][0]["message"] = json!("x".repeat(256 * 1024));
    cases.push(json!({"status":"SUCCESS","data":oversized}));
    for body in cases {
        let mut h = Harness::new(&[]);
        command(
            &mut h,
            "im_forward_detail",
            json!({"root_post_id":"root","path":[],"offset":0,"req_id":"r"}),
        );
        let (corr, _) = http(&h);
        reply(&mut h, corr, body);
        assert!(events(&h)[0]["data"]["error"].is_string());
        assert!(events(&h)[0]["data"].get("body").is_none());
        command(&mut h, "im_cancel_forward_detail", json!({"req_id":"r"}));
        assert!(events(&h).is_empty());
    }
}

/// A 32-layer path is bounded and request capacity is released on explicit cancellation.
#[test]
fn detail_depth_capacity_and_conflicting_request_are_fail_closed() {
    let mut h = Harness::new(&[]);
    for (depth, accepted) in [(0, true), (7, true), (17, true), (31, true), (32, false)] {
        let args =
            json!({"root_post_id":"root","path":vec!["child";depth],"offset":0,"req_id":"depth"});
        command(&mut h, "im_forward_detail", args);
        let has_http = h
            .sink
            .as_slice()
            .iter()
            .any(|e| matches!(e, Effect::Http { .. }));
        assert_eq!(has_http, accepted);
        if !accepted {
            assert!(events(&h)[0]["data"]["error"].is_string());
        }
        command(
            &mut h,
            "im_cancel_forward_detail",
            json!({"req_id":"depth"}),
        );
    }
    for i in 0..32 {
        command(
            &mut h,
            "im_forward_detail",
            json!({"root_post_id":"root","path":[],"offset":0,"req_id":format!("r-{i}")}),
        );
        http(&h);
    }
    let overflow = json!({"root_post_id":"root","path":[],"offset":0,"req_id":"overflow"});
    command(&mut h, "im_forward_detail", overflow.clone());
    assert!(events(&h)[0]["data"]["error"]
        .as_str()
        .unwrap()
        .contains("FORWARD_DETAIL_BUSY"));
    command(&mut h, "im_cancel_forward_detail", json!({"req_id":"r-0"}));
    command(&mut h, "im_forward_detail", overflow.clone());
    let (corr, _) = http(&h);
    let mut conflict = overflow;
    conflict["offset"] = json!(1);
    command(&mut h, "im_forward_detail", conflict);
    assert_eq!(
        events(&h)[0]["data"]["error"],
        "FORWARD_DETAIL_REQUEST_CONFLICT"
    );
    reply(&mut h, corr, json!({"status":"SUCCESS","data":{}}));
    assert!(events(&h).is_empty());
    h.sink.clear();
    h.module.on_stop(&mut h.sink).unwrap();
    assert_eq!(
        events(&h)
            .iter()
            .filter(|e| e["data"]["error"] == "CANCELLED")
            .count(),
        31
    );
}

/// A malformed committed response cannot mark fewer posts than the requested target as complete.
#[test]
fn committed_retry_rejects_incomplete_identity_set_and_can_retry() {
    let mut h = Harness::new(&[]);
    let mut args = json!({"post_ids":[common::sid(1),common::sid(2)],"target_channel_ids":[common::cid(3)],"mode":"item","command_id":"stable","req_id":"first"});
    command(&mut h, "im_relay_messages", args.clone());
    let (corr, _) = http(&h);
    reply(
        &mut h,
        corr,
        json!({"status":"SUCCESS","data":{"targets":[{"channelId":common::cid(3),"status":"committed","postIds":[common::sid(10)],"temporaryIds":["t10"]}]}}),
    );
    assert_eq!(events(&h)[0]["data"]["batchStatus"], "failed");
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Http { .. })));
    args["req_id"] = json!("retry");
    command(&mut h, "im_relay_messages", args);
    http(&h);
}

/// Transport failure, identity replacement and duplicate commands leave no live result route.
#[test]
fn detail_transport_identity_and_duplicates_are_closed() {
    let mut h = Harness::new(&[]);
    let args = json!({"root_post_id":"root","path":[],"offset":0,"req_id":"r"});
    command(&mut h, "im_forward_detail", args.clone());
    let (corr, _) = http(&h);
    command(&mut h, "im_forward_detail", args);
    assert!(h.sink.as_slice().is_empty());
    h.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Err(helix_core::tick::PortError::Network),
        },
        102,
    );
    assert!(events(&h)[0]["data"]["error"]
        .as_str()
        .unwrap()
        .contains("HTTP_FAILED"));
    command(
        &mut h,
        "im_forward_detail",
        json!({"root_post_id":"root","path":[],"offset":0,"req_id":"next"}),
    );
    let (corr, _) = http(&h);
    h.sink.clear();
    h.module.on_stop(&mut h.sink).unwrap();
    assert!(events(&h).iter().any(|e| e["data"]["error"] == "CANCELLED"));
    reply(&mut h, corr, json!({"status":"SUCCESS","data":{}}));
    assert!(events(&h).is_empty());
}

/// Stable duplicate commands cannot replace the first wait; timeout permits an explicit retry.
#[test]
fn duplicate_forward_attempt_and_timeout_keep_stable_command() {
    let mut h = Harness::new(&[]);
    let mut args = json!({"post_ids":[common::sid(1)],"target_channel_ids":[common::cid(2)],"mode":"merge","command_id":"stable","req_id":"first"});
    command(&mut h, "im_relay_messages", args.clone());
    let timer = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::ScheduleTimer { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    let (corr, _) = http(&h);
    args["req_id"] = json!("second");
    command(&mut h, "im_relay_messages", args.clone());
    assert!(events(&h)[0]["data"]["targets"][0]["error"]
        .as_str()
        .unwrap()
        .contains("FORWARD_IN_PROGRESS"));
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Http { .. })));
    h.step(Tick::Timer(timer), 40_000);
    assert_eq!(events(&h)[0]["data"]["reqId"], "first");
    reply(
        &mut h,
        corr,
        json!({"status":"SUCCESS","data":{"targets":[]}}),
    );
    assert!(events(&h).is_empty());
    command(&mut h, "im_relay_messages", args);
    assert_eq!(http(&h).1["commandId"], "stable");
}

/// A committed retry still requires every requested post to cross Persist and durable readback.
#[test]
fn committed_retry_requires_durable_rows_and_cleans_persist_failure() {
    for fail_persist in [false, true] {
        let mut h = Harness::new_with_identity(&[], "sender", "company", "Sender", "", "");
        command(
            &mut h,
            "im_relay_messages",
            json!({"post_ids":[common::sid(1),common::sid(2)],"target_channel_ids":[common::cid(3)],"mode":"item","command_id":"stable","req_id":"retry"}),
        );
        let (corr, _) = http(&h);
        reply(
            &mut h,
            corr,
            json!({"status":"SUCCESS","data":{"targets":[{"channelId":common::cid(3),"status":"committed","postIds":[common::sid(10),common::sid(11)],"temporaryIds":["t10","t11"]}]}}),
        );
        assert!(!events(&h).iter().any(|e| e["event"] == "im:post:received"));
        let (corr, body) = http(&h);
        assert_eq!(body["postIds"], json!([common::sid(10), common::sid(11)]));
        let posts=(10..12).map(|i| json!({"id":common::sid(i),"temporaryId":format!("t{i}"),"channelId":common::cid(3),"userId":"sender","type":"TEXT","message":"http","viewers":["all"],"props":{}})).collect::<Vec<_>>();
        reply(&mut h, corr, json!({"status":"SUCCESS","data":posts}));
        let corr = h
            .sink
            .as_slice()
            .iter()
            .find_map(|e| match e {
                Effect::Persist { corr, .. } => Some(*corr),
                _ => None,
            })
            .expect("durable persist");
        assert!(events(&h).is_empty());
        h.step(
            Tick::PortReply {
                corr,
                outcome: if fail_persist {
                    PortOutcome::Err(helix_core::tick::PortError::Storage(1))
                } else {
                    PortOutcome::Ok(ReplyBytes::default())
                },
            },
            103,
        );
        if fail_persist {
            assert_eq!(events(&h)[0]["data"]["batchStatus"], "failed");
            assert!(h
                .sink
                .as_slice()
                .iter()
                .any(|e| matches!(e, Effect::CancelTimer { .. })));
            continue;
        }
        for i in 10..12 {
            let corr = h
                .sink
                .as_slice()
                .iter()
                .find_map(|e| match e {
                    Effect::Persist { corr, .. } => Some(*corr),
                    _ => None,
                })
                .unwrap();
            let row = json!([{"id":common::sid(i),"temporary_id":format!("t{i}"),"channel_id":common::cid(3),"user_id":"sender","type":"TEXT","message":"durable","viewers":"[\"all\"]","props":"{}"}]);
            h.step(
                Tick::PortReply {
                    corr,
                    outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(row.to_string()))),
                },
                104 + i,
            );
            if i == 10 {
                assert!(events(&h).is_empty());
            }
        }
        let emitted = events(&h);
        assert_eq!(emitted.len(), 2);
        assert_eq!(emitted[0]["data"]["message"], "durable");
        assert!(emitted[0]["data"].get("requestId").is_none());
        assert_eq!(emitted[1]["data"]["requestId"], "retry");
        assert!(h
            .sink
            .as_slice()
            .iter()
            .any(|e| matches!(e, Effect::CancelTimer { .. })));
    }
}
