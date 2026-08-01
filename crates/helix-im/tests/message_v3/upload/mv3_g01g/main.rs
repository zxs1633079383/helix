//! MV3-G01g：发送 TEMPLATE/IMAGE。
//!
//! 本 Gate 最容易实现错的一条是**回落**：把「TEMPLATE 壳 + 内芯 IMAGE」实现成普通 IMAGE
//! 消息，或把内芯 `type` 覆写成默认的 `TEXT`。两者都会静默丢掉 TEMPLATE 壳承载的
//! 「👌🏻 收到」回执能力，而普通断言（type 正确、图片能显示）仍然全绿。
//! 因此下面有**独立的负向测试组**（`template_must_not_fall_back_to_*`）专门盯这件事，
//! 并配一条对照测试证明「内芯默认值是 TEXT」不是被硬编成 IMAGE。
//!
//! 线型权威 = `specs/003-messagev3-gate-inventory/gates/upload/mv3-g01g/*.json`。

use helix_core::effect::{Effect, SqlValue, StorageOp};
use serde_json::{json, Map, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{command_json, port_reply_ok, EffectKind, Harness};
use helix_im::outbound::posts::props::{
    build_media_message_props, strip_host_private_media_fields, HOST_PRIVATE_MEDIA_FIELDS,
};

const NOW_MS: u64 = 1_700_000_000_000;
const GATE_DIR: &str = "upload/mv3-g01g";
const AUTH_USER_ID: &str = "user-actor-444";

// ── 参考包读取 ───────────────────────────────────────────────────────────────

/// 读取本 Gate 参考包内的一份 JSON。
fn reference(file_name: &str) -> Value {
    phase1_reference::read_json(
        &phase1_reference::reference_root()
            .join(GATE_DIR)
            .join(file_name),
    )
}

/// `inbound-sample.json` 里唯一的 `Tick::Command` 载荷（Angular 提交的最小意图）。
fn reference_command_payload() -> Value {
    reference("inbound-sample.json")["rust_entry_inputs"]
        .as_array()
        .expect("rust_entry_inputs must be an array")
        .iter()
        .find(|entry| entry["tick_variant"] == "Tick::Command")
        .map(|entry| entry["value"]["payload"].clone())
        .expect("inbound sample must carry the TEMPLATE send tick")
}

/// 唯一一条 Host 暂存结果。
fn reference_stage_result() -> Value {
    reference("inbound-sample.json")["host_stage_results"][0].clone()
}

/// 参考包声明的最终 props（线型权威）。
fn reference_final_props() -> Value {
    reference("outbound-sample.json")["samples"]
        .as_array()
        .expect("samples must be an array")
        .iter()
        .find(|sample| {
            sample["viewer"] == "sender" && sample["rust"]["event"] == "im:post:received"
        })
        .map(|sample| sample["rust"]["data"]["props"].clone())
        .expect("missing sender im:post:received sample")
}

// ── Host 边界模拟 ────────────────────────────────────────────────────────────

/// 模拟 `media_stage::resolve_send_payload`：opaque handle → Host 可信 mediaInput + 提取元数据。
fn host_resolved_command() -> Value {
    let mut payload = reference_command_payload();
    let media_handle = payload["props"]["template"]["file"]["mediaHandle"]
        .as_str()
        .expect("Angular 只提交 opaque mediaHandle")
        .to_string();
    let stage = reference_stage_result();

    let mut resolved = Map::new();
    resolved.insert(
        "mediaInput".to_string(),
        json!({
            "localPath": format!("/host-private/staged/{media_handle}"),
            "fileName": stage["file_name"],
            "contentType": stage["content_type"],
            "size": stage["size"],
            "sha256": stage["sha256"],
        }),
    );
    for key in ["width", "height"] {
        if let Some(value) = stage["metadata"].get(key) {
            resolved.insert(key.to_string(), value.clone());
        }
    }
    payload["props"]["template"]["file"] = Value::Object(resolved);
    payload["channel_id"] = json!(common::cid(1));
    payload
}

/// 跑「命令 → optimistic Persist → PersistOk 续接」两拍。
fn run_send(payload: Value) -> Harness {
    let mut harness = Harness::new_with_identity(&[1], AUTH_USER_ID, "company-phase1", "", "", "");
    let first = harness.step(command_json("im_send_message", payload), NOW_MS);
    let corr = match first.as_slice() {
        [EffectKind::Persist(corr)] | [EffectKind::PersistAtomic(corr)] => *corr,
        other => panic!("expected a single optimistic persist, got {other:?}"),
    };
    harness.step(port_reply_ok(corr), NOW_MS + 1);
    harness
}

/// 解码当前 sink 中的指定 MessageV3 事件。
fn emitted_event(harness: &Harness, name: &str) -> Option<Value> {
    harness.sink.as_slice().iter().find_map(|effect| {
        let Effect::Emit { event } = effect else {
            return None;
        };
        let value = serde_json::from_slice::<Value>(event.0.as_ref()).ok()?;
        (value["event"] == name).then_some(value)
    })
}

/// 读取 optimistic message 写集中的某一列。
fn persisted_column(effects: &[Effect], column: &str) -> Option<String> {
    effects.iter().find_map(|effect| {
        let (Effect::Persist { ops, .. } | Effect::PersistAtomic { ops, .. }) = effect else {
            return None;
        };
        ops.iter().find_map(|op| {
            let StorageOp::BatchUpsert(spec) = op else {
                return None;
            };
            spec.rows.iter().find_map(|row| {
                row.iter()
                    .find_map(|(key, value)| match (key.as_str(), value) {
                        (key, SqlValue::Text(text)) if key == column => Some(text.clone()),
                        _ => None,
                    })
            })
        })
    })
}

/// 取本批次 posts/create 的 JSON body。
fn posts_create_body(harness: &Harness) -> Value {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } if req.url.ends_with("/posts/create") => req
                .body
                .as_ref()
                .and_then(|body| serde_json::from_slice(body.as_ref()).ok()),
            _ => None,
        })
        .expect("posts/create body")
}

/// 把参考包最终 props 的 `template.file` 改一个字段后交给构造器。
fn mutate_reference_file(mutate: impl FnOnce(&mut Map<String, Value>)) -> Value {
    let mut props = reference_final_props();
    let file = props["template"]["file"]
        .as_object_mut()
        .expect("reference final props must carry props.template.file");
    mutate(file);
    props
}

// ═══ 标准测试集 ═════════════════════════════════════════════════════════════

/// 证明 MV3-G01g 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G01g", GATE_DIR);
}

/// ① inbound 样本载荷经 Host 解析后被发送边界接受；Angular 只提交 opaque handle。
#[test]
fn inbound_sample_command_is_accepted_by_the_send_boundary() {
    let payload = reference_command_payload();
    assert_eq!(payload["type"], "TEMPLATE");
    assert_eq!(payload["props"]["template"]["type"], "IMAGE");
    assert!(payload["props"]["template"]["file"]["mediaHandle"].is_string());
    for forbidden in ["temporaryId", "createAt", "simpleMessage", "readBits"] {
        assert!(
            payload.get(forbidden).is_none(),
            "Angular 不得提交 {forbidden}"
        );
    }
    for forbidden in ["width", "height", "sha256", "localPath", "bucket", "uri"] {
        assert!(
            payload["props"]["template"]["file"]
                .get(forbidden)
                .is_none(),
            "Angular 不得提交 Host/Java owned 字段 {forbidden}"
        );
    }

    let harness = run_send(host_resolved_command());
    let body = posts_create_body(&harness);
    assert_eq!(body["channelId"], common::cid(1));
    assert!(body["temporaryId"]
        .as_str()
        .is_some_and(|value| value.starts_with("helix_tmp_")));
}

/// ② 首批只允许一条 optimistic 持久化（INV-04）。
#[test]
fn first_effect_batch_is_only_the_optimistic_persist() {
    let mut harness = Harness::new_with_identity(&[1], AUTH_USER_ID, "company-phase1", "", "", "");
    let first = harness.step(
        command_json("im_send_message", host_resolved_command()),
        NOW_MS,
    );

    assert_eq!(first.len(), 1, "首批只允许一条 Effect：{first:?}");
    assert!(matches!(
        first.as_slice(),
        [EffectKind::Persist(_)] | [EffectKind::PersistAtomic(_)]
    ));
    assert!(emitted_event(&harness, "im:post:sending").is_none());
}

/// ③ PersistOk 续接才发布 sending 并出站 posts/create + 15s echo timer。
#[test]
fn persist_ok_continuation_publishes_sending_and_posts_create() {
    let harness = run_send(host_resolved_command());

    let after_ms = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::ScheduleTimer { after_ms, .. } => Some(*after_ms),
            _ => None,
        })
        .expect("echo timer 只能与真正出站的 posts/create 同批");
    assert_eq!(after_ms, 15_000);
    let sending = emitted_event(&harness, "im:post:sending").expect("sending projection");
    assert_eq!(sending["data"]["sendStatus"], "sending");
    assert_eq!(sending["data"]["type"], "TEMPLATE");
}

/// ④ TEMPLATE 壳在 wire body 与 sending 投影里同时保留，内芯 `type` 保持 `IMAGE`。
#[test]
fn template_shell_and_inner_image_type_are_preserved_on_the_wire() {
    let harness = run_send(host_resolved_command());
    let body = posts_create_body(&harness);
    let sending = emitted_event(&harness, "im:post:sending").expect("sending projection");

    assert_eq!(body["type"], "TEMPLATE");
    assert_eq!(
        body["props"]["template"]["type"], "IMAGE",
        "内芯 type 必须沿用客户端声明的 IMAGE，不得被默认值覆写"
    );
    assert!(
        body["props"]["template"]["file"].is_object(),
        "TEMPLATE 内芯必须继续挂着媒体节点"
    );
    assert_eq!(sending["data"]["type"], "TEMPLATE");
    assert_eq!(sending["data"]["props"]["template"]["type"], "IMAGE");
}

/// ⑤ **负向（本 Gate 核心）**：TEMPLATE 绝不能回落成 IMAGE 或 TEXT 消息。
///
/// 三条独立证伪：消息 `type` 不得变成 IMAGE/TEXT；props 不得被摊平成顶层 `file`/`files`；
/// 会话摘要不得走 IMAGE 分支的 `[图片]` 前缀。
#[test]
fn template_must_not_fall_back_to_image_or_text_message() {
    let harness = run_send(host_resolved_command());
    let body = posts_create_body(&harness);
    let row_type = persisted_column(harness.sink.as_slice(), "type");

    assert_ne!(body["type"], json!("IMAGE"), "禁止回落为 IMAGE 消息");
    assert_ne!(body["type"], json!("TEXT"), "禁止回落为 TEXT 消息");
    assert_ne!(
        body["props"]["template"]["type"],
        json!("TEXT"),
        "禁止把内芯回落为 TEXT 渲染"
    );
    assert!(
        body["props"].get("file").is_none() && body["props"].get("files").is_none(),
        "TEMPLATE 壳不得被摊平成顶层媒体 props：{}",
        body["props"]
    );
    assert_ne!(
        body["simpleMessage"], "[图片]",
        "摘要走 IMAGE 分支即说明类型已回落"
    );
    // 落库行同样不得回落（重启 rehydrate 会以此行为准）。
    assert!(
        row_type.is_none() || row_type.as_deref() == Some("TEMPLATE"),
        "落库 type 回落：{row_type:?}"
    );

    // 构造器层同样 fail-closed：把壳摊平后必须报错而不是接受。
    let mut flattened = reference_final_props();
    let file = flattened["template"]["file"].clone();
    flattened.as_object_mut().unwrap().remove("template");
    flattened["file"] = file;
    let error = build_media_message_props("TEMPLATE", &flattened)
        .expect_err("摊平的 TEMPLATE 必须 fail-closed");
    assert!(error.to_string().contains("回落"), "{error}");
}

/// ⑥ **对照**：不声明内芯 type 时 Helix 补的是 `TEXT`——证明 ④/⑤ 里的 `IMAGE` 来自客户端
/// 意图被保留，而不是实现把内芯硬编成了 IMAGE。
#[test]
fn omitted_inner_type_defaults_to_text_not_image() {
    let mut payload = host_resolved_command();
    payload["props"]["template"]
        .as_object_mut()
        .expect("template object")
        .remove("type");
    let harness = run_send(payload);
    let body = posts_create_body(&harness);

    assert_eq!(body["type"], "TEMPLATE", "壳不受内芯缺省影响");
    assert_eq!(
        body["props"]["template"]["type"], "TEXT",
        "内芯缺省值必须是 TEXT；若这里也是 IMAGE，说明实现把内芯硬编了"
    );
}

/// ⑦ TEMPLATE 内芯的兄弟字段（回执名单等）与同级 props 在规范化后原样保留。
#[test]
fn template_siblings_survive_normalization() {
    let mut payload = host_resolved_command();
    payload["props"]["template"]["userIds"] = json!(["user-receiver-445"]);
    payload["props"]["template"]["title"] = json!("季度海报");
    payload["props"]["extra"] = json!({"campaign": "phase1"});
    let harness = run_send(payload);
    let body = posts_create_body(&harness);

    assert_eq!(
        body["props"]["template"]["userIds"],
        json!(["user-receiver-445"])
    );
    assert_eq!(body["props"]["template"]["title"], "季度海报");
    assert_eq!(body["props"]["extra"], json!({"campaign": "phase1"}));
    assert_eq!(body["props"]["template"]["type"], "IMAGE");
}

/// ⑧ 重复 PersistOk 幂等：不得二次出站 posts/create 或二次 arm echo timer。
#[test]
fn repeated_persist_reply_is_idempotent() {
    let mut harness = Harness::new_with_identity(&[1], AUTH_USER_ID, "company-phase1", "", "", "");
    let first = harness.step(
        command_json("im_send_message", host_resolved_command()),
        NOW_MS,
    );
    let corr = match first.as_slice() {
        [EffectKind::Persist(corr)] | [EffectKind::PersistAtomic(corr)] => *corr,
        other => panic!("unexpected first batch {other:?}"),
    };
    harness.step(port_reply_ok(corr), NOW_MS + 1);
    let replay = harness.step(port_reply_ok(corr), NOW_MS + 2);

    assert!(replay
        .iter()
        .all(|effect| !matches!(effect, EffectKind::Http(_) | EffectKind::ScheduleTimer(_))));
    assert!(emitted_event(&harness, "im:post:sending").is_none());
}

/// ⑨ 最终 props 与参考包线型权威逐字一致；稳定对象引用被真正校验。
#[test]
fn final_props_round_trip_matches_the_reference_sample() {
    let expected = reference_final_props();
    let built = build_media_message_props("TEMPLATE", &expected).expect("参考样本必须被接受");

    assert_eq!(built, expected, "最终 props 必须与参考包逐字一致");
    assert_eq!(
        serde_json::to_string(&built).unwrap(),
        serde_json::to_string(&expected).unwrap()
    );
    assert_eq!(built["template"]["type"], "IMAGE");
    assert!(built["template"]["file"]["width"]
        .as_u64()
        .is_some_and(|v| v > 0));

    // 内芯 type 若不是 IMAGE，构造器必须拒绝而不是默默改写。
    for wrong in ["TEXT", "FILE", "image", ""] {
        let mut props = reference_final_props();
        props["template"]["type"] = json!(wrong);
        assert!(
            build_media_message_props("TEMPLATE", &props).is_err(),
            "template.type={wrong} 必须 fail-closed"
        );
    }
    // 预签名 / 越界 uri 不得冒充稳定对象引用。
    for forged_uri in [
        json!("https://signed.example/template.png?sig=x"),
        json!("//evil.example/template.png"),
        json!("/message/../../etc/passwd"),
        json!("/message/template.png#frag"),
    ] {
        let props = mutate_reference_file(|file| {
            file.insert("uri".to_string(), forged_uri.clone());
        });
        assert!(
            build_media_message_props("TEMPLATE", &props).is_err(),
            "uri={forged_uri} 必须失败"
        );
    }
    // Host 提取的尺寸缺一不可。
    for required in ["width", "height", "bucket", "id"] {
        let props = mutate_reference_file(|file| {
            file.remove(required);
        });
        assert!(
            build_media_message_props("TEMPLATE", &props).is_err(),
            "缺 {required} 必须失败"
        );
    }
}

/// ⑩ Host 私有字段在 TEMPLATE 壳**内芯**同样 fail-closed 并可被投影闸门剥除。
///
/// 这条独立于 AUDIO/VIDEO：内芯媒体节点藏在 `props.template.file`，比顶层 `props.file`
/// 多一层，最容易被只覆盖顶层的过滤器漏掉。
#[test]
fn host_private_fields_never_survive_the_template_inner_node() {
    let clean = reference_final_props();
    for field in HOST_PRIVATE_MEDIA_FIELDS {
        let polluted = mutate_reference_file(|file| {
            file.insert(field.to_string(), json!("host-private"));
        });
        let error = build_media_message_props("TEMPLATE", &polluted)
            .expect_err(&format!("{field} 必须 fail-closed"));
        assert!(error.to_string().contains(field), "{field}: {error}");
        assert_eq!(
            strip_host_private_media_fields(&polluted),
            clean,
            "{field} 必须在 props.template.file 层被剥除"
        );
    }
    let provenance = reference("provenance.json");
    assert_eq!(provenance["evidence"]["reference_sample"], json!(true));
    assert_eq!(provenance["evidence"]["real_http_ws_storage"], json!(false));
}
