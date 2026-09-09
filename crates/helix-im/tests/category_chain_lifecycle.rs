//! 分类接龙公开 seam：先锁住命令路由，随后覆盖 authority/persist/readback/terminal 生命周期。
mod common;

use common::{cid, Harness};
use helix_core::{Effect, Tick};
use serde_json::json;

/// 回归锚：分类接龙必须独立认领命令，不能落入普通发送或文字接龙。
#[test]
fn category_create_enters_public_command_seam() {
    let mut harness = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let command = helix_im::client_api::build_command(
        "im_category_chain_create_draft",
        &serde_json::to_vec(&json!({
            "channelId":cid(1), "title":"活动", "description":"", "example":"",
            "deadlineAt":1789113600000_i64, "audience":{"mode":"CHANNEL_SNAPSHOT"},
            "categories":[{"clientId":"a","name":"室内"},{"clientId":"b","name":"户外"}],
            "max_categories":2, "clientMutationId":"mutation-create"
        }))
        .unwrap(),
    )
    .unwrap();
    harness.step(Tick::Command(command), 1000);
    assert!(harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Http { .. })));
    assert!(!harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));
}

/// 新草稿的上限必须经公开命令边界进入同一条 CATEGORY HTTP wire。
#[test]
fn category_max_categories_maps_create_draft_to_http() {
    let (_, body) = category_mode_http(
        "im_category_chain_create_draft",
        json!({
            "channelId": cid(1), "title": "活动", "description": "", "example": "",
            "deadlineAt": 1789113600000_i64, "audience": {"mode": "CHANNEL_SNAPSHOT"},
            "categories": [{"clientId": "a", "name": "室内"}, {"clientId": "b", "name": "户外"}],
            "max_categories": 2,
            "clientMutationId": "max-create"
        }),
    );
    assert_eq!(body["maxCategories"], 2);
}

/// 新建和更新草稿必须显式带上限；无效输入在公开 command 边界失败。
#[test]
fn category_max_categories_draft_wire_requires_bounded_input() {
    use helix_core::Module;

    let base = json!({
        "channelId": cid(1), "title": "活动", "description": "", "example": "",
        "deadlineAt": 1789113600000_i64, "audience": {"mode": "CHANNEL_SNAPSHOT"},
        "categories": [{"clientId": "a", "name": "室内"}, {"clientId": "b", "name": "户外"}],
        "clientMutationId": "max-required"
    });
    let missing = helix_im::client_api::build_command(
        "im_category_chain_create_draft",
        &serde_json::to_vec(&base).unwrap(),
    )
    .unwrap();
    let mut missing_harness = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    assert!(missing_harness
        .module
        .handle(&Tick::Command(missing), 1000, &mut missing_harness.sink)
        .is_err());
    assert!(missing_harness.sink.as_slice().is_empty());

    let mut update = base.clone();
    update["chainId"] = json!("chain-a");
    update["expectedDefinitionRevision"] = json!("9007199254740993");
    update["max_categories"] = json!(1);
    let (_, mapped) = category_mode_http("im_category_chain_update_draft", update);
    assert_eq!(mapped["maxCategories"], 1);

    for max_categories in [
        Value::Null,
        json!("1"),
        json!(1.5),
        json!(0),
        json!(-1),
        json!(3),
    ] {
        let mut args = base.clone();
        args["max_categories"] = max_categories;
        let command = helix_im::client_api::build_command(
            "im_category_chain_create_draft",
            &serde_json::to_vec(&args).unwrap(),
        )
        .unwrap();
        let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
        assert!(h
            .module
            .handle(&Tick::Command(command), 1000, &mut h.sink)
            .is_err());
        assert!(h.sink.as_slice().is_empty());
    }
}

use bytes::Bytes;
use helix_core::effect::{Correlation, SqlValue, StorageOp};
use helix_core::tick::{InboundBytes, PortError, PortOutcome, ReplyBytes};
use serde_json::Value;
use std::collections::BTreeMap;

type Store = BTreeMap<String, BTreeMap<String, Value>>;

/// 回放host scalar row表示；不使用手写业务event作为结果。
fn sql(value: &SqlValue) -> Value {
    match value {
        SqlValue::Text(s) => json!(s),
        SqlValue::Integer(n) => json!(n),
        SqlValue::Real(n) => json!(n),
        SqlValue::Null => Value::Null,
        SqlValue::Blob(_) => panic!("category facts are scalar"),
    }
}

/// 用确定性内存表兑现公开StorageOp；真实SQLite由host独立测试证明。
fn storage_reply(ops: &[StorageOp], store: &mut Store) -> Value {
    let mut output = Vec::new();
    for op in ops {
        match op {
            StorageOp::BatchUpsert(spec) => {
                for row in &spec.rows {
                    let value =
                        Value::Object(row.iter().map(|(k, v)| (k.clone(), sql(v))).collect());
                    let key = if spec.table == "category_chain_head" {
                        value["scope"].to_string()
                    } else if spec.table == "message" {
                        value["temporary_id"].to_string()
                    } else {
                        json!([value["snapshot_key"], value["path"]]).to_string()
                    };
                    let rows = store.entry(spec.table.to_owned()).or_default();
                    if let Some(old) = rows.get_mut(&key) {
                        if let Some(guard) = &spec.update_guard {
                            assert_eq!(
                                old[guard.column],
                                sql(&guard.expected),
                                "unexpected synthetic guard mismatch"
                            );
                        }
                        old.as_object_mut().unwrap().extend(
                            value
                                .as_object()
                                .unwrap()
                                .iter()
                                .map(|(k, v)| (k.clone(), v.clone())),
                        );
                    } else {
                        rows.insert(key, value);
                    }
                }
            }
            StorageOp::Get(spec) => {
                output.extend(
                    store
                        .get(spec.table)
                        .into_iter()
                        .flat_map(|rows| rows.values())
                        .filter(|row| row[spec.key_col] == sql(&spec.key_val))
                        .cloned(),
                );
            }
            StorageOp::Scan(spec) => {
                output.extend(
                    store
                        .get(spec.table)
                        .into_iter()
                        .flat_map(|rows| rows.values())
                        .filter(|row| {
                            spec.filter
                                .as_ref()
                                .is_none_or(|(key, val)| row[*key] == sql(val))
                        })
                        .cloned(),
                );
                for order in spec.order_by.iter().rev() {
                    output.sort_by(|a, b| a[order.column].as_str().cmp(&b[order.column].as_str()));
                }
                if let Some(limit) = spec.limit {
                    output.truncate(limit as usize);
                }
            }
            _ => panic!("unexpected storage op: {op:?}"),
        }
    }
    json!(output)
}

/// 模拟host实际numeric-status/base64信封，业务body不能直接当PortReply。
fn http_bytes(raw: impl AsRef<[u8]>) -> Bytes {
    common::enveloped(Bytes::copy_from_slice(raw.as_ref()))
}

/// 完整执行有界存储continuation直到终态，捕获真正ImModule输出。
fn drain(h: &mut Harness, store: &mut Store) -> Vec<Value> {
    let mut events = Vec::new();
    for _ in 0..32 {
        let mut persist = None;
        for effect in h.sink.as_slice() {
            match effect {
                Effect::Persist { corr, ops } | Effect::PersistAtomic { corr, ops } => {
                    persist = Some((*corr, storage_reply(ops, store)))
                }
                Effect::Emit { event } => events.push(serde_json::from_slice(&event.0).unwrap()),
                _ => {}
            }
        }
        let Some((corr, body)) = persist else {
            return events;
        };
        h.step(
            Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(
                    serde_json::to_vec(&body).unwrap(),
                ))),
            },
            1100,
        );
    }
    panic!("category continuation did not settle");
}

/// 生成服务端输入fixture；终态仍必须由公开Command回放生产。
fn card(revision: &str) -> Value {
    json!({"schemaVersion":1,"chainId":"chain-a","revision":revision,"status":"OPEN","title":"活动","description":"","example":"","deadlineAt":1789113600000_i64,"audienceCount":5,"participantCount":1,"categoryCount":2,"maxCategories":2,"categories":[
        {"categoryId":"a","name":"室内","ordinal":0,"tone":"purple","revision":revision,"participantCount":1,"entries":[],"hasMore":true},
        {"categoryId":"b","name":"户外","ordinal":1,"tone":"blue","revision":revision,"participantCount":1,"entries":[],"hasMore":true}
    ]})
}

/// 本人状态含多类完整选择，不从卡片预览推断。
fn mine() -> Value {
    json!({"chainId":"chain-a","participationRevision":"1","entries":[],"canParticipate":true,"canEdit":true,"canCancel":true,"denialReason":null,"maxCategories":2})
}

/// CanonicalPost输入fixture，沿真实parser和message upsert生成输出。
fn post(revision: &str) -> Value {
    json!({"id":common::sid(1),"temporaryId":"category-anchor-1","channelId":cid(1),"userId":"user-a","type":"CATEGORY_CHAIN","message":"分类接龙：活动","simpleMessage":"[分类接龙] 活动","viewers":["all"],"props":{"categoryChain":card(revision)},"createAt":1788770000000_i64,"updateAt":1788770000000_i64})
}

/// 保留权威operation关联和十进制大版本，不假造Post身份。
fn mutation(revision: &str, id: &str) -> Value {
    json!({"outcome":"CONFIRMED","commandKind":"set-participation","clientMutationId":id,"operationId":format!("op-{id}"),"chainId":"chain-a","revision":revision,"eventId":null,"eventSeq":null,"draft":null,"card":card(revision),"post":post(revision),"myParticipation":mine()})
}

/// 发起整组mutation并取真实HTTP correlation。
fn set(h: &mut Harness, id: &str) -> Correlation {
    let command=helix_im::client_api::build_command("im_category_chain_set_participation",&serde_json::to_vec(&json!({"channelId":cid(1),"chainId":"chain-a","expectedParticipationRevision":"0","selections":[],"clientMutationId":id})).unwrap()).unwrap();
    h.step(Tick::Command(command), 1000);
    h.sink
        .as_slice()
        .iter()
        .find_map(|e| {
            if let Effect::Http { corr, .. } = e {
                Some(*corr)
            } else {
                None
            }
        })
        .expect("HTTP")
}

/// HTTP authority只是观察，不能越过PersistAtomic和readback终态。
#[test]
fn category_authority_waits_for_commit_and_readback_and_ignores_duplicate_reply() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let corr = set(&mut h, "m1");
    h.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(http_bytes(
                serde_json::to_vec(
                    &json!({"status":"SUCCESS","data":mutation("9007199254740993","m1")}),
                )
                .unwrap(),
            ))),
        },
        1001,
    );
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Emit { .. })));
    let mut store = Store::new();
    let events = drain(&mut h, &mut store);
    let event = events
        .iter()
        .find(|e| e["event"] == "im:category_chain:operation")
        .expect("durable projection");
    assert_eq!(event["data"]["card"]["revision"], "9007199254740993");
    assert_eq!(event["data"]["card"]["participantCount"], 1);
    assert_eq!(event["data"]["operationId"], "op-m1");
    h.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(http_bytes(b"{}"))),
        },
        1200,
    );
    assert!(h.sink.as_slice().is_empty());
}

/// 超时释放correlation/timer，迟到SUCCESS不确认原操作。
#[test]
fn category_timeout_is_reconciling_and_late_success_is_ignored() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let corr = set(&mut h, "timeout");
    let timer = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| {
            if let Effect::ScheduleTimer { id, .. } = e {
                Some(*id)
            } else {
                None
            }
        })
        .unwrap();
    h.step(Tick::Timer(timer), 31000);
    let event = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| {
            if let Effect::Emit { event } = e {
                Some(serde_json::from_slice::<Value>(&event.0).unwrap())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(event["data"]["state"], "RECONCILING");
    assert_private_operation(&event, None);
    operation_fixture("reconciling", &event);
    h.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(http_bytes(b"{}"))),
        },
        31001,
    );
    assert!(h.sink.as_slice().is_empty());
}

/// 端口/持久化失败都不能产生成功投影。
#[test]
fn category_storage_failure_never_confirms() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let corr = set(&mut h, "persist-fail");
    h.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(http_bytes(
                serde_json::to_vec(
                    &json!({"status":"SUCCESS","data":mutation("1","persist-fail")}),
                )
                .unwrap(),
            ))),
        },
        1001,
    );
    let head = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| {
            if let Effect::Persist { corr, .. } = e {
                Some(*corr)
            } else {
                None
            }
        })
        .unwrap();
    h.step(
        Tick::PortReply {
            corr: head,
            outcome: PortOutcome::Err(PortError::Other(1)),
        },
        1002,
    );
    let events: Vec<Value> = h
        .sink
        .as_slice()
        .iter()
        .filter_map(|e| match e {
            Effect::Emit { event } => Some(serde_json::from_slice(&event.0).unwrap()),
            _ => None,
        })
        .collect();
    assert!(events.iter().any(|e| e["data"]["state"] == "RECONCILING"));
    assert!(!events.iter().any(|e| e["data"]["state"] == "CONFIRMED"));
}

/// 无效/未知命令不产生请求；分类输入不能伪造actor。
#[test]
fn category_invalid_and_unknown_commands_fail_closed() {
    use helix_core::Module;
    assert!(helix_im::client_api::build_command("im_category_chain_unknown", b"{}").is_err());
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let cmd=helix_im::client_api::build_command("im_category_chain_set_participation",&serde_json::to_vec(&json!({"channelId":cid(1),"chainId":"chain-a","userId":"victim","expectedParticipationRevision":"0","selections":[],"clientMutationId":"bad"})).unwrap()).unwrap();
    assert!(h
        .module
        .handle(&Tick::Command(cmd), 1000, &mut h.sink)
        .is_err());
    assert!(h.sink.as_slice().is_empty());
}

/// WS共享投影不得携带任何人的私有权限状态。
#[test]
fn category_ws_private_state_is_rejected() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    h.step(Tick::Inbound(InboundBytes(Bytes::from(serde_json::to_vec(&json!({"action":"im:category_chain:projection","data":{"schemaVersion":1,"channelId":cid(1),"chainId":"chain-a","revision":"1","eventId":"e1","eventSeq":"1","anchorPostId":common::sid(1),"temporaryId":"category-anchor-1","card":card("1"),"myParticipation":mine()}})).unwrap()))),1000);
    assert!(h.sink.as_slice().is_empty());
}

/// 重新创建ImModule仍读取durable head，旧HTTP结果不覆盖较新卡片。
#[test]
fn category_restart_keeps_newer_canonical_revision() {
    let mut store = Store::new();
    for (revision, id) in [("10", "first"), ("9", "late")] {
        let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
        let corr = set(&mut h, id);
        h.step(
            Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(http_bytes(
                    serde_json::to_vec(&json!({"status":"SUCCESS","data":mutation(revision,id)}))
                        .unwrap(),
                ))),
            },
            1001,
        );
        let events = drain(&mut h, &mut store);
        let event = events
            .iter()
            .find(|e| e["event"] == "im:category_chain:operation")
            .unwrap();
        if revision == "10" {
            assert_eq!(event["data"]["card"]["revision"], "10");
        } else {
            assert_eq!(event["data"]["state"], "RECONCILING");
            assert!(event["data"]["card"].is_null());
            assert!(!events.iter().any(|e| e["event"] == "im:post:updated"));
        }
    }
}

/// 业务failed精确转为REJECTED，不尝试把错误data持久化成Card。
#[test]
fn category_business_failure_is_rejected() {
    for code in ["PARTICIPATION_CONFLICT", "PARTICIPATION_LIMIT_EXCEEDED"] {
        let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
        let corr = set(&mut h, "business-error");
        h.step(
            Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(http_bytes(
                    serde_json::to_vec(&json!({"status":"failed","data":{"code":code}})).unwrap(),
                ))),
            },
            1001,
        );
        let event = h
            .sink
            .as_slice()
            .iter()
            .find_map(|e| {
                if let Effect::Emit { event } = e {
                    Some(serde_json::from_slice::<Value>(&event.0).unwrap())
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(event["data"]["state"], "REJECTED");
        assert_private_operation(&event, None);
        if code == "PARTICIPATION_CONFLICT" {
            operation_fixture("rejected", &event);
        }
        assert!(event["data"]["errorCode"]
            .as_str()
            .is_some_and(|actual| actual.contains(code)));
    }
}

/// runtime退出取消timer并移除correlation；旧账号回包无权继续。
#[test]
fn category_stop_cancels_inflight_and_blocks_late_results() {
    use helix_core::Module;
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let corr = set(&mut h, "stop");
    h.sink.clear();
    h.module.on_stop(&mut h.sink).unwrap();
    assert!(h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::CancelTimer { .. })));
    h.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(http_bytes(b"{}"))),
        },
        1001,
    );
    assert!(h.sink.as_slice().is_empty());
}

/// 后端能力false原样传给waiter，客户端命令注册不等于服务已上线。
#[test]
fn category_capability_false_is_preserved() {
    let mut h = Harness::new(&[]);
    let cmd = helix_im::client_api::build_command(
        "im_category_chain_capabilities",
        br#"{"req_id":"caps"}"#,
    )
    .unwrap();
    h.step(Tick::Command(cmd), 1000);
    let corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| {
            if let Effect::Http { corr, .. } = e {
                Some(*corr)
            } else {
                None
            }
        })
        .unwrap();
    h.step(Tick::PortReply {corr,outcome:PortOutcome::Ok(ReplyBytes(http_bytes(br#"{"status":"SUCCESS","data":{"categoryChain":{"schemaVersion":1,"supported":false}}}"#)))},1001);
    let event = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| {
            if let Effect::Emit { event } = e {
                Some(serde_json::from_slice::<Value>(&event.0).unwrap())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(
        event["data"]["body"]["data"]["categoryChain"]["supported"],
        false
    );
}

/// 发布的transport身份先落库；重启及重试相同mutation不会生成第二个锚点身份。
#[test]
fn category_publish_identity_is_durable_and_stable() {
    let mut store = Store::new();
    let mut identities = Vec::new();
    for _ in 0..2 {
        let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
        let cmd=helix_im::client_api::build_command("im_category_chain_publish",&serde_json::to_vec(&json!({"channelId":cid(1),"chainId":"chain-a","expectedDefinitionRevision":"1","clientMutationId":"publish-stable"})).unwrap()).unwrap();
        h.step(Tick::Command(cmd), 1000);
        assert!(h
            .sink
            .as_slice()
            .iter()
            .any(|e| matches!(e, Effect::PersistAtomic { .. })));
        assert!(!h
            .sink
            .as_slice()
            .iter()
            .any(|e| matches!(e, Effect::Http { .. })));
        assert!(drain(&mut h, &mut store).is_empty());
        let body = h.last_http_body().expect("HTTP after identity PersistOk");
        identities.push(body["temporaryId"].as_str().unwrap().to_owned());
        let corr = h
            .sink
            .as_slice()
            .iter()
            .find_map(|e| match e {
                Effect::Http { corr, .. } => Some(*corr),
                _ => None,
            })
            .unwrap();
        let mut result = mutation("1", "publish-stable");
        result["commandKind"] = json!("publish");
        result["myParticipation"] = Value::Null;
        result["post"]["temporaryId"] = body["temporaryId"].clone();
        h.step(
            Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(http_bytes(
                    serde_json::to_vec(&json!({"status":"SUCCESS","data":result})).unwrap(),
                ))),
            },
            1200,
        );
        let events = drain(&mut h, &mut store);
        if identities.len() == 1 {
            let operation = events
                .iter()
                .find(|e| e["event"] == "im:category_chain:operation")
                .unwrap();
            assert_private_operation(operation, Some(&result));
            operation_fixture("publish", operation);
            assert!(
                events.iter().any(|e| e["event"] == "im:post:received"),
                "{events:?}"
            );
            assert!(
                events
                    .iter()
                    .any(|e| e["event"] == "im:category_chain:operation"
                        && e["data"]["state"] == "CONFIRMED"),
                "{events:?}"
            );
        }
    }
    assert_eq!(identities[0], identities[1]);
    assert!(!identities[0].is_empty());
}

/// 统一回灌真实HTTP correlation，复用public command以覆盖各读族路由。
fn query(
    h: &mut Harness,
    command: &str,
    args: Value,
    data: Value,
    store: &mut Store,
) -> Vec<Value> {
    let command =
        helix_im::client_api::build_command(command, &serde_json::to_vec(&args).unwrap()).unwrap();
    h.step(Tick::Command(command), 1000);
    let corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .unwrap();
    h.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(http_bytes(
                serde_json::to_vec(&json!({"status":"SUCCESS","data":data})).unwrap(),
            ))),
        },
        1001,
    );
    drain(h, store)
}

/// 无版本号的对账观察可以从PENDING转为REJECTED或CONFIRMED，不能被rev0缓存截住。
#[test]
fn category_reconcile_advances_from_pending_to_terminal() {
    for terminal in ["REJECTED", "CONFIRMED"] {
        let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
        let mut store = Store::new();
        for state in ["PENDING", terminal] {
            let data = json!({"state":state,"clientMutationId":"reconcile-a","operationId":if state=="PENDING" {Value::Null} else {json!("op-reconcile-a")},"result":if state=="CONFIRMED" {mutation("2","reconcile-a")} else {Value::Null},"error":if state=="REJECTED" {json!({"code":"PARTICIPATION_CONFLICT","message":"conflict"})} else {Value::Null}});
            let events = query(
                &mut h,
                "im_category_chain_reconcile",
                json!({"channelId":cid(1),"chainId":"chain-a","clientMutationId":"reconcile-a","req_id":"rec"}),
                data.clone(),
                &mut store,
            );
            let reply = events
                .iter()
                .find(|e| e["event"] == "im:read:result")
                .unwrap();
            assert_eq!(
                reply["data"]["body"]["data"], data,
                "read CommonRes must remain unchanged"
            );
            if state == "CONFIRMED" {
                let operation = events
                    .iter()
                    .find(|e| e["event"] == "im:category_chain:operation")
                    .unwrap();
                assert_private_operation(operation, Some(&data["result"]));
                operation_fixture("reconcile-confirmed", operation);
            }
            if state == "CONFIRMED" {
                assert_eq!(
                    reply["data"]["body"]["data"]["result"]["card"]["revision"],
                    "2"
                );
            }
        }
    }
}

/// 公共卡片按revision幂等，但单独不能提交paired Post的频道cursor或产生未读。
#[test]
fn category_ws_card_does_not_acknowledge_post_or_duplicate_emit() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let mut store = Store::new();
    for attempt in 0..2 {
        let frame = json!({"action":"im:category_chain:projection","data":{"schemaVersion":1,"channelId":cid(1),"chainId":"chain-a","revision":"1","eventId":"e1","eventSeq":"1","anchorPostId":common::sid(1),"temporaryId":"category-anchor-1","card":card("1")}});
        h.step(
            Tick::Inbound(InboundBytes(Bytes::from(
                serde_json::to_vec(&frame).unwrap(),
            ))),
            1000,
        );
        let events = drain(&mut h, &mut store);
        if attempt == 0 {
            let projection = events
                .iter()
                .find(|e| e["event"] == "im:category_chain:projection")
                .unwrap();
            let mut original = frame["data"].clone();
            original["command"] = json!("category_chain_projection");
            assert_eq!(
                projection["data"], original,
                "public WS shape must not gain private context"
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e["event"] == "im:category_chain:projection")
                    .count(),
                1,
                "{events:?}"
            );
        } else {
            assert!(events.is_empty(), "{events:?}");
        }
        assert!(!store.contains_key("channel_event_cursor"));
        assert_eq!(h.module.cursor_for(common::channel_id(1)), Some(0));
        assert!(!store.contains_key("message"));
        assert!(!events
            .iter()
            .any(|e| e["event"] == "im:post:received" || e["event"] == "im:channel:update"));
    }
}

/// 本人集合与分页通过独立读回；详情scope不同不互相替换。
#[test]
fn category_private_and_entries_reads_preserve_complete_shapes() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let mut store = Store::new();
    let events = query(
        &mut h,
        "im_category_chain_get_mine",
        json!({"channelId":cid(1),"chainId":"chain-a","req_id":"mine"}),
        mine(),
        &mut store,
    );
    assert_eq!(events[0]["data"]["body"]["data"], mine());
    for category in ["a", "b"] {
        let page = json!({"chainId":"chain-a","categoryId":category,"categoryRevision":"1","entries":[],"nextCursor":null,"hasMore":false});
        let events = query(
            &mut h,
            "im_category_chain_entries",
            json!({"channelId":cid(1),"chainId":"chain-a","categoryId":category,"req_id":"page"}),
            page.clone(),
            &mut store,
        );
        assert_eq!(events[0]["data"]["body"]["data"], page);
    }
}

/// 发布入口禁止UI注入临时消息身份，失败不落库也不发HTTP。
#[test]
fn category_publish_rejects_ui_temporary_identity() {
    use helix_core::Module;
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let command=helix_im::client_api::build_command("im_category_chain_publish",&serde_json::to_vec(&json!({"channelId":cid(1),"chainId":"chain-a","expectedDefinitionRevision":"1","clientMutationId":"m","temporaryId":"ui-owned"})).unwrap()).unwrap();
    assert!(h
        .module
        .handle(&Tick::Command(command), 1000, &mut h.sink)
        .is_err());
    assert!(h.sink.as_slice().is_empty());
}

/// 完整定义fixture用于草稿和get分支，不从客户端输入补服务端身份。
fn definition(revision: &str, status: &str) -> Value {
    let mut chain = json!({"id":"chain-a","channelId":cid(1),"anchorPostId":if status=="DRAFT" {Value::Null} else {json!(common::sid(1))},"creatorId":"user-a","title":"活动","description":"","example":"","deadlineAt":1789113600000_i64,"status":status,"closeReason":null,"revision":revision,"definitionRevision":"1","createdAt":1788770000000_i64,"updatedAt":1788770000000_i64});
    if status != "RETRACTED" {
        chain["maxCategories"] = json!(2);
    }
    chain
}

/// 草稿没有公共Post，私有分类和audience只进入operation。
#[test]
fn category_draft_create_update_are_durable_without_public_posts() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let mut store = Store::new();
    for (command, kind, revision) in [
        ("im_category_chain_create_draft", "create-draft", "1"),
        ("im_category_chain_update_draft", "update-draft", "2"),
    ] {
        let mut args = json!({"channelId":cid(1),"title":"活动","description":"","example":"","deadlineAt":1789113600000_i64,"audience":{"mode":"CHANNEL_SNAPSHOT"},"categories":[{"clientId":"a","name":"室内"},{"clientId":"b","name":"户外"}],"max_categories":2,"clientMutationId":kind});
        if kind == "update-draft" {
            args["chainId"] = json!("chain-a");
            args["expectedDefinitionRevision"] = json!("1");
        }
        let draft = json!({"chain":definition(revision,"DRAFT"),"categories":[{"id":"a","chainId":"chain-a","name":"室内","ordinal":0,"tone":"purple","revision":"1"},{"id":"b","chainId":"chain-a","name":"户外","ordinal":1,"tone":"blue","revision":"1"}],"audience":{"chainId":"chain-a","mode":"CHANNEL_SNAPSHOT","memberIds":["user-a"],"revision":"1"},"categoryIdMap":[{"clientId":"a","categoryId":"a"},{"clientId":"b","categoryId":"b"}]});
        let data = json!({"outcome":"CONFIRMED","commandKind":kind,"clientMutationId":kind,"operationId":format!("op-{kind}"),"chainId":"chain-a","revision":revision,"eventId":null,"eventSeq":null,"draft":draft,"card":null,"post":null,"myParticipation":null});
        if kind == "create-draft" {
            assert!(
                args.get("chainId").is_none(),
                "first create has no existing chain identity"
            );
        }
        let receipt = data.clone();
        let events = query(&mut h, command, args, data, &mut store);
        let event = events
            .iter()
            .find(|e| e["event"] == "im:category_chain:operation")
            .unwrap();
        assert_eq!(event["data"]["state"], "CONFIRMED", "{events:?}");
        assert_eq!(event["data"]["draft"]["chain"]["revision"], revision);
        assert_private_operation(event, Some(&receipt));
        if kind == "create-draft" {
            operation_fixture("create-draft", event);
        }
        assert!(!events
            .iter()
            .any(|e| e["event"] == "im:category_chain:projection"
                || e["event"] == "im:post:received"));
        assert!(!store.contains_key("message"));
    }
}

/// Draft、HTTP mutation、持久 read-back 与 WS 都传递同一显式上限，不为旧值补默认。
#[test]
fn category_max_categories_projection_is_shared_across_draft_http_ws_and_readback() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let mut store = Store::new();
    let args = json!({
        "channelId": cid(1), "title": "活动", "description": "", "example": "",
        "deadlineAt": 1789113600000_i64, "audience": {"mode": "CHANNEL_SNAPSHOT"},
        "categories": [{"clientId": "a", "name": "室内"}, {"clientId": "b", "name": "户外"}],
        "max_categories": 2, "clientMutationId": "max-draft"
    });
    let mut draft = json!({
        "chain": definition("9007199254740993", "DRAFT"),
        "categories": [
            {"id":"a","chainId":"chain-a","name":"室内","ordinal":0,"tone":"purple","revision":"1"},
            {"id":"b","chainId":"chain-a","name":"户外","ordinal":1,"tone":"blue","revision":"1"}
        ],
        "audience":{"chainId":"chain-a","mode":"CHANNEL_SNAPSHOT","memberIds":["user-a"],"revision":"1"},
        "categoryIdMap":[{"clientId":"a","categoryId":"a"},{"clientId":"b","categoryId":"b"}]
    });
    draft["chain"]["maxCategories"] = json!(2);
    let receipt = json!({"outcome":"CONFIRMED","commandKind":"create-draft","clientMutationId":"max-draft","operationId":"op-max-draft","chainId":"chain-a","revision":"9007199254740993","eventId":null,"eventSeq":null,"draft":draft,"card":null,"post":null,"myParticipation":null});
    let events = query(
        &mut h,
        "im_category_chain_create_draft",
        args,
        receipt,
        &mut store,
    );
    let draft_event = events
        .iter()
        .find(|event| event["event"] == "im:category_chain:operation")
        .unwrap();
    assert_eq!(draft_event["data"]["draft"]["chain"]["maxCategories"], 2);

    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let mut data = mutation("9007199254740993", "max-public");
    data["card"]["maxCategories"] = json!(2);
    data["post"]["props"]["categoryChain"] = data["card"].clone();
    data["myParticipation"]["maxCategories"] = json!(2);
    let events = query(
        &mut h,
        "im_category_chain_set_participation",
        json!({"channelId":cid(1),"chainId":"chain-a","expectedParticipationRevision":"0","selections":[],"clientMutationId":"max-public"}),
        data.clone(),
        &mut store,
    );
    let operation = events
        .iter()
        .find(|event| event["event"] == "im:category_chain:operation")
        .unwrap();
    assert_eq!(operation["data"]["card"]["maxCategories"], 2);
    assert_eq!(operation["data"]["myParticipation"]["maxCategories"], 2);
    let row = store["message"].values().next().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(row["props"].as_str().unwrap()).unwrap()["categoryChain"],
        data["card"]
    );

    let frame = json!({"action":"im:category_chain:projection","data":{"schemaVersion":1,"channelId":cid(1),"chainId":"chain-a","revision":"9007199254740993","eventId":"max-event","eventSeq":"1","anchorPostId":common::sid(1),"temporaryId":"category-anchor-1","card":data["card"]}});
    h.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&frame).unwrap(),
        ))),
        1200,
    );
    let events = drain(&mut h, &mut store);
    let projection = events
        .iter()
        .find(|event| event["event"] == "im:category_chain:projection")
        .unwrap();
    assert_eq!(projection["data"]["card"]["maxCategories"], 2);
}

/// 新字段只在三种指定投影中可选；非法、超范围和撤回态均在权威输入边界拒绝。
#[test]
fn category_max_categories_projection_rejects_invalid_and_retracted_values() {
    use helix_core::Module;

    for (name, max_categories, retracted) in [
        ("null", Value::Null, false),
        ("string", json!("2"), false),
        ("fraction", json!(1.5), false),
        ("over", json!(3), false),
        ("retracted", json!(1), true),
    ] {
        let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
        let mut invalid = card("9007199254740993");
        invalid["maxCategories"] = max_categories;
        if retracted {
            invalid["status"] = json!("RETRACTED");
            invalid["title"] = json!("分类接龙已撤回");
            invalid["categories"] = json!([]);
            invalid["categoryCount"] = json!(0);
            invalid["participantCount"] = json!(0);
            invalid["audienceCount"] = json!(0);
        }
        let frame = json!({"action":"im:category_chain:projection","data":{"schemaVersion":1,"channelId":cid(1),"chainId":"chain-a","revision":"9007199254740993","eventId":format!("bad-{name}"),"eventSeq":"1","anchorPostId":common::sid(1),"temporaryId":"category-anchor-1","card":invalid}});
        h.module
            .handle(
                &Tick::Inbound(InboundBytes(Bytes::from(
                    serde_json::to_vec(&frame).unwrap(),
                ))),
                1000,
                &mut h.sink,
            )
            .unwrap();
        assert!(h.sink.as_slice().is_empty(), "{name}");
    }
}

/// Card-only WS之后同版本get仍能补齐独立chain/Post事实。
#[test]
fn category_get_after_card_only_ws_preserves_complete_result() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let mut store = Store::new();
    let frame = json!({"action":"im:category_chain:projection","data":{"schemaVersion":1,"channelId":cid(1),"chainId":"chain-a","revision":"1","eventId":"e1","eventSeq":"1","anchorPostId":common::sid(1),"temporaryId":"category-anchor-1","card":card("1")}});
    h.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&frame).unwrap(),
        ))),
        1000,
    );
    drain(&mut h, &mut store);
    let result =
        json!({"chain":definition("1","OPEN"),"card":card("1"),"draft":null,"post":post("1")});
    let events = query(
        &mut h,
        "im_category_chain_get",
        json!({"channelId":cid(1),"chainId":"chain-a","req_id":"get"}),
        result.clone(),
        &mut store,
    );
    assert_eq!(events[0]["data"]["body"]["data"], result, "{events:?}");
    assert!(store.get("message").is_some_and(|rows| rows.len() == 1));
}

/// close/retract只更新原锚点；撤回后迟到分页不能恢复正文。
#[test]
fn category_close_retract_and_late_private_reads_fail_closed() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let mut store = Store::new();
    for (kind, status, revision) in [("close", "CLOSED", "2"), ("retract", "RETRACTED", "3")] {
        let mut canonical = card(revision);
        canonical["status"] = json!(status);
        if status == "RETRACTED" {
            canonical["title"] = json!("分类接龙已撤回");
            canonical["categories"] = json!([]);
            canonical.as_object_mut().unwrap().remove("maxCategories");
            for key in ["categoryCount", "participantCount", "audienceCount"] {
                canonical[key] = json!(0);
            }
        }
        let mut data = mutation(revision, kind);
        data["commandKind"] = json!(kind);
        data["myParticipation"] = Value::Null;
        data["card"] = canonical.clone();
        data["post"]["props"]["categoryChain"] = canonical;
        let events = query(
            &mut h,
            &format!("im_category_chain_{kind}"),
            json!({"channelId":cid(1),"chainId":"chain-a","clientMutationId":kind,"expectedRevision":"1"}),
            data,
            &mut store,
        );
        assert!(
            events
                .iter()
                .any(|e| e["event"] == "im:category_chain:operation"
                    && e["data"]["state"] == "CONFIRMED"),
            "{events:?}"
        );
        assert!(!events.iter().any(|e| e["event"] == "im:post:received"));
        assert_eq!(store["message"].len(), 1);
    }
    let page = json!({"chainId":"chain-a","categoryId":"a","categoryRevision":"1","entries":[],"nextCursor":null,"hasMore":false});
    let events = query(
        &mut h,
        "im_category_chain_entries",
        json!({"channelId":cid(1),"chainId":"chain-a","categoryId":"a","req_id":"late-page"}),
        page,
        &mut store,
    );
    assert!(events[0]["data"]["body"].is_null(), "{events:?}");
    assert!(events[0]["data"]["error"].to_string().contains("RETRACTED"));
}

/// 同人多类按服务端完整集合替换；取消空集合会过滤旧revision的参与尾行。
#[test]
fn category_multiple_selections_then_cancel_keep_one_anchor_and_clear_private_entries() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let mut store = Store::new();
    for (revision, id) in [("1", "multi"), ("2", "cancel")] {
        let selections = if id == "multi" {
            json!([{"categoryId":"a","content":"室内参加"},{"categoryId":"b","content":"户外参加"}])
        } else {
            json!([])
        };
        let args = json!({"channelId":cid(1),"chainId":"chain-a","expectedParticipationRevision":if id=="multi" {"0"} else {"1"},"selections":selections,"clientMutationId":id});
        let command = helix_im::client_api::build_command(
            "im_category_chain_set_participation",
            &serde_json::to_vec(&args).unwrap(),
        )
        .unwrap();
        h.step(Tick::Command(command), 1000);
        assert_eq!(h.last_http_body().unwrap()["selections"], selections);
        let corr = h
            .sink
            .as_slice()
            .iter()
            .find_map(|e| match e {
                Effect::Http { corr, .. } => Some(*corr),
                _ => None,
            })
            .unwrap();
        let mut data = mutation(revision, id);
        data["myParticipation"]["participationRevision"] = json!(revision);
        if id == "multi" {
            data["myParticipation"]["entries"]=json!(["a","b"].map(|category|json!({"id":format!("entry-{category}"),"chainId":"chain-a","categoryId":category,"userId":"user-a","content":"参加","position":"9007199254740993","revision":"1","status":"ACTIVE","createdAt":1788770000000_i64,"updatedAt":1788770000000_i64})));
        }
        h.step(
            Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(http_bytes(
                    serde_json::to_vec(&json!({"status":"SUCCESS","data":data})).unwrap(),
                ))),
            },
            1001,
        );
        let events = drain(&mut h, &mut store);
        let event = events
            .iter()
            .find(|e| e["event"] == "im:category_chain:operation")
            .unwrap();
        assert_eq!(event["data"]["state"], "CONFIRMED", "{events:?}");
        assert_eq!(
            event["data"]["myParticipation"]["entries"]
                .as_array()
                .unwrap()
                .len(),
            if id == "multi" { 2 } else { 0 }
        );
        assert!(!events.iter().any(|e| e["event"] == "im:post:received"));
        assert_eq!(store["message"].len(), 1);
    }
}

/// 普通消息流先落新Post但分类head尚未到，旧HTTP也不能把消息正文倒退。
#[test]
fn category_http_cannot_overwrite_newer_canonical_message_without_category_head() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let mut store = Store::new();
    let args = |id| json!({"channelId":cid(1),"chainId":"chain-a","expectedParticipationRevision":"0","selections":[],"clientMutationId":id});
    query(
        &mut h,
        "im_category_chain_set_participation",
        args("new"),
        mutation("10", "new"),
        &mut store,
    );
    store.remove("category_chain_head");
    store.remove("category_chain_fact");
    let before = store["message"].clone();
    let events = query(
        &mut h,
        "im_category_chain_set_participation",
        args("old"),
        mutation("9", "old"),
        &mut store,
    );
    assert_eq!(
        store["message"], before,
        "old HTTP overwrote canonical message"
    );
    assert!(
        events
            .iter()
            .any(|e| e["event"] == "im:category_chain:operation"
                && e["data"]["state"] == "RECONCILING"),
        "{events:?}"
    );
    assert!(!events.iter().any(|e| e["event"] == "im:post:updated"));
}

/// Card-only WS也必须尊重普通消息流已经持久化的更高版本。
#[test]
fn category_ws_cannot_emit_older_card_than_canonical_message_without_category_head() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let mut store = Store::new();
    query(
        &mut h,
        "im_category_chain_set_participation",
        json!({"channelId":cid(1),"chainId":"chain-a","expectedParticipationRevision":"0","selections":[],"clientMutationId":"new"}),
        mutation("10", "new"),
        &mut store,
    );
    store.remove("category_chain_head");
    store.remove("category_chain_fact");
    let frame = json!({"action":"im:category_chain:projection","data":{"schemaVersion":1,"channelId":cid(1),"chainId":"chain-a","revision":"9","eventId":"old","eventSeq":"1","anchorPostId":common::sid(1),"temporaryId":"category-anchor-1","card":card("9")}});
    h.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&frame).unwrap(),
        ))),
        1002,
    );
    let mut rejected = false;
    for _ in 0..32 {
        assert!(
            !h.sink
                .as_slice()
                .iter()
                .any(|effect| matches!(effect, Effect::Emit { .. })),
            "stale card emitted"
        );
        let (corr, rows) = h
            .sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, ops } | Effect::PersistAtomic { corr, ops } => {
                    Some((*corr, storage_reply(ops, &mut store)))
                }
                _ => None,
            })
            .expect("durable continuation");
        h.sink.clear();
        let result = helix_core::Module::handle(
            &mut h.module,
            &Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(
                    serde_json::to_vec(&rows).unwrap(),
                ))),
            },
            1100,
            &mut h.sink,
        );
        if let Err(error) = result {
            assert!(
                error.to_string().contains("STALE_MESSAGE_RECONCILE"),
                "{error}"
            );
            rejected = true;
            break;
        }
    }
    assert!(
        rejected,
        "older card must be rejected at the durable boundary"
    );
    assert!(!store.contains_key("category_chain_head"));
    let mut fresh = frame;
    fresh["data"]["revision"] = json!("10");
    fresh["data"]["card"] = card("10");
    h.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&fresh).unwrap(),
        ))),
        1101,
    );
    assert!(
        drain(&mut h, &mut store)
            .iter()
            .any(|event| event["event"] == "im:category_chain:projection"),
        "rejection must release the queue for a current card"
    );
}

/// Verify captured routing context and byte-decoded preservation of the complete Go receipt.
fn assert_private_operation(event: &Value, receipt: Option<&Value>) {
    assert_eq!(event["event"], "im:category_chain:operation");
    assert_eq!(event["data"]["channelId"], cid(1));
    assert_eq!(event["data"]["viewerId"], "user-a");
    if let Some(receipt) = receipt {
        assert!(receipt.get("channelId").is_none());
        assert!(receipt.get("viewerId").is_none());
        let mut actual = event["data"].clone();
        for key in ["channelId", "viewerId", "command", "state", "req_id"] {
            actual.as_object_mut().unwrap().remove(key);
        }
        assert_eq!(
            &actual, receipt,
            "private context must not truncate or wrap the Go receipt"
        );
    }
}

/// Only an explicit update run writes canonical fixtures; ordinary tests compare actual Emit JSON.
fn operation_fixture(name: &str, event: &Value) {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/fixtures/category-chain-operation");
    let path = directory.join(format!("{name}.json"));
    let mut bytes = serde_json::to_vec_pretty(event).unwrap();
    bytes.push(b'\n');
    if std::env::var_os("HELIX_UPDATE_CATEGORY_OPERATION_FIXTURES").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&path, &bytes).unwrap();
    }
    assert_eq!(
        std::fs::read(&path).expect("explicit fixture generation required"),
        bytes,
        "fixture drift: {}",
        path.display()
    );
}

/// Untrusted reply routing fields cannot replace the context captured by Helix.
#[test]
fn category_operation_rejects_reply_viewer_override() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let corr = set(&mut h, "untrusted-viewer");
    let mut receipt = mutation("1", "untrusted-viewer");
    receipt["viewerId"] = json!("different-user");
    h.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(http_bytes(
                serde_json::to_vec(&json!({"status":"SUCCESS","data":receipt})).unwrap(),
            ))),
        },
        1001,
    );
    let events = drain(&mut h, &mut Store::new());
    let operation = events
        .iter()
        .find(|e| e["event"] == "im:category_chain:operation")
        .unwrap();
    assert_private_operation(operation, None);
    assert_eq!(operation["data"]["state"], "RECONCILING");
}

/// Go snapshots missing user profiles as two empty strings; populated previews expose that contract.
fn go_profile_receipt(id: &str) -> Value {
    let mut data = mutation("4", id);
    data["eventId"] = json!("go-event-4");
    data["eventSeq"] = json!("4");
    data["myParticipation"]["participationRevision"] = json!("1");
    let entries: Vec<Value> = ["a", "b"]
        .into_iter()
        .map(|category| {
            json!({"id":format!("entry-{category}"),"chainId":"chain-a","categoryId":category,
            "userId":"user-a","content":"fixture participation","position":"1","revision":"1",
            "status":"ACTIVE","createdAt":1788770000000_i64,"updatedAt":1788770000000_i64})
        })
        .collect();
    for (index, entry) in entries.iter().enumerate() {
        data["card"]["categories"][index]["hasMore"] = json!(false);
        data["card"]["categories"][index]["entries"] = json!([{
            "entryId":entry["id"],"userId":entry["userId"],"displayName":"","departmentName":"",
            "content":entry["content"],"createdAt":entry["createdAt"],"updatedAt":entry["updatedAt"],"displayNumber":1
        }]);
    }
    data["myParticipation"]["entries"] = json!(entries);
    data["post"]["props"]["categoryChain"] = data["card"].clone();
    data
}

/// Replay the real same-user/two-category response: no ACK until storage readback, no duplicate ACK.
#[test]
fn category_go_profile_empty_snapshot_commits_full_participation() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let command = helix_im::client_api::build_command("im_category_chain_set_participation",
        &serde_json::to_vec(&json!({"channelId":cid(1),"chainId":"chain-a",
            "expectedParticipationRevision":"0","selections":[{"categoryId":"a","content":"fixture participation"},
                {"categoryId":"b","content":"fixture participation"}],"clientMutationId":"go-profile"})).unwrap()).unwrap();
    h.step(Tick::Command(command), 1000);
    let corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .unwrap();
    let data = go_profile_receipt("go-profile");
    let response =
        http_bytes(serde_json::to_vec(&json!({"status":"SUCCESS","data":data})).unwrap());
    h.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(response.clone())),
        },
        1001,
    );
    let mut store = Store::new();
    assert!(
        !h.sink
            .as_slice()
            .iter()
            .any(|e| matches!(e, Effect::Emit { .. })),
        "HTTP must wait for durable facts: {:?}",
        h.sink.as_slice()
    );
    let events = drain(&mut h, &mut store);
    let operations: Vec<_> = events
        .iter()
        .filter(|e| e["event"] == "im:category_chain:operation")
        .collect();
    assert_eq!(operations.len(), 1);
    let operation = &operations[0]["data"];
    assert_eq!(operation["state"], "CONFIRMED");
    assert_eq!(operation["card"], data["card"]);
    assert_eq!(operation["myParticipation"], data["myParticipation"]);
    assert_eq!(operation["viewerId"], "user-a");
    assert_eq!(store["message"].len(), 1);
    let row = store["message"].values().next().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(row["props"].as_str().unwrap()).unwrap()["categoryChain"],
        data["card"]
    );
    h.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(response)),
        },
        1200,
    );
    assert!(drain(&mut h, &mut store).is_empty());
}

/// The same Go preview crosses private HTTP pages and public WS without synthesized author names.
#[test]
fn category_go_profile_empty_snapshot_survives_page_and_ws() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let mut store = Store::new();
    let data = go_profile_receipt("page-ws");
    let page = json!({"chainId":"chain-a","categoryId":"a","categoryRevision":"4",
        "entries":data["card"]["categories"][0]["entries"],"nextCursor":null,"hasMore":false});
    let events = query(
        &mut h,
        "im_category_chain_entries",
        json!({"channelId":cid(1),"chainId":"chain-a","categoryId":"a","req_id":"page"}),
        page.clone(),
        &mut store,
    );
    assert_eq!(events[0]["data"]["body"]["data"], page);
    // Channel cursor starts at zero; category revision and channel event sequence are independent.
    let frame = json!({"action":"im:category_chain:projection","data":{"schemaVersion":1,"channelId":cid(1),
        "chainId":"chain-a","revision":"4","eventId":"go-event-4","eventSeq":"1",
        "anchorPostId":common::sid(1),"temporaryId":"category-anchor-1","card":data["card"]}});
    h.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&frame).unwrap(),
        ))),
        1200,
    );
    let events = drain(&mut h, &mut store);
    assert_eq!(
        events
            .iter()
            .filter(|e| e["event"] == "im:category_chain:projection")
            .count(),
        1
    );
    assert_eq!(events[0]["data"]["card"], data["card"]);
}

/// Accepting empty profile strings never admits malformed fields, forged viewers, or divergent cards.
#[test]
fn category_go_profile_invalid_canonical_still_fails_closed() {
    for case in [
        "missing",
        "null",
        "number",
        "overlong",
        "empty-content",
        "viewer",
        "revision",
        "card-post",
    ] {
        let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
        let corr = set(&mut h, "invalid-profile");
        let mut data = go_profile_receipt("invalid-profile");
        let entry = &mut data["card"]["categories"][0]["entries"][0];
        match case {
            "missing" => {
                entry.as_object_mut().unwrap().remove("displayName");
            }
            "null" => entry["displayName"] = Value::Null,
            "number" => entry["displayName"] = json!(7),
            "overlong" => entry["displayName"] = json!("x".repeat(2001)),
            "empty-content" => entry["content"] = json!(""),
            "viewer" => data["myParticipation"]["entries"][0]["userId"] = json!("another-viewer"),
            "revision" => data["card"]["revision"] = json!("04"),
            "card-post" => data["post"]["props"]["categoryChain"]["title"] = json!("divergent"),
            _ => unreachable!(),
        }
        if case != "card-post" {
            data["post"]["props"]["categoryChain"] = data["card"].clone();
        }
        h.step(
            Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(http_bytes(
                    serde_json::to_vec(&json!({"status":"SUCCESS","data":data})).unwrap(),
                ))),
            },
            1001,
        );
        assert!(
            !h.sink
                .as_slice()
                .iter()
                .any(|e| matches!(e, Effect::Persist { .. } | Effect::PersistAtomic { .. })),
            "{case}"
        );
        let events = drain(&mut h, &mut Store::new());
        assert_eq!(events.len(), 1, "{case}");
        assert_eq!(events[0]["data"]["state"], "RECONCILING", "{case}");
        assert!(events[0]["data"]["errorCode"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
    }
}

/// Drive only the real pre-HTTP storage barrier (publish identity); never synthesize authority.
fn category_mode_http(alias: &str, args: Value) -> (String, Value) {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let cmd =
        helix_im::client_api::build_command(alias, &serde_json::to_vec(&args).unwrap()).unwrap();
    h.step(Tick::Command(cmd), 1000);
    let mut store = Store::new();
    for _ in 0..4 {
        assert!(!h
            .sink
            .as_slice()
            .iter()
            .any(|e| matches!(e, Effect::Emit { .. })));
        if let Some((url, body)) = h.sink.as_slice().iter().find_map(|e| match e {
            Effect::Http { req, .. } => Some((
                req.url.to_string(),
                serde_json::from_slice(req.body.as_ref().unwrap()).unwrap(),
            )),
            _ => None,
        }) {
            return (url, body);
        }
        let (corr, body) = h
            .sink
            .as_slice()
            .iter()
            .find_map(|e| match e {
                Effect::Persist { corr, ops } | Effect::PersistAtomic { corr, ops } => {
                    Some((*corr, storage_reply(ops, &mut store)))
                }
                _ => None,
            })
            .expect("command must dispatch HTTP or its publish identity barrier");
        h.step(
            Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(
                    serde_json::to_vec(&body).unwrap(),
                ))),
            },
            1001,
        );
    }
    panic!("publish identity did not lead to HTTP");
}

/// All eleven semantic commands share post-chain transport; get views and string CAS stay distinct.
#[test]
fn post_chain_mode_v3_maps_all_category_commands() {
    let scope = json!({"channelId":cid(1),"chainId":"chain-a","req_id":"read-mode"});
    let draft = json!({"channelId":cid(1),"title":"活动","description":"","example":"",
        "deadlineAt":1789113600000_i64,"audience":{"mode":"CHANNEL_SNAPSHOT"},
        "categories":[{"clientId":"a","name":"室内"},{"clientId":"b","name":"户外"}],"max_categories":2,"clientMutationId":"draft-mode"});
    let mut update = draft.clone();
    update["chainId"] = json!("chain-a");
    update["expectedDefinitionRevision"] = json!("9007199254740993");
    let cases = [
        (
            "capabilities",
            json!({"req_id":"cap-mode"}),
            "capabilities",
            None,
        ),
        ("create_draft", draft, "create", None),
        ("update_draft", update, "update-draft", None),
        (
            "publish",
            json!({"channelId":cid(1),"chainId":"chain-a","expectedDefinitionRevision":"9007199254740993","clientMutationId":"publish-mode"}),
            "publish",
            None,
        ),
        ("get", scope.clone(), "get", Some("card")),
        ("get_mine", scope.clone(), "get", Some("mine")),
        (
            "entries",
            json!({"channelId":cid(1),"chainId":"chain-a","categoryId":"a","cursor":"opaque:cursor","limit":5,"req_id":"entries-mode"}),
            "get",
            Some("entries"),
        ),
        (
            "set_participation",
            json!({"channelId":cid(1),"chainId":"chain-a","expectedParticipationRevision":"9007199254740993","selections":[{"categoryId":"a","content":"一"},{"categoryId":"b","content":"二"}],"clientMutationId":"set-mode"}),
            "append",
            None,
        ),
        (
            "close",
            json!({"channelId":cid(1),"chainId":"chain-a","expectedRevision":"9007199254740993","clientMutationId":"close-mode"}),
            "close",
            None,
        ),
        (
            "retract",
            json!({"channelId":cid(1),"chainId":"chain-a","expectedRevision":"9007199254740993","clientMutationId":"retract-mode"}),
            "retract",
            None,
        ),
        (
            "reconcile",
            json!({"channelId":cid(1),"chainId":"chain-a","clientMutationId":"set-mode","req_id":"reconcile-mode"}),
            "reconcile",
            None,
        ),
    ];
    for (suffix, args, route, view) in cases {
        let (url, body) = category_mode_http(&format!("im_category_chain_{suffix}"), args);
        assert!(
            url.ends_with(&format!("/post-chain/{route}?mode=CATEGORY")),
            "{suffix}: {url}"
        );
        assert_eq!(body["mode"], "CATEGORY", "{suffix}");
        assert_eq!(body.get("view").and_then(Value::as_str), view, "{suffix}");
        assert!(body.get("req_id").is_none() && body.get("reqId").is_none());
        if suffix == "capabilities" {
            assert_eq!(body, json!({"mode":"CATEGORY"}));
        }
        if suffix == "get" || suffix == "get_mine" {
            assert_eq!(
                body,
                json!({"mode":"CATEGORY","view":view,"channelId":cid(1),"chainId":"chain-a"})
            );
        }
        if suffix == "entries" {
            assert_eq!(
                body,
                json!({"mode":"CATEGORY","view":"entries","channelId":cid(1),"chainId":"chain-a","categoryId":"a","cursor":"opaque:cursor","limit":5})
            );
        }
        if suffix == "update_draft" || suffix == "publish" {
            assert_eq!(body["expectedDefinitionRevision"], "9007199254740993");
        }
        if suffix == "set_participation" {
            assert_eq!(body["expectedParticipationRevision"], "9007199254740993");
            assert_eq!(
                body["selections"],
                json!([{"categoryId":"a","content":"一"},{"categoryId":"b","content":"二"}])
            );
            assert!(body.get("text").is_none() && body.get("attachments").is_none());
        }
        if suffix == "retract" {
            assert!(body.get("entryId").is_none());
        }
    }
    let (_, cancel) = category_mode_http(
        "im_category_chain_set_participation",
        json!({"channelId":cid(1),"chainId":"chain-a","expectedParticipationRevision":"2","selections":[],"clientMutationId":"cancel-mode"}),
    );
    assert_eq!(cancel["selections"], json!([]));
    assert_eq!(cancel["mode"], "CATEGORY");
}

/// Caller fields cannot override the transport-owned mode/view or turn anchor retraction into entry removal.
#[test]
fn post_chain_mode_v3_rejects_category_transport_overrides() {
    use helix_core::Module;
    for (alias, extra) in [
        ("im_category_chain_get", json!({"mode":"TEXT"})),
        ("im_category_chain_get", json!({"mode":null})),
        ("im_category_chain_get", json!({"mode":42})),
        ("im_category_chain_get", json!({"mode":"UNKNOWN"})),
        ("im_category_chain_get", json!({"view":"entries"})),
        ("im_category_chain_get_mine", json!({"categoryId":"a"})),
        (
            "im_category_chain_retract",
            json!({"entryId":"entry-a","expectedRevision":"1","clientMutationId":"wrong-retract"}),
        ),
    ] {
        let mut args = json!({"channelId":cid(1),"chainId":"chain-a","req_id":"invalid-mode"});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let cmd = helix_im::client_api::build_command(alias, &serde_json::to_vec(&args).unwrap())
            .unwrap();
        let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
        assert!(
            h.module
                .handle(&Tick::Command(cmd), 1000, &mut h.sink)
                .is_err(),
            "{alias}"
        );
        assert!(
            !h.sink.as_slice().iter().any(|e| matches!(
                e,
                Effect::Http { .. } | Effect::Persist { .. } | Effect::PersistAtomic { .. }
            )),
            "{alias}"
        );
    }
    assert!(helix_im::client_api::build_command("im_category_chain_mark_read", b"{}").is_err());
}

/// Legacy TEXT payloads omit mode outside create; all nine endpoints and numeric revisions are unchanged.
#[test]
fn post_chain_mode_v3_preserves_text_nine_actions_without_mode() {
    for (suffix, extra, expected_extra) in [
        (
            "create",
            json!({"title":"TEXT"}),
            json!({"title":"TEXT","mode":"TEXT"}),
        ),
        (
            "update_draft",
            json!({"expectedRevision":3}),
            json!({"expectedRevision":3}),
        ),
        (
            "publish",
            json!({"expectedRevision":3}),
            json!({"expectedRevision":3}),
        ),
        (
            "append",
            json!({"text":"entry"}),
            json!({"text":"entry","attachments":[]}),
        ),
        (
            "get",
            json!({"middleCursor":"old-cursor","limit":10}),
            json!({"middleCursor":"old-cursor","limit":10}),
        ),
        (
            "close",
            json!({"expectedRevision":3}),
            json!({"expectedRevision":3}),
        ),
        (
            "retract",
            json!({"entryId":"entry-a","expectedRevision":3}),
            json!({"entryId":"entry-a","expectedRevision":3}),
        ),
        ("reconcile", json!({}), json!({})),
        (
            "mark_read",
            json!({"lastEventSeq":7,"lastReadAt":1000}),
            json!({"lastEventSeq":7,"lastReadAt":1000}),
        ),
    ] {
        let mut args =
            json!({"channelId":cid(1),"chainId":"text-chain","clientMutationId":"text-mutation"});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        if suffix == "get" {
            args.as_object_mut().unwrap().remove("clientMutationId");
        }
        let cmd = helix_im::client_api::build_command(
            &format!("im_post_chain_{suffix}"),
            &serde_json::to_vec(&args).unwrap(),
        )
        .unwrap();
        let effects = helix_im::outbound::handle_outbound(
            cmd.name.as_ref(),
            &cmd.payload,
            "http://fixture/api/cses",
            "",
            None,
            Correlation::from_raw(1),
        )
        .unwrap();
        let (url, body) = effects
            .iter()
            .find_map(|e| match e {
                Effect::Http { req, .. } => Some((
                    &req.url,
                    serde_json::from_slice::<Value>(req.body.as_ref().unwrap()).unwrap(),
                )),
                _ => None,
            })
            .unwrap();
        let mut expected =
            json!({"channelId":cid(1),"chainId":"text-chain","clientMutationId":"text-mutation"});
        if suffix == "get" {
            expected.as_object_mut().unwrap().remove("clientMutationId");
        }
        expected
            .as_object_mut()
            .unwrap()
            .extend(expected_extra.as_object().unwrap().clone());
        assert_eq!(
            url,
            &format!(
                "http://fixture/api/cses/post-chain/{}",
                suffix.replace('_', "-")
            )
        );
        assert_eq!(body, expected, "{suffix}");
    }
}

/// Loaded-module feature identifies the new transport; canonical registry names are not public aliases.
#[test]
fn post_chain_mode_v3_feature_and_canonical_registration() {
    let caps: Value = serde_json::from_slice(&helix_im::client_api::capabilities_json()).unwrap();
    assert_eq!(caps["features"]["postChainCategoryMode"], true);
    assert_eq!(caps["features"]["postChainCategoryWireVersion"], 3);
    let commands = caps["commands"].as_array().unwrap();
    assert!(commands.iter().any(|c| c == "category_chain_capabilities"));
    assert!(!commands
        .iter()
        .any(|c| c == "im_category_chain_capabilities"));
}

// 回归锚: category-review R2 — permission observations do not share the entry-set revision.
#[test]
/// A fresh authoritative mine read must replace permissions even when entries did not change.
fn category_review_same_participation_revision_refreshes_permissions() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let mut store = Store::new();
    query(
        &mut h,
        "im_category_chain_get_mine",
        json!({"channelId":cid(1),"chainId":"chain-a","req_id":"before"}),
        mine(),
        &mut store,
    );
    let mut closed = mine();
    closed["canParticipate"] = json!(false);
    closed["canEdit"] = json!(false);
    closed["canCancel"] = json!(false);
    closed["denialReason"] = json!("CATEGORY_CHAIN_CLOSED");
    let events = query(
        &mut h,
        "im_category_chain_get_mine",
        json!({"channelId":cid(1),"chainId":"chain-a","req_id":"after"}),
        closed.clone(),
        &mut store,
    );
    assert_eq!(events[0]["data"]["body"]["data"], closed);
}

#[test]
/// Old paths cannot consume the bounded read budget for a new canonical Post revision.
fn category_review_old_fact_paths_cannot_truncate_current_snapshot() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let mut store = Store::new();
    for (revision, key) in [("1", "aa"), ("2", "zz")] {
        let mut result = mutation(revision, key);
        let extra: serde_json::Map<String, Value> = (0..3000)
            .map(|index| (format!("{key}{index:04}"), json!(index)))
            .collect();
        result["post"]["extra"] = Value::Object(extra);
        let expected = result["post"].clone();
        let events = query(
            &mut h,
            "im_category_chain_set_participation",
            json!({"channelId":cid(1),"chainId":"chain-a","expectedParticipationRevision":"0","selections":[],"clientMutationId":key}),
            result,
            &mut store,
        );
        let actual = events
            .iter()
            .find(|event| event["event"] == "im:category_chain:operation")
            .expect("operation");
        assert_eq!(actual["data"]["state"], "CONFIRMED", "{actual}");
        assert_eq!(
            actual["data"]["post"], expected,
            "snapshot lost current fields"
        );
    }
}

/// Build the two authorized category intents; nested profile keys remain camelCase on both entry paths.
fn category_snapshot_args(publish: bool) -> Value {
    if publish {
        json!({"channelId":cid(1),"chainId":"chain-a","expectedDefinitionRevision":"1","clientMutationId":"snapshot"})
    } else {
        json!({"channelId":cid(1),"chainId":"chain-a","expectedParticipationRevision":"0",
            "selections":[{"categoryId":"a","content":"fixture participation"}],"clientMutationId":"snapshot"})
    }
}

/// Enter the public command and consume the publish identity barrier before capturing the outbound body.
fn category_snapshot_start(
    publish: bool,
    snapshot: Option<Value>,
    key: &str,
) -> (Harness, Store, Correlation, Value) {
    let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let mut args = category_snapshot_args(publish);
    if let Some(snapshot) = snapshot {
        args[key] = snapshot;
    }
    let name = if publish {
        "im_category_chain_publish"
    } else {
        "im_category_chain_set_participation"
    };
    let command =
        helix_im::client_api::build_command(name, &serde_json::to_vec(&args).unwrap()).unwrap();
    h.step(Tick::Command(command), 1000);
    let mut store = Store::new();
    assert!(
        drain(&mut h, &mut store).is_empty(),
        "no success before HTTP authority"
    );
    let (corr, body) = h
        .sink
        .as_slice()
        .iter()
        .find_map(|fx| match fx {
            Effect::Http { corr, req } => Some((
                *corr,
                serde_json::from_slice::<Value>(req.body.as_ref().unwrap()).unwrap(),
            )),
            _ => None,
        })
        .expect("category HTTP after identity barrier");
    (h, store, corr, body)
}

/// Display hints cross HTTP unchanged; only returned Go authority can populate the durable preview.
#[test]
fn category_user_snapshot_participation_preserves_hints_and_server_authority() {
    let hint = json!({"userId":"untrusted-hint","userName":"Client Label","deptName":"Client Department","orgName":"Client Organization"});
    let (mut h, mut store, corr, body) =
        category_snapshot_start(false, Some(hint.clone()), "userSnapshot");
    assert_eq!(body["userSnapshot"], hint);
    assert!(body.get("user_snapshot").is_none());
    assert!(body.get("userId").is_none());
    let mut data = go_profile_receipt("snapshot");
    for category in data["card"]["categories"].as_array_mut().unwrap() {
        for entry in category["entries"].as_array_mut().unwrap() {
            entry["displayName"] = json!("Server Label");
            entry["departmentName"] = json!("Server Department");
        }
    }
    data["post"]["props"]["categoryChain"] = data["card"].clone();
    let reply = PortOutcome::Ok(ReplyBytes(http_bytes(
        serde_json::to_vec(&json!({"status":"SUCCESS","data":data})).unwrap(),
    )));
    h.step(
        Tick::PortReply {
            corr,
            outcome: reply.clone(),
        },
        1200,
    );
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|fx| matches!(fx, Effect::Emit { .. })));
    let events = drain(&mut h, &mut store);
    let operation = events
        .iter()
        .find(|event| event["event"] == "im:category_chain:operation")
        .unwrap();
    assert_eq!(
        operation["data"]["card"]["categories"][0]["entries"][0]["displayName"],
        "Server Label"
    );
    assert_eq!(
        operation["data"]["myParticipation"]["entries"][0]["userId"],
        "user-a"
    );
    h.step(
        Tick::PortReply {
            corr,
            outcome: reply,
        },
        1300,
    );
    assert!(
        h.sink.as_slice().is_empty(),
        "late HTTP cannot re-emit or recreate state"
    );
}

/// A publish profile survives the identity barrier, and a rejected HTTP response cannot emit success.
#[test]
fn category_user_snapshot_publish_preserves_profile_and_failure_terminal() {
    let hint = json!({"userName":"Publisher Label","deptName":"","orgName":"Organization"});
    let (mut h, _, corr, body) = category_snapshot_start(true, Some(hint.clone()), "user_snapshot");
    assert_eq!(body["userSnapshot"], hint);
    assert!(body["temporaryId"]
        .as_str()
        .is_some_and(|id| !id.is_empty()));
    h.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Err(PortError::Http(403)),
        },
        1200,
    );
    assert!(h
        .sink
        .as_slice()
        .iter()
        .any(|fx| matches!(fx, Effect::CancelTimer { .. })));
    let emitted: Vec<Value> = h
        .sink
        .as_slice()
        .iter()
        .filter_map(|fx| match fx {
            Effect::Emit { event } => Some(serde_json::from_slice(&event.0).unwrap()),
            _ => None,
        })
        .collect();
    assert!(emitted
        .iter()
        .any(|event| event["data"]["state"] == "REJECTED"));
    assert!(!emitted
        .iter()
        .any(|event| event["data"]["state"] == "CONFIRMED"));
}

/// Missing/null profiles retain legacy wire bodies; an explicitly empty object stays an empty hint.
#[test]
fn category_user_snapshot_optional_compatibility() {
    for publish in [false, true] {
        for snapshot in [None, Some(Value::Null), Some(json!({}))] {
            let empty_object = snapshot.as_ref().is_some_and(Value::is_object);
            let (_, _, _, body) = category_snapshot_start(publish, snapshot, "userSnapshot");
            if empty_object {
                assert_eq!(body["userSnapshot"], json!({}));
            } else {
                assert!(body.get("userSnapshot").is_none());
            }
        }
    }
}

/// Malformed snapshots fail before HTTP, timers or publish identity persistence; authority keys stay rejected.
#[test]
fn category_user_snapshot_invalid_input_leaves_no_effects() {
    use helix_core::Module;
    for publish in [false, true] {
        for snapshot in [
            json!("profile"),
            json!([]),
            json!(true),
            json!({"userName":7}),
            json!({"deptName":null}),
            json!({"orgName":{}}),
            json!({"userId":1}),
            json!({"userName":"x".repeat(2001)}),
            json!({"tenantId":"forged"}),
            json!({"user_name":"wrong case"}),
        ] {
            let mut args = category_snapshot_args(publish);
            args["userSnapshot"] = snapshot;
            let name = if publish {
                "im_category_chain_publish"
            } else {
                "im_category_chain_set_participation"
            };
            let command =
                helix_im::client_api::build_command(name, &serde_json::to_vec(&args).unwrap())
                    .unwrap();
            let mut h = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
            assert!(h
                .module
                .handle(&Tick::Command(command), 1000, &mut h.sink)
                .is_err());
            assert!(
                h.sink.as_slice().is_empty(),
                "invalid input must not register work"
            );
            h.module.on_stop(&mut h.sink).unwrap();
            assert!(
                h.sink.as_slice().is_empty(),
                "invalid input left category resources"
            );
        }
    }
}
