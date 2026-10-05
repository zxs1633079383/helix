//! 用户级持久化不等于当前公司可渲染；验证真实 PersistOk → Emit 接缝。
mod common;
use bytes::Bytes;
use helix_core::effect::{SqlValue, StorageOp, UpsertSpec};
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::{Effect, EffectSink, Module, Tick};
use helix_im::{event::MessageV3Event, ImConfig, ImModule};
use serde_json::{json, Value};

const CHANNEL: &str = "j519p4hif7di7xibc9p6a1189r";

/// 通过公共提交入口冻结真实频道归属，完成持久回执后检查渲染结果。
fn committed_events(company: &str, event: &'static str, data: Value) -> Vec<Value> {
    committed_events_for("company-a", "viewer", company, event, data)
}

/// 按真实 RuntimeAuth 和持久 channel owner 回放相同的 PersistOk 屏障。
fn committed_events_for(
    auth_company: &str,
    stored_user: &str,
    company: &str,
    event: &'static str,
    data: Value,
) -> Vec<Value> {
    let mut module = ImModule::new(ImConfig {
        auth_user_id: "viewer".into(),
        company_id: auth_company.into(),
        ..Default::default()
    });
    let mut sink = EffectSink::new();
    let op = StorageOp::BatchUpsert(UpsertSpec::new(
        "channel",
        vec![vec![
            ("id".into(), SqlValue::Text(CHANNEL.into())),
            ("team_id".into(), SqlValue::Text(company.into())),
            ("user_id".into(), SqlValue::Text(stored_user.into())),
        ]],
        Some("id"),
    ));
    let corr = module.queue_message_v3_commit(
        vec![op],
        vec![MessageV3Event::new(event, data).unwrap()],
        &mut sink,
    );
    assert!(sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::PersistAtomic { .. })));
    assert!(!sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Emit { .. })));
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"[]"))),
            },
            1,
            &mut sink,
        )
        .unwrap();
    sink.as_slice()
        .iter()
        .filter_map(|e| match e {
            Effect::Emit { event } => Some(serde_json::from_slice(event.0.as_ref()).unwrap()),
            _ => None,
        })
        .collect()
}

#[test]
fn foreign_company_persists_without_render_broadcast() {
    assert!(committed_events(
        "company-b",
        "im:channel:update",
        json!({"channelId": CHANNEL, "displayName":"B secret"})
    )
    .is_empty());
}

#[test]
fn active_company_still_renders_after_commit() {
    let events = committed_events(
        "company-a",
        "im:channel:update",
        json!({"channelId": CHANNEL, "displayName":"A"}),
    );
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["displayName"], "A");
}

#[test]
fn foreign_local_read_completes_without_returning_projection() {
    let events = committed_events(
        "company-b",
        "im:read:result",
        json!({"req_id":"read-b", "body":{"channel":{"id":CHANNEL,"teamId":"company-b"},"messages":[{"channelId":CHANNEL,"content":"B secret"}]}}),
    );
    assert_eq!(
        events.len(),
        1,
        "correlated read must finish without waiting for timeout"
    );
    assert_eq!(events[0]["data"]["req_id"], "read-b");
    assert!(
        events[0]["data"].get("body").is_none(),
        "foreign body must never cross Rust boundary"
    );
    assert_eq!(events[0]["data"]["error"], "RENDER_SCOPE_MISMATCH");
}

/// 即使驱动回读混合用户/公司行，离线 dialog query 的结果仍只含当前视图。
#[test]
fn offline_dialog_query_filters_mixed_user_company_rows() {
    let mut module = ImModule::new(ImConfig {
        auth_user_id: "viewer".into(),
        company_id: "company-a".into(),
        ..Default::default()
    });
    let mut sink = EffectSink::new();
    module
        .handle(
            &Tick::Command(helix_core::tick::AppCommand::new(
                "im_query_dialog_list",
                br#"{"req_id":"offline"}"#.to_vec(),
            )),
            0,
            &mut sink,
        )
        .unwrap();
    let corr = sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .unwrap();
    let rows = json!([
        {"id":CHANNEL,"team_id":"company-a","user_id":"viewer","display_name":"A"},
        {"id":"chfixx00000000000000000002","team_id":"company-b","user_id":"viewer","display_name":"B"},
        {"id":"chfixx00000000000000000003","team_id":"company-a","user_id":"other","display_name":"Other"}
    ]);
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(serde_json::to_vec(&rows).unwrap().into())),
            },
            1,
            &mut sink,
        )
        .unwrap();
    let event: Value = sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .unwrap();
    assert_eq!(event["data"]["body"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(event["data"]["body"]["items"][0]["id"], CHANNEL);
}

/// 跨公司补水真实经历 HTTP→PersistOk→四段读回，但不会返回任何业务投影。
#[test]
fn foreign_hydration_completes_storage_lifecycle_without_projection() {
    let mut module = ImModule::new(ImConfig {
        auth_user_id: "viewer-g13b".into(),
        company_id: "company-a".into(),
        api_base_url: "http://scope.invalid/api".into(),
        default_api_base_url: "http://scope.invalid/api".into(),
        ..Default::default()
    });
    let mut sink = EffectSink::new();
    const CH: &str = "chfixx0000000000000000003b";
    module
        .handle(
            &Tick::Command(helix_core::tick::AppCommand::new(
                "im_channel_load_increment_by_channel_id",
                serde_json::to_vec(&json!({"channel_id":CH,"req_id":"hydrate-b"})).unwrap(),
            )),
            0,
            &mut sink,
        )
        .unwrap();
    let corr = sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .unwrap();
    // Go ADR-007 fixture: company-g13b，当前 viewer 是真实成员，needSync=false。
    let body="eyJkYXRhIjp7ImlkIjoiY2hmaXh4MDAwMDAwMDAwMDAwMDAwMDAwM2IiLCJ0ZWFtSWQiOiJjb21wYW55LWcxM2IiLCJkaXNwbGF5TmFtZSI6IkcxM2IiLCJsYXN0RXZlbnRTZXEiOjAsInVucmVhZENvdW50IjozLCJuZWVkU3luYyI6ZmFsc2UsIm1lbWJlckNvdW50IjoxLCJtZW1iZXJzIjpbeyJ1c2VySWQiOiJ2aWV3ZXItZzEzYiIsInJvbGUiOiJNRU1CRVIifV19fQ==";
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(
                    serde_json::to_vec(&json!({"status":200,"headers":[],"body":body}))
                        .unwrap()
                        .into(),
                )),
            },
            1,
            &mut sink,
        )
        .unwrap();
    let persist = sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::PersistAtomic { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("foreign authorized hydration must persist");
    assert!(!sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Emit { .. })));
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: persist,
                outcome: PortOutcome::Ok(ReplyBytes(Bytes::new())),
            },
            2,
            &mut sink,
        )
        .unwrap();
    for rows in [
        json!([{"id":CH,"team_id":"company-g13b","user_id":"viewer-g13b","members":"[]"}]),
        json!([{"channel_id":CH,"team_id":"","user_id":"viewer-g13b","role":"MEMBER"}]),
        json!([]),
        json!([{"channel_id":CH,"last_event_seq":0}]),
    ] {
        let corr = sink
            .as_slice()
            .iter()
            .find_map(|e| match e {
                Effect::Persist { corr, .. } => Some(*corr),
                _ => None,
            })
            .expect("next durable readback");
        sink.clear();
        module
            .handle(
                &Tick::PortReply {
                    corr,
                    outcome: PortOutcome::Ok(ReplyBytes(serde_json::to_vec(&rows).unwrap().into())),
                },
                3,
                &mut sink,
            )
            .unwrap();
    }
    let events: Vec<Value> = sink
        .as_slice()
        .iter()
        .filter_map(|e| match e {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "im:read:result");
    assert!(events[0]["data"].get("body").is_none());
    assert_eq!(events[0]["data"]["error"], "RENDER_SCOPE_MISMATCH");
}

/// 公司切换只改变可见集合；失败的归属写入和重复回执不能污染持久 scope。
#[test]
fn failed_scope_change_does_not_relabel_channel_and_switch_uses_real_company() {
    let mut module = ImModule::new(ImConfig {
        auth_user_id: "viewer".into(),
        company_id: "company-a".into(),
        ..Default::default()
    });
    let mut sink = EffectSink::new();
    let channel_op = |team: &str| {
        StorageOp::BatchUpsert(UpsertSpec::new(
            "channel",
            vec![vec![
                ("id".into(), SqlValue::Text(CHANNEL.into())),
                ("team_id".into(), SqlValue::Text(team.into())),
                ("user_id".into(), SqlValue::Text("viewer".into())),
            ]],
            Some("id"),
        ))
    };
    let reply = |module: &mut ImModule, sink: &mut EffectSink, corr, outcome| {
        sink.clear();
        module
            .handle(&Tick::PortReply { corr, outcome }, 0, sink)
            .unwrap();
    };
    let success = || PortOutcome::Ok(ReplyBytes(Bytes::new()));
    let corr = module.queue_message_v3_commit(vec![channel_op("company-a")], vec![], &mut sink);
    reply(&mut module, &mut sink, corr, success());
    let corr = module.queue_message_v3_commit(vec![channel_op("company-b")], vec![], &mut sink);
    reply(
        &mut module,
        &mut sink,
        corr,
        PortOutcome::Err(helix_core::tick::PortError::Storage(1)),
    );
    // 重复成功回执不能把失败的 scope 写入复活。
    reply(&mut module, &mut sink, corr, success());
    let event = || MessageV3Event::new("im:channel:update", json!({"channelId":CHANNEL})).unwrap();
    let corr = module.queue_message_v3_commit(vec![], vec![event()], &mut sink);
    reply(&mut module, &mut sink, corr, success());
    assert!(
        sink.as_slice()
            .iter()
            .any(|e| matches!(e, Effect::Emit { .. })),
        "A remains visible after failed relabel"
    );
    sink.clear();
    module
        .handle(
            &Tick::Command(helix_core::tick::AppCommand::new(
                helix_im::module::RUNTIME_IDENTITY_COMMAND,
                br#"{"company_id":"company-b"}"#.to_vec(),
            )),
            0,
            &mut sink,
        )
        .unwrap();
    let corr = module.queue_message_v3_commit(vec![], vec![event()], &mut sink);
    reply(&mut module, &mut sink, corr, success());
    assert!(
        !sink
            .as_slice()
            .iter()
            .any(|e| matches!(e, Effect::Emit { .. })),
        "same user switch cannot render A in B"
    );
}

/// 外部成员发在当前公司频道的消息按频道可见，不按作者 teamId 错杀。
#[test]
fn active_channel_message_keeps_external_author_company() {
    let events = committed_events(
        "company-a",
        "im:post:received",
        json!({
            "channelId":CHANNEL,"post":{"id":"postxx00000000000000000001","channelId":CHANNEL,"teamId":"company-b","userId":"external-author","message":"allowed A message"}
        }),
    );
    assert_eq!(events.len(), 1);
}

/// 尚未落库的外公司频道行仍不能返回；成员 id/teamId 不冒充频道身份。
#[test]
fn unknown_foreign_channel_catalog_is_not_returned() {
    let events = committed_events(
        "company-a",
        "im:read:result",
        json!({"req_id":"catalog","body":{"channel":{"id":"chfixx00000000000000000099","teamId":"company-b","displayName":"foreign"}}}),
    );
    assert_eq!(events[0]["data"]["error"], "RENDER_SCOPE_MISMATCH");
    assert!(events[0]["data"].get("body").is_none());
}

/// 单频道读回可能只有纯文本：即使响应里无 channelId，也必须沿 correlation 绑定的频道拦截。
#[test]
fn foreign_channel_scalar_http_read_cannot_bypass_gate() {
    let mut module = ImModule::new(ImConfig {
        auth_user_id: "viewer".into(),
        company_id: "company-a".into(),
        api_base_url: "http://scope.invalid/api".into(),
        default_api_base_url: "http://scope.invalid/api".into(),
        ..Default::default()
    });
    let mut sink = EffectSink::new();
    let command = helix_core::tick::AppCommand::new(
        "im_channel_load_notice",
        serde_json::to_vec(&json!({"channel_id":CHANNEL,"req_id":"notice-b"})).unwrap(),
    );
    module
        .handle(&Tick::Command(command), 0, &mut sink)
        .unwrap();
    let corr = sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .unwrap();
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(
                    serde_json::to_vec(
                        &json!({"status":200,"headers":[],"body":"eyJkYXRhIjoiQiBzZWNyZXQifQ=="}),
                    )
                    .unwrap()
                    .into(),
                )),
            },
            1,
            &mut sink,
        )
        .unwrap();
    let event: Value = sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .unwrap();
    assert_eq!(event["data"]["error"], "RENDER_SCOPE_MISMATCH");
    assert!(event["data"].get("body").is_none());
}

/// byIds 的频道身份位于 map key，合法 A 成员的 B 公司与 26 字符用户 ID 不能被误判。
#[test]
fn members_by_ids_checks_channel_keys_not_member_company() {
    for foreign in [false, true] {
        let mut module = ImModule::new(ImConfig {
            auth_user_id: "viewer".into(),
            company_id: "company-a".into(),
            api_base_url: "http://scope.invalid/api".into(),
            default_api_base_url: "http://scope.invalid/api".into(),
            ..Default::default()
        });
        let mut sink = EffectSink::new();
        let corr = module.queue_message_v3_commit(
            vec![StorageOp::BatchUpsert(UpsertSpec::new(
                "channel",
                vec![vec![
                    ("id".into(), SqlValue::Text(CHANNEL.into())),
                    ("team_id".into(), SqlValue::Text("company-a".into())),
                    ("user_id".into(), SqlValue::Text("viewer".into())),
                ]],
                Some("id"),
            ))],
            vec![],
            &mut sink,
        );
        sink.clear();
        module
            .handle(
                &Tick::PortReply {
                    corr,
                    outcome: PortOutcome::Ok(ReplyBytes::default()),
                },
                0,
                &mut sink,
            )
            .unwrap();
        sink.clear();
        module
            .handle(
                &Tick::Command(helix_core::tick::AppCommand::new(
                    "im_channels_members_by_ids",
                    serde_json::to_vec(&json!({"channel_ids":[CHANNEL],"req_id":"members"}))
                        .unwrap(),
                )),
                1,
                &mut sink,
            )
            .unwrap();
        let corr = sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Http { corr, .. } => Some(*corr),
                _ => None,
            })
            .unwrap();
        let body = if foreign {
            json!({"status":"SUCCESS","data":{"chfixx00000000000000000099":[]}})
        } else {
            json!({"status":"SUCCESS","data":{CHANNEL:[{"id":"user0000000000000000000001","teamId":"company-b","role":"MEMBER","nickName":"External"}]}})
        };
        sink.clear();
        module
            .handle(
                &Tick::PortReply {
                    corr,
                    outcome: PortOutcome::Ok(ReplyBytes(common::enveloped(
                        serde_json::to_vec(&body).unwrap().into(),
                    ))),
                },
                2,
                &mut sink,
            )
            .unwrap();
        let events: Vec<Value> = sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
                _ => None,
            })
            .collect();
        let read = events
            .iter()
            .find(|event| event["event"] == "im:read:result")
            .unwrap();
        if foreign {
            assert_eq!(read["data"]["error"], "RENDER_SCOPE_MISMATCH");
            assert!(read["data"].get("body").is_none());
            assert_eq!(events.len(), 1);
        } else {
            assert_eq!(
                read["data"]["body"]["data"][CHANNEL][0]["teamId"],
                "company-b"
            );
            assert!(events
                .iter()
                .any(|event| event["event"] == "im:channel:members"));
        }
    }
}

/// 当前公司目录可在尚未落库时返回，但真实资源公司必须明确匹配。
#[test]
fn channel_query_checks_resource_company_before_raw_relay() {
    for company in ["company-a", "company-b", ""] {
        let mut module = scoped_module();
        let mut sink = EffectSink::new();
        let corr = start_catalog_read(&mut module, &mut sink);
        let body = json!({"status":"SUCCESS","data":[{
            "id":CHANNEL,"teamId":company,"displayName":"catalog"
        }]});
        sink.clear();
        module
            .handle(
                &Tick::PortReply {
                    corr,
                    outcome: PortOutcome::Ok(ReplyBytes(common::enveloped(
                        serde_json::to_vec(&body).unwrap().into(),
                    ))),
                },
                1,
                &mut sink,
            )
            .unwrap();
        let events = emitted(&sink);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["data"]["req_id"], "catalog");
        if company == "company-a" {
            assert_eq!(events[0]["data"]["body"], body);
        } else {
            assert_eq!(events[0]["data"]["error"], "RENDER_SCOPE_MISMATCH");
            assert!(events[0]["data"].get("body").is_none());
        }
    }
}

/// A→B→A 的迟到 HTTP 响应不能复活；旧请求只收到一次无渲染数据的取消终态。
#[test]
fn company_switch_cancels_pending_raw_reads_even_after_switching_back() {
    let mut module = scoped_module();
    let mut sink = EffectSink::new();
    let corr = start_catalog_read(&mut module, &mut sink);
    for company in ["company-b", "company-a"] {
        sink.clear();
        module
            .handle(
                &Tick::Command(helix_core::tick::AppCommand::new(
                    helix_im::module::RUNTIME_IDENTITY_COMMAND,
                    serde_json::to_vec(&json!({"company_id":company})).unwrap(),
                )),
                1,
                &mut sink,
            )
            .unwrap();
        let events = emitted(&sink);
        if company == "company-b" {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0]["data"]["req_id"], "catalog");
            assert_eq!(events[0]["data"]["error"], "CANCELLED");
            assert!(events[0]["data"].get("body").is_none());
        } else {
            assert!(events.is_empty(), "cancel must be exactly once");
        }
    }
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(common::enveloped(
                    serde_json::to_vec(&json!({"data":[{"id":CHANNEL,"teamId":"company-a"}]}))
                        .unwrap()
                        .into(),
                ))),
            },
            2,
            &mut sink,
        )
        .unwrap();
    assert!(
        emitted(&sink).is_empty(),
        "late reply cannot revive the old projection"
    );
}

fn scoped_module() -> ImModule {
    ImModule::new(ImConfig {
        auth_user_id: "viewer".into(),
        company_id: "company-a".into(),
        api_base_url: "http://scope.invalid/api".into(),
        default_api_base_url: "http://scope.invalid/api".into(),
        ..Default::default()
    })
}

fn start_catalog_read(module: &mut ImModule, sink: &mut EffectSink) -> helix_core::Correlation {
    module
        .handle(
            &Tick::Command(helix_core::tick::AppCommand::new(
                "im_channel_query",
                br#"{"req_id":"catalog"}"#.to_vec(),
            )),
            0,
            sink,
        )
        .unwrap();
    sink.as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .unwrap()
}

fn emitted(sink: &EffectSink) -> Vec<Value> {
    sink.as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => Some(serde_json::from_slice(event.0.as_ref()).unwrap()),
            _ => None,
        })
        .collect()
}

/// hello 抢先到达不能把“尚未读出公司频道集合”当作“当前公司没有消息”。
#[test]
fn cold_hello_waits_for_channel_scope_before_reading_watermark() {
    let mut module = scoped_module();
    let mut sink = EffectSink::new();
    module.on_start(&mut sink).unwrap();
    let cursor_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .unwrap();
    sink.clear();
    module
        .handle(&common::hello_inbound("cold-scope"), 1, &mut sink)
        .unwrap();
    assert!(
        !sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Http { .. } | Effect::Persist { .. })),
        "cold hello must wait for its in-flight channel scan instead of querying an empty scope"
    );
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: cursor_corr,
                outcome: PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"[]"))),
            },
            2,
            &mut sink,
        )
        .unwrap();
    let channel_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .unwrap();
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: channel_corr,
                outcome: PortOutcome::Ok(ReplyBytes(
                    serde_json::to_vec(&json!([
                        {"id":CHANNEL,"team_id":"company-a","user_id":"viewer"},
                        {"id":"chfixx00000000000000000002","team_id":"company-b","user_id":"viewer"}
                    ]))
                    .unwrap()
                    .into(),
                )),
            },
            3,
            &mut sink,
        )
        .unwrap();
    let max_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } => match ops.as_slice() {
                [StorageOp::ScopedMax(spec)] => {
                    assert!(matches!(spec.scope_values.as_slice(), [SqlValue::Text(id)] if id == CHANNEL));
                    Some(*corr)
                }
                _ => None,
            },
            _ => None,
        })
        .expect("completed channel scan must request the real company watermark");
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: max_corr,
                outcome: PortOutcome::Ok(ReplyBytes(Bytes::from_static(br#"[{"create_at":300}]"#))),
            },
            4,
            &mut sink,
        )
        .unwrap();
    let bodies: Vec<Value> = sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Http { req, .. } => {
                Some(serde_json::from_slice(req.body.as_ref().unwrap()).unwrap())
            }
            _ => None,
        })
        .collect();
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0]["timestamp"], 300);
}

/// 回归锚：0921 latest 超时；话题引用没有本地归属也不能吞掉父消息窗口。
#[test]
fn latest_query_keeps_topic_reference_without_using_its_scope() {
    for topic_company in [None, Some("company-a"), Some("company-b")] {
        let mut h = common::Harness::new_with_identity(&[1], "viewer", "company-a", "", "", "");
        if let Some(company) = topic_company {
            h.seed_channel_scope(&common::cid(2), company, "viewer");
        }
        let topic = json!({"channelId":common::cid(2),"participantCount":3});
        let rows = json!([{
            "id":common::sid(1),"temporary_id":"topic-parent","channel_id":common::cid(1),
            "user_id":"viewer","type":"TEXT","message":"parent message","create_at":100,
            "topic":topic.to_string(),"viewers":"[\"all\"]"
        }]);
        let remote = common::enveloped(
            serde_json::to_vec(&json!({
                "status":"SUCCESS","data":{"posts":[{
                    "id":common::sid(1),"temporaryId":"topic-parent","channelId":common::cid(1),
                    "userId":"viewer","type":"TEXT","message":"parent message","createAt":100,
                    "topic":topic,"viewers":["all"]
                }]}
            }))
            .unwrap()
            .into(),
        );
        // 第二次查询走已建立 coverage 的本地直出，同样必须结算。
        for round in 0..2 {
            let token = format!("topic-window-{round}");
            h.step(
                Tick::Command(helix_core::tick::AppCommand::new(
                    "im_query_messages_by_channel",
                    serde_json::to_vec(&json!({
                        "channel_id":common::cid(1),"limit":20,"windowToken":token
                    }))
                    .unwrap(),
                )),
                1000,
            );
            let replies = if round == 0 {
                vec![
                    Bytes::from_static(b"[]"),
                    remote.clone(),
                    Bytes::from_static(b"[]"),
                    Bytes::from(rows.to_string()),
                ]
            } else {
                vec![Bytes::from(rows.to_string())]
            };
            for (index, body) in replies.into_iter().enumerate() {
                assert!(
                    emitted(&h.sink).is_empty(),
                    "no ready before durable readback"
                );
                let corr = match h.sink.as_slice() {
                    [Effect::Http { corr, .. }] if round == 0 && index == 1 => *corr,
                    [Effect::Persist { corr, .. }] => *corr,
                    other => panic!("unexpected query stage: {other:?}"),
                };
                h.step(
                    common::port_reply_with_body(corr.raw(), body),
                    1001 + index as u64,
                );
            }
            let events = emitted(&h.sink);
            assert_eq!(
                events.len(),
                1,
                "topic company: {topic_company:?}, round: {round}"
            );
            assert_eq!(events[0]["event"], "im:timeline:ready");
            assert_eq!(events[0]["data"]["state"], "ready");
            assert_eq!(events[0]["data"]["windowToken"], token);
            assert_eq!(events[0]["data"]["messages"][0]["topic"], topic);
        }
    }
}

/// 话题引用豁免不能授权主会话或窗口里的另一条未知频道消息。
#[test]
fn topic_reference_does_not_bypass_message_scope() {
    for (company, message_channel) in [("company-b", CHANNEL), ("company-a", "unknown-channel")] {
        let events = committed_events(
            company,
            "im:timeline:ready",
            json!({
                "channelId":CHANNEL,"messages":[{"channelId":message_channel,
                    "topic":{"channelId":CHANNEL,"participantCount":3}}]
            }),
        );
        assert!(events.is_empty());
    }
}

#[test]
fn personal_account_renders_joined_channel_across_companies_after_commit() {
    for tenant in ["company-b", "personal-b"] {
        let events = committed_events_for(
            "",
            "viewer",
            tenant,
            "im:channel:update",
            json!({"channelId":CHANNEL,"teamId":tenant}),
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["data"]["teamId"], tenant);
    }
    assert!(committed_events_for(
        "",
        "other-user",
        "company-b",
        "im:channel:update",
        json!({"channelId":CHANNEL,"teamId":"company-b"})
    )
    .is_empty());
}
