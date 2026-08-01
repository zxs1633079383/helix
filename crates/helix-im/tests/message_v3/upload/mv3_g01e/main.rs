//! MV3-G01e：发送 AUDIO / VIDEO。
//!
//! AUDIO 与 VIDEO 是**两组互不共享发送 Tick 的独立场景**（spec §2.1 第 6 行 +
//! helix-gate-test-matrix「专项测试」行），因此下面每一项标准测试都有 `audio_*` 与
//! `video_*` 两个独立函数，各自完整跑一遍。
//!
//! 线型权威 = `specs/003-messagev3-gate-inventory/gates/upload/mv3-g01e/*.json`。
//! 涉及最终 props 的断言一律**从样本文件读取期望值**，不在测试里另抄一份常量：样本改了、
//! 实现没跟上（或反过来）都必须红。

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
const GATE_DIR: &str = "upload/mv3-g01e";
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

/// 取 `inbound-sample.json` 里指定变体的 `Tick::Command` 载荷（Angular 提交的最小意图）。
fn reference_command_payload(variant: &str) -> Value {
    reference("inbound-sample.json")["rust_entry_inputs"]
        .as_array()
        .expect("rust_entry_inputs must be an array")
        .iter()
        .find(|entry| {
            entry["tick_variant"] == "Tick::Command" && entry["value"]["payload"]["type"] == variant
        })
        .map(|entry| entry["value"]["payload"].clone())
        .unwrap_or_else(|| panic!("inbound sample must carry a {variant} send tick"))
}

/// 取 `inbound-sample.json` 里与某个 mediaHandle 对应的 Host 暂存结果。
fn reference_stage_result(media_handle: &str) -> Value {
    reference("inbound-sample.json")["host_stage_results"]
        .as_array()
        .expect("host_stage_results must be an array")
        .iter()
        .find(|entry| entry["media_handle"] == media_handle)
        .cloned()
        .unwrap_or_else(|| panic!("missing host stage result for {media_handle}"))
}

/// 取 `outbound-sample.json` 里指定变体 / 视角 / 事件的 `data`。
fn reference_event_data(variant: &str, viewer: &str, event: &str) -> Value {
    reference("outbound-sample.json")["samples"]
        .as_array()
        .expect("samples must be an array")
        .iter()
        .find(|sample| {
            sample["variant"] == variant
                && sample["viewer"] == viewer
                && sample["rust"]["event"] == event
        })
        .map(|sample| sample["rust"]["data"].clone())
        .unwrap_or_else(|| panic!("missing {variant}/{viewer}/{event} sample"))
}

/// 参考包声明的最终 props（线型权威，`build_media_message_props` 的期望输出）。
fn reference_final_props(variant: &str) -> Value {
    reference_event_data(variant, "sender", "im:post:received")["props"].clone()
}

// ── Host 边界模拟 ────────────────────────────────────────────────────────────

/// 模拟 `media_stage::resolve_send_payload`：把 opaque `mediaHandle` 换成 Host 可信 `mediaInput`
/// 并挂上 Host 提取的媒体元数据。
///
/// 这一步刻意在测试里复刻，是为了让 Helix 侧断言的输入**恰好**是 Host 合同允许交出去的形态：
/// Angular 只给 handle，路径 / SHA / 时长 / 尺寸全部由 Host 补。
fn host_resolved_command(variant: &str) -> Value {
    let mut payload = reference_command_payload(variant);
    let file = payload["props"]["file"]
        .as_object()
        .expect("media send intent must carry props.file")
        .clone();
    let media_handle = file["mediaHandle"]
        .as_str()
        .expect("Angular 只提交 opaque mediaHandle")
        .to_string();
    let stage = reference_stage_result(&media_handle);

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
    for key in ["duration", "width", "height"] {
        if let Some(value) = stage["metadata"].get(key) {
            resolved.insert(key.to_string(), value.clone());
        }
    }
    payload["props"]["file"] = Value::Object(resolved);
    // 参考包的 `channel-phase1-001` 是可读占位；Helix 的 ChannelId 是 26 字符 Id26。
    payload["channel_id"] = json!(common::cid(1));
    payload
}

/// 跑完「命令 → optimistic Persist → PersistOk 续接」两拍，返回 harness 与首拍 corr。
fn run_send(variant: &str) -> (Harness, Vec<EffectKind>, Vec<EffectKind>) {
    let mut harness = Harness::new_with_identity(&[1], AUTH_USER_ID, "company-phase1", "", "", "");
    let first = harness.step(
        command_json("im_send_message", host_resolved_command(variant)),
        NOW_MS,
    );
    let corr = match first.as_slice() {
        [EffectKind::Persist(corr)] | [EffectKind::PersistAtomic(corr)] => *corr,
        other => panic!("{variant}: expected a single optimistic persist, got {other:?}"),
    };
    let second = harness.step(port_reply_ok(corr), NOW_MS + 1);
    (harness, first, second)
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

/// 把参考包最终 props 的媒体节点改一个字段后交给构造器。
fn mutate_reference_file(variant: &str, mutate: impl FnOnce(&mut Map<String, Value>)) -> Value {
    let mut props = reference_final_props(variant);
    let file = props["file"]
        .as_object_mut()
        .expect("reference final props must carry props.file");
    mutate(file);
    props
}

// ═══ 共享 ═══════════════════════════════════════════════════════════════════

/// 证明 MV3-G01e 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G01e", GATE_DIR);
}

/// AUDIO 与 VIDEO 必须是**两组独立场景**：各自的 stage 与 send Tick 齐全、句柄互不复用，
/// 同一 Helix 实例连续跑两条也不会共享 temporaryId。
#[test]
fn audio_and_video_are_two_independent_scenarios() {
    let inbound = reference("inbound-sample.json");
    let stage_ticks = inbound["rust_entry_inputs"]
        .as_array()
        .expect("rust_entry_inputs")
        .iter()
        .filter(|entry| entry["tick_variant"] == "TauriHostCommand")
        .count();
    assert_eq!(stage_ticks, 2, "AUDIO 与 VIDEO 必须各有独立 im_stage_media");

    let audio = reference_command_payload("AUDIO");
    let video = reference_command_payload("VIDEO");
    assert_ne!(
        audio["props"]["file"]["mediaHandle"], video["props"]["file"]["mediaHandle"],
        "两组场景不得共享 mediaHandle"
    );
    assert_ne!(
        audio["props"]["file"]["contentType"], video["props"]["file"]["contentType"],
        "两组场景不得共享 contentType"
    );

    let mut harness = Harness::new_with_identity(&[1], AUTH_USER_ID, "company-phase1", "", "", "");
    harness.step(
        command_json("im_send_message", host_resolved_command("AUDIO")),
        NOW_MS,
    );
    let audio_tmp = persisted_column(harness.sink.as_slice(), "temporary_id").expect("audio row");
    harness.step(
        command_json("im_send_message", host_resolved_command("VIDEO")),
        NOW_MS + 1,
    );
    let video_tmp = persisted_column(harness.sink.as_slice(), "temporary_id").expect("video row");
    assert_ne!(
        audio_tmp, video_tmp,
        "两条独立发送必须各自铸造 temporaryId（INV-02）"
    );
}

// ═══ AUDIO 场景（10 项） ════════════════════════════════════════════════════

/// ①（AUDIO）inbound 样本载荷经 Host 解析后可被发送边界接受，并铸造 Helix 侧 temporaryId。
#[test]
fn audio_inbound_sample_command_is_accepted_by_the_send_boundary() {
    let payload = reference_command_payload("AUDIO");
    assert_eq!(payload["type"], "AUDIO");
    // INV-01：Angular 只提交 opaque handle 与用户选择。
    assert!(payload["props"]["file"]["mediaHandle"].is_string());
    for forbidden in ["temporaryId", "createAt", "simpleMessage", "readBits"] {
        assert!(
            payload.get(forbidden).is_none(),
            "Angular 不得提交 {forbidden}"
        );
    }
    for forbidden in ["duration", "width", "height", "sha256", "localPath"] {
        assert!(
            payload["props"]["file"].get(forbidden).is_none(),
            "Angular 不得提交 Host owned 字段 {forbidden}"
        );
    }

    let (harness, _, _) = run_send("AUDIO");
    let body = posts_create_body(&harness);
    assert_eq!(body["type"], "AUDIO");
    assert_eq!(body["channelId"], common::cid(1));
    assert!(
        body["temporaryId"]
            .as_str()
            .is_some_and(|value| value.starts_with("helix_tmp_")),
        "temporaryId 必须由 Helix 铸造"
    );
}

/// ②（AUDIO）首批只允许一条 optimistic 持久化，禁止提前 Emit / HTTP / timer（INV-04）。
#[test]
fn audio_first_effect_batch_is_only_the_optimistic_persist() {
    let mut harness = Harness::new_with_identity(&[1], AUTH_USER_ID, "company-phase1", "", "", "");
    let first = harness.step(
        command_json("im_send_message", host_resolved_command("AUDIO")),
        NOW_MS,
    );

    assert_eq!(first.len(), 1, "首批只允许一条 Effect：{first:?}");
    assert!(
        matches!(
            first.as_slice(),
            [EffectKind::Persist(_)] | [EffectKind::PersistAtomic(_)]
        ),
        "首批必须是 optimistic 持久化：{first:?}"
    );
    assert!(emitted_event(&harness, "im:post:sending").is_none());
}

/// ③（AUDIO）PersistOk 续接才发布 sending 并真正出站 posts/create + 15s echo timer。
#[test]
fn audio_persist_ok_continuation_publishes_sending_and_posts_create() {
    let (harness, _, second) = run_send("AUDIO");

    assert_eq!(
        second
            .iter()
            .filter(|effect| matches!(effect, EffectKind::Http(_)))
            .count(),
        1
    );
    assert_eq!(
        second
            .iter()
            .filter(|effect| matches!(effect, EffectKind::ScheduleTimer(_)))
            .count(),
        1,
        "echo timer 只能与真正出站的 posts/create 同批"
    );
    let after_ms = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::ScheduleTimer { after_ms, .. } => Some(*after_ms),
            _ => None,
        })
        .expect("echo timer");
    assert_eq!(after_ms, 15_000, "INV-04：echo 等待窗固定 15 秒");
    let sending = emitted_event(&harness, "im:post:sending").expect("sending projection");
    assert_eq!(sending["data"]["sendStatus"], "sending");
    assert_eq!(sending["data"]["type"], "AUDIO");
}

/// ④（AUDIO）sender 回放：wire body 的类型 / 摘要 / 身份字段全部由 Helix 权威构造。
#[test]
fn audio_sender_wire_body_is_helix_authoritative() {
    let (harness, _, _) = run_send("AUDIO");
    let body = posts_create_body(&harness);

    assert_eq!(body["type"], "AUDIO");
    assert_eq!(
        body["simpleMessage"], "[音频]",
        "AUDIO 的会话摘要前缀由 Helix 决定，Angular 未提交"
    );
    assert_eq!(body["userId"], AUTH_USER_ID);
    assert_eq!(body["createAt"], json!(NOW_MS));
    assert_eq!(body["revoke"], json!(false));
    assert_eq!(body["viewers"], json!(["all"]));
}

/// ⑤（AUDIO）sending 投影只由 Helix 侧 body 派生，不回显任何 Angular 未提交的客户端计算。
#[test]
fn audio_sending_projection_is_derived_from_the_helix_body() {
    let (harness, _, _) = run_send("AUDIO");
    let sending = emitted_event(&harness, "im:post:sending").expect("sending projection");
    let body = posts_create_body(&harness);

    assert_eq!(sending["data"]["temporaryId"], body["temporaryId"]);
    assert_eq!(sending["data"]["simpleMessage"], "[音频]");
    assert_eq!(sending["data"]["createAt"], json!(NOW_MS));
    assert_eq!(sending["data"]["userId"], AUTH_USER_ID);
    // INV-07：Angular 永不接触 bitmap。
    assert!(sending["data"].get("readBits").is_none());
}

/// ⑥（AUDIO）Storage read-back：optimistic 行的类型 / 摘要 / 发送态与 Host 私有字段边界。
#[test]
fn audio_optimistic_row_matches_the_sending_projection() {
    let mut harness = Harness::new_with_identity(&[1], AUTH_USER_ID, "company-phase1", "", "", "");
    harness.step(
        command_json("im_send_message", host_resolved_command("AUDIO")),
        NOW_MS,
    );
    let effects = harness.sink.as_slice();

    assert_eq!(persisted_column(effects, "type").as_deref(), Some("AUDIO"));
    assert_eq!(
        persisted_column(effects, "simple_message").as_deref(),
        Some("[音频]")
    );
    assert_eq!(
        persisted_column(effects, "send_status").as_deref(),
        Some("sending")
    );
    let props = persisted_column(effects, "props").expect("props column");
    for forbidden in ["mediaInput", "localPath", "sha256", "host-private"] {
        assert!(
            !props.contains(forbidden),
            "落库 props 不得携带 {forbidden}：{props}"
        );
    }
    assert!(
        props.contains("duration"),
        "Host 提取的时长必须留在可渲染 props 中：{props}"
    );
}

/// ⑦（AUDIO）重复 PersistOk 幂等：不得二次出站 posts/create 或二次 arm echo timer。
#[test]
fn audio_repeated_persist_reply_is_idempotent() {
    let mut harness = Harness::new_with_identity(&[1], AUTH_USER_ID, "company-phase1", "", "", "");
    let first = harness.step(
        command_json("im_send_message", host_resolved_command("AUDIO")),
        NOW_MS,
    );
    let corr = match first.as_slice() {
        [EffectKind::Persist(corr)] | [EffectKind::PersistAtomic(corr)] => *corr,
        other => panic!("unexpected first batch {other:?}"),
    };
    harness.step(port_reply_ok(corr), NOW_MS + 1);
    let replay = harness.step(port_reply_ok(corr), NOW_MS + 2);

    assert!(
        replay
            .iter()
            .all(|effect| !matches!(effect, EffectKind::Http(_) | EffectKind::ScheduleTimer(_))),
        "重复 PortReply 不得再次出站：{replay:?}"
    );
    assert!(emitted_event(&harness, "im:post:sending").is_none());
}

/// ⑧（AUDIO）伪造或缺失 Host 元数据一律 fail-closed。
#[test]
fn audio_forged_or_missing_host_metadata_fails_closed() {
    // media-host-contract §3：Host 不为音频提取尺寸，出现 width/height 只可能是客户端伪造。
    for forged in ["width", "height"] {
        let props = mutate_reference_file("AUDIO", |file| {
            file.insert(forged.to_string(), json!(1280));
        });
        let error = build_media_message_props("AUDIO", &props)
            .expect_err(&format!("AUDIO 携带 {forged} 必须失败"));
        assert!(
            error.to_string().contains("width/height"),
            "{forged}: {error}"
        );
    }
    for broken in [json!(0), json!(-1.0), json!("8.25"), Value::Null] {
        let props = mutate_reference_file("AUDIO", |file| {
            file.insert("duration".to_string(), broken.clone());
        });
        assert!(
            build_media_message_props("AUDIO", &props).is_err(),
            "duration={broken} 必须失败"
        );
    }
    let props = mutate_reference_file("AUDIO", |file| {
        file.remove("duration");
    });
    assert!(
        build_media_message_props("AUDIO", &props).is_err(),
        "缺 duration 必须失败"
    );
    // 单文件形态不得混入多图集合。
    let mut multi = reference_final_props("AUDIO");
    multi["files"] = json!([]);
    assert!(build_media_message_props("AUDIO", &multi).is_err());
}

/// ⑨（AUDIO）最终 props 与参考包线型权威逐字一致（PC/Mobile 共用同一构造器，故同为 byte-exact）。
#[test]
fn audio_final_props_round_trip_matches_the_reference_sample() {
    let expected = reference_final_props("AUDIO");
    let built = build_media_message_props("AUDIO", &expected).expect("参考样本必须被接受");

    assert_eq!(built, expected, "最终 props 必须与参考包逐字一致");
    assert_eq!(
        serde_json::to_string(&built).unwrap(),
        serde_json::to_string(&expected).unwrap(),
        "序列化后同样 byte-exact"
    );
    assert!(built["file"]["duration"].as_f64().is_some_and(|d| d > 0.0));
    assert!(
        built["file"].get("width").is_none() && built["file"].get("height").is_none(),
        "AUDIO 不得出现尺寸字段"
    );
}

/// ⑩（AUDIO）Host 私有字段既不能进入构造器输出，也必须能被投影闸门剥除（§6 + provenance）。
#[test]
fn audio_host_private_fields_never_survive_the_projection_gate() {
    let clean = reference_final_props("AUDIO");
    for field in HOST_PRIVATE_MEDIA_FIELDS {
        let polluted = mutate_reference_file("AUDIO", |file| {
            file.insert(field.to_string(), json!("host-private"));
        });
        let error = build_media_message_props("AUDIO", &polluted)
            .expect_err(&format!("{field} 必须 fail-closed"));
        assert!(error.to_string().contains(field), "{field}: {error}");
        assert_eq!(
            strip_host_private_media_fields(&polluted),
            clean,
            "{field} 必须被投影闸门剥除"
        );
    }
    // provenance：参考包必须自证是目标样本而非运行时捕获。
    let provenance = reference("provenance.json");
    assert_eq!(provenance["evidence"]["reference_sample"], json!(true));
    assert_eq!(provenance["evidence"]["real_http_ws_storage"], json!(false));
}

// ═══ VIDEO 场景（10 项） ════════════════════════════════════════════════════

/// ①（VIDEO）inbound 样本载荷经 Host 解析后可被发送边界接受，并铸造 Helix 侧 temporaryId。
#[test]
fn video_inbound_sample_command_is_accepted_by_the_send_boundary() {
    let payload = reference_command_payload("VIDEO");
    assert_eq!(payload["type"], "VIDEO");
    assert!(payload["props"]["file"]["mediaHandle"].is_string());
    for forbidden in ["temporaryId", "createAt", "simpleMessage", "readBits"] {
        assert!(
            payload.get(forbidden).is_none(),
            "Angular 不得提交 {forbidden}"
        );
    }
    for forbidden in ["duration", "width", "height", "sha256", "localPath"] {
        assert!(
            payload["props"]["file"].get(forbidden).is_none(),
            "Angular 不得提交 Host owned 字段 {forbidden}"
        );
    }

    let (harness, _, _) = run_send("VIDEO");
    let body = posts_create_body(&harness);
    assert_eq!(body["type"], "VIDEO");
    assert_eq!(body["channelId"], common::cid(1));
    assert!(body["temporaryId"]
        .as_str()
        .is_some_and(|value| value.starts_with("helix_tmp_")));
}

/// ②（VIDEO）首批只允许一条 optimistic 持久化，禁止提前 Emit / HTTP / timer（INV-04）。
#[test]
fn video_first_effect_batch_is_only_the_optimistic_persist() {
    let mut harness = Harness::new_with_identity(&[1], AUTH_USER_ID, "company-phase1", "", "", "");
    let first = harness.step(
        command_json("im_send_message", host_resolved_command("VIDEO")),
        NOW_MS,
    );

    assert_eq!(first.len(), 1, "首批只允许一条 Effect：{first:?}");
    assert!(matches!(
        first.as_slice(),
        [EffectKind::Persist(_)] | [EffectKind::PersistAtomic(_)]
    ));
    assert!(emitted_event(&harness, "im:post:sending").is_none());
}

/// ③（VIDEO）PersistOk 续接才发布 sending 并真正出站 posts/create + 15s echo timer。
#[test]
fn video_persist_ok_continuation_publishes_sending_and_posts_create() {
    let (harness, _, second) = run_send("VIDEO");

    assert_eq!(
        second
            .iter()
            .filter(|effect| matches!(effect, EffectKind::Http(_)))
            .count(),
        1
    );
    assert_eq!(
        second
            .iter()
            .filter(|effect| matches!(effect, EffectKind::ScheduleTimer(_)))
            .count(),
        1
    );
    let after_ms = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::ScheduleTimer { after_ms, .. } => Some(*after_ms),
            _ => None,
        })
        .expect("echo timer");
    assert_eq!(after_ms, 15_000);
    let sending = emitted_event(&harness, "im:post:sending").expect("sending projection");
    assert_eq!(sending["data"]["sendStatus"], "sending");
    assert_eq!(sending["data"]["type"], "VIDEO");
}

/// ④（VIDEO）sender 回放：wire body 的类型 / 摘要 / 身份字段全部由 Helix 权威构造。
#[test]
fn video_sender_wire_body_is_helix_authoritative() {
    let (harness, _, _) = run_send("VIDEO");
    let body = posts_create_body(&harness);

    assert_eq!(body["type"], "VIDEO");
    assert_eq!(
        body["simpleMessage"], "[视频]",
        "VIDEO 的会话摘要前缀必须与 AUDIO 区分开"
    );
    assert_eq!(body["userId"], AUTH_USER_ID);
    assert_eq!(body["createAt"], json!(NOW_MS));
    assert_eq!(body["revoke"], json!(false));
    assert_eq!(body["viewers"], json!(["all"]));
}

/// ⑤（VIDEO）sending 投影只由 Helix 侧 body 派生。
#[test]
fn video_sending_projection_is_derived_from_the_helix_body() {
    let (harness, _, _) = run_send("VIDEO");
    let sending = emitted_event(&harness, "im:post:sending").expect("sending projection");
    let body = posts_create_body(&harness);

    assert_eq!(sending["data"]["temporaryId"], body["temporaryId"]);
    assert_eq!(sending["data"]["simpleMessage"], "[视频]");
    assert_eq!(sending["data"]["createAt"], json!(NOW_MS));
    assert_eq!(sending["data"]["userId"], AUTH_USER_ID);
    assert!(sending["data"].get("readBits").is_none());
}

/// ⑥（VIDEO）Storage read-back：optimistic 行的类型 / 摘要 / 发送态与 Host 私有字段边界。
#[test]
fn video_optimistic_row_matches_the_sending_projection() {
    let mut harness = Harness::new_with_identity(&[1], AUTH_USER_ID, "company-phase1", "", "", "");
    harness.step(
        command_json("im_send_message", host_resolved_command("VIDEO")),
        NOW_MS,
    );
    let effects = harness.sink.as_slice();

    assert_eq!(persisted_column(effects, "type").as_deref(), Some("VIDEO"));
    assert_eq!(
        persisted_column(effects, "simple_message").as_deref(),
        Some("[视频]")
    );
    assert_eq!(
        persisted_column(effects, "send_status").as_deref(),
        Some("sending")
    );
    let props = persisted_column(effects, "props").expect("props column");
    for forbidden in ["mediaInput", "localPath", "sha256", "host-private"] {
        assert!(
            !props.contains(forbidden),
            "落库 props 不得携带 {forbidden}"
        );
    }
    for kept in ["duration", "width", "height"] {
        assert!(
            props.contains(kept),
            "Host 提取的 {kept} 必须留在可渲染 props 中：{props}"
        );
    }
}

/// ⑦（VIDEO）重复 PersistOk 幂等：不得二次出站 posts/create 或二次 arm echo timer。
#[test]
fn video_repeated_persist_reply_is_idempotent() {
    let mut harness = Harness::new_with_identity(&[1], AUTH_USER_ID, "company-phase1", "", "", "");
    let first = harness.step(
        command_json("im_send_message", host_resolved_command("VIDEO")),
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

/// ⑧（VIDEO）尺寸与时长三件套缺一不可，越权取值 fail-closed。
#[test]
fn video_forged_or_missing_host_metadata_fails_closed() {
    for required in ["duration", "width", "height"] {
        let props = mutate_reference_file("VIDEO", |file| {
            file.remove(required);
        });
        let error = build_media_message_props("VIDEO", &props)
            .expect_err(&format!("VIDEO 缺 {required} 必须失败"));
        assert!(error.to_string().contains(required), "{required}: {error}");
    }
    for broken in [json!(0), json!(-720), json!("720"), json!(65_536)] {
        let props = mutate_reference_file("VIDEO", |file| {
            file.insert("height".to_string(), broken.clone());
        });
        assert!(
            build_media_message_props("VIDEO", &props).is_err(),
            "height={broken} 必须失败"
        );
    }
    // 稳定对象引用必须是站内绝对路径，预签名地址不得冒充。
    for forged_uri in [
        json!("https://signed.example/sample.mp4?sig=x"),
        json!("//evil.example/sample.mp4"),
        json!("/message/../../etc/passwd"),
        json!(""),
    ] {
        let props = mutate_reference_file("VIDEO", |file| {
            file.insert("uri".to_string(), forged_uri.clone());
        });
        assert!(
            build_media_message_props("VIDEO", &props).is_err(),
            "uri={forged_uri} 必须失败"
        );
    }
}

/// ⑨（VIDEO）最终 props 与参考包线型权威逐字一致，且与 AUDIO 的键集真正不同。
#[test]
fn video_final_props_round_trip_matches_the_reference_sample() {
    let expected = reference_final_props("VIDEO");
    let built = build_media_message_props("VIDEO", &expected).expect("参考样本必须被接受");

    assert_eq!(built, expected);
    assert_eq!(
        serde_json::to_string(&built).unwrap(),
        serde_json::to_string(&expected).unwrap()
    );
    let video_keys: Vec<&String> = built["file"].as_object().unwrap().keys().collect();
    let audio_props = reference_final_props("AUDIO");
    let audio_keys: Vec<&String> = audio_props["file"].as_object().unwrap().keys().collect();
    assert_ne!(video_keys, audio_keys, "VIDEO 必须比 AUDIO 多出尺寸字段");
    assert!(built["file"]["width"].as_u64().is_some_and(|v| v > 0));
    assert!(built["file"]["height"].as_u64().is_some_and(|v| v > 0));
}

/// ⑩（VIDEO）Host 私有字段既不能进入构造器输出，也必须能被投影闸门剥除（§6 + provenance）。
#[test]
fn video_host_private_fields_never_survive_the_projection_gate() {
    let clean = reference_final_props("VIDEO");
    for field in HOST_PRIVATE_MEDIA_FIELDS {
        let polluted = mutate_reference_file("VIDEO", |file| {
            file.insert(field.to_string(), json!("host-private"));
        });
        let error = build_media_message_props("VIDEO", &polluted)
            .expect_err(&format!("{field} 必须 fail-closed"));
        assert!(error.to_string().contains(field), "{field}: {error}");
        assert_eq!(strip_host_private_media_fields(&polluted), clean);
    }
    let inbound = reference("inbound-sample.json");
    assert_eq!(inbound["evidence_level"], "TARGET_REFERENCE_SAMPLE");
    assert_eq!(inbound["runtime_capture"], json!(false));
}
