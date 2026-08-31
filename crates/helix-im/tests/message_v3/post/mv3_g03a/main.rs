//! MV3-G03a｜逐条转发（canonical `mode: "item"`）。
//!
//! 本 Gate 的核心是**数量语义**：N 条源消息 × M 个目标频道 → 每个目标频道各得 N 条独立消息，
//! 接收者在每个目标频道各累加 N 次未读；发送者自身未读恒为 0（INV-12，禁止跨 persona 复制）。
//!
//! 未读断言不接受测试喂数：本文件内置一个 `channel_member` 迷你驱动，严格按实现写出的
//! `BatchUpsert` / `ScopedGuardedBump`（含 `exclude_from_update` 与前进守卫）演算，
//! 读回再交还实现组装 `im:channel:update`。因此 `unreadCount` 是实现写的 delta 的函数。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::{InboundBytes, ReplyBytes};
use helix_core::{Correlation, Module, Tick};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{cid, enveloped, port_reply_ok, port_reply_with_body, sid, Harness};

const TARGET_A: u64 = 5;
const TARGET_B: u64 = 6;
const SOURCE_CHANNEL: u64 = 50;
const SOURCE_A: u64 = 51;
const SOURCE_B: u64 = 52;
const SENDER: &str = "user-actor-444";
const RECEIVER: &str = "user-receiver-445";
const REQ_ID: &str = "mv3-g03a-forward-1";
const CREATE_AT: i64 = 1_700_000_000_000;

// ─── 参考包 ────────────────────────────────────────────────────────────────

/// 证明 MV3-G03a 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G03a", "post/mv3-g03a");
}

/// 从参考包读出 canonical 入站载荷，避免测试凭空发明字段形状。
fn reference_payload() -> Value {
    let path = phase1_reference::reference_root()
        .join("post/mv3-g03a")
        .join("inbound-sample.json");
    let sample = phase1_reference::read_json(&path);
    sample["rust_entry_inputs"][0]["value"]["payload"].clone()
}

// ─── channel_member 迷你驱动 ───────────────────────────────────────────────

/// 忠实复演 `BatchUpsert(exclude_from_update)` 与 `ScopedGuardedBump(guard_col>)` 的复合主键表。
#[derive(Default)]
struct MemberTable {
    rows: BTreeMap<(String, String), BTreeMap<String, SqlValue>>,
}

impl MemberTable {
    fn apply(&mut self, ops: &[StorageOp]) {
        for op in ops {
            match op {
                StorageOp::BatchUpsert(spec) if spec.table == "channel_member" => {
                    for row in &spec.rows {
                        let key = (column_text(row, "channel_id"), column_text(row, "user_id"));
                        let entry = self.rows.entry(key).or_default();
                        let inserting = entry.is_empty();
                        for (column, value) in row {
                            if !inserting && spec.exclude_from_update.contains(&column.as_str()) {
                                continue;
                            }
                            entry.insert(column.clone(), value.clone());
                        }
                    }
                }
                StorageOp::ScopedGuardedBump(spec) if spec.table == "channel_member" => {
                    let key = (text_of(&spec.scope_val), text_of(&spec.key_val));
                    let Some(entry) = self.rows.get_mut(&key) else {
                        continue;
                    };
                    let guard = entry.get(spec.guard_col).map(int_of).unwrap_or_default();
                    if spec.guard_val <= guard {
                        continue; // 前进守卫：重复/乱序帧不重复自增
                    }
                    let current = entry.get(spec.bump_col).map(int_of).unwrap_or_default();
                    entry.insert(
                        spec.bump_col.to_string(),
                        SqlValue::Integer(current + spec.bump_delta),
                    );
                    for (column, value) in &spec.set_cols {
                        entry.insert(column.clone(), value.clone());
                    }
                }
                _ => {}
            }
        }
    }

    /// 复合键读回：交还实现自己去组装 `im:channel:update`。
    fn read(&self, channel_id: &str, viewer: &str) -> ReplyBytes {
        let row: Row = self
            .rows
            .get(&(channel_id.to_string(), viewer.to_string()))
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        ReplyBytes(rows_to_reply_bytes(&[row]))
    }

    fn unread(&self, channel_id: &str, viewer: &str) -> i64 {
        self.rows
            .get(&(channel_id.to_string(), viewer.to_string()))
            .and_then(|row| row.get("unread_count"))
            .map(int_of)
            .unwrap_or_default()
    }
}

fn text_of(value: &SqlValue) -> String {
    match value {
        SqlValue::Text(value) => value.clone(),
        other => panic!("expected text column, got {other:?}"),
    }
}

fn int_of(value: &SqlValue) -> i64 {
    match value {
        SqlValue::Integer(value) => *value,
        _ => 0,
    }
}

fn column_text(row: &Row, column: &str) -> String {
    row.iter()
        .find_map(|(name, value)| (name == column).then(|| text_of(value)))
        .unwrap_or_else(|| panic!("channel_member row missing {column}"))
}

// ─── Effect 探针 ───────────────────────────────────────────────────────────

fn emitted(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| {
            let Effect::Emit { event } = effect else {
                return None;
            };
            serde_json::from_slice(event.0.as_ref()).ok()
        })
        .collect()
}

fn http_corr(effects: &[Effect]) -> Option<Correlation> {
    effects.iter().find_map(|effect| match effect {
        Effect::Http { corr, .. } => Some(*corr),
        _ => None,
    })
}

fn persist_corr(effects: &[Effect]) -> Option<Correlation> {
    effects.iter().find_map(|effect| match effect {
        Effect::Persist { corr, .. } => Some(*corr),
        _ => None,
    })
}

/// 断言来源阶段只做按 id 的 `message` 只读 Get，转发绝不改写来源行。
fn expect_source_get(effects: &[Effect], source_id: &str) -> Correlation {
    effects
        .iter()
        .find_map(|effect| {
            let Effect::Persist { corr, ops } = effect else {
                return None;
            };
            let [StorageOp::Get(spec)] = ops.as_slice() else {
                return None;
            };
            (spec.table == "message" && spec.key_col == "id" && text_of(&spec.key_val) == source_id)
                .then_some(*corr)
        })
        .unwrap_or_else(|| panic!("missing read-only source lookup for {source_id}"))
}

fn atomic_commit(effects: &[Effect]) -> Option<(Correlation, &Vec<StorageOp>)> {
    effects.iter().find_map(|effect| {
        let Effect::PersistAtomic { corr, ops } = effect else {
            return None;
        };
        Some((*corr, ops))
    })
}

/// 抽出写集中对 `channel_member.unread_count` 的自增意图。
fn unread_bumps(ops: &[StorageOp]) -> Vec<i64> {
    ops.iter()
        .filter_map(|op| {
            let StorageOp::ScopedGuardedBump(spec) = op else {
                return None;
            };
            (spec.table == "channel_member"
                && spec.bump_col == "unread_count"
                && spec.bump_delta > 0)
                .then_some(spec.bump_delta)
        })
        .collect()
}

/// 抽出写集中对 `channel_member.unread_count` 的绝对置位意图（发送者视角）。
fn unread_absolute_writes(ops: &[StorageOp]) -> Vec<i64> {
    ops.iter()
        .filter_map(|op| {
            let StorageOp::BatchUpsert(spec) = op else {
                return None;
            };
            if spec.table != "channel_member" || spec.exclude_from_update.contains(&"unread_count")
            {
                return None;
            }
            spec.rows.first().and_then(|row| {
                row.iter()
                    .find_map(|(name, value)| (name == "unread_count").then(|| int_of(value)))
            })
        })
        .collect()
}

// ─── 输入构造 ──────────────────────────────────────────────────────────────

fn source_row(id: u64, user_id: &str, msg_type: &str, message: &str, create_at: i64) -> Value {
    json!({
        "id": sid(id),
        "temporary_id": format!("source-{}", sid(id)),
        "channel_id": cid(SOURCE_CHANNEL),
        "user_id": user_id,
        "type": msg_type,
        "message": message,
        "simple_message": message,
        "props": "{}",
        "mentions": "[]",
        "viewers": "[\"all\"]",
        "user_snapshot": json!({"userName": user_id}).to_string(),
        "create_at": create_at,
    })
}

fn relay_payload(mode: &str, source_ids: &[String], targets: &[String]) -> Value {
    json!({
        "post_ids": source_ids,
        "target_channel_ids": targets,
        "mode": mode,
        "req_id": REQ_ID,
    })
}

fn relay_tick(payload: &Value) -> Tick {
    let bytes = serde_json::to_vec(payload).expect("payload encodes");
    helix_im::client_api::build_command("im_relay_messages", &bytes)
        .map(Tick::Command)
        .expect("public relay command builds")
}

/// 驱动 item 模式直到 createPosts 出站，返回真实 HTTP body。
fn drive_item_forward(
    harness: &mut Harness,
    rows: &[Value],
    targets: &[String],
    mode: &str,
    now_ms: u64,
) -> Value {
    let source_ids: Vec<String> = rows
        .iter()
        .map(|row| row["id"].as_str().expect("source id").to_string())
        .collect();
    harness.step(
        relay_tick(&relay_payload(mode, &source_ids, targets)),
        now_ms,
    );
    for (index, source_id) in source_ids.iter().enumerate() {
        let corr = expect_source_get(harness.sink.as_slice(), source_id);
        let body = serde_json::to_vec(&json!([rows[index]])).expect("row encodes");
        harness.step(
            port_reply_with_body(corr.raw(), Bytes::from(body)),
            now_ms + index as u64 + 1,
        );
    }
    harness
        .last_http_body()
        .expect("item forward must reach createPosts")
}

fn acceptance(targets: &[String]) -> Bytes {
    let targets: Vec<Value> = targets
        .iter()
        .map(|channel_id| json!({"channelId": channel_id, "status": "accepted"}))
        .collect();
    let raw = serde_json::to_vec(&json!({
        "status": "SUCCESS",
        "data": {"targets": targets}
    }))
    .expect("acceptance encodes");
    enveloped(Bytes::from(raw))
}

/// 把 createPosts 的一条 post 补成 Go WS `post` authority。
fn authority(post: &Value, channel_id: &str, id: u64, seq: u64, create_at: i64) -> Value {
    let mut post = post.clone();
    post["id"] = json!(sid(id));
    post["channelId"] = json!(channel_id);
    post["createAt"] = json!(create_at);
    post["updateAt"] = json!(create_at);
    json!({
        "action": "post",
        "tracing": {"csesTrackId": REQ_ID},
        "data": {"event_seq": seq, "channelId": channel_id, "post": post}
    })
}

struct Applied {
    committed: bool,
    bumps: Vec<i64>,
    absolute: Vec<i64>,
    events: Vec<Value>,
}

/// 完整跑 authority → PersistAtomic → 读回 → Emit，并把写集应用到迷你驱动。
fn apply_authority(
    harness: &mut Harness,
    table: &mut MemberTable,
    viewer: &str,
    channel_id: &str,
    authority: &Value,
    now_ms: u64,
) -> Applied {
    let inbound = serde_json::to_vec(authority).expect("authority encodes");
    harness.step(Tick::Inbound(InboundBytes(Bytes::from(inbound))), now_ms);
    let Some((commit_corr, ops)) = atomic_commit(harness.sink.as_slice()) else {
        return Applied {
            committed: false,
            bumps: Vec::new(),
            absolute: Vec::new(),
            events: emitted(harness.sink.as_slice()),
        };
    };
    let bumps = unread_bumps(ops);
    let absolute = unread_absolute_writes(ops);
    table.apply(ops);

    harness.step(port_reply_ok(commit_corr.raw()), now_ms + 1);
    let readback = persist_corr(harness.sink.as_slice())
        .expect("committed authority must read the viewer's member row back");
    harness.step(
        Tick::PortReply {
            corr: readback,
            outcome: helix_core::tick::PortOutcome::Ok(table.read(channel_id, viewer)),
        },
        now_ms + 2,
    );
    Applied {
        committed: true,
        bumps,
        absolute,
        events: emitted(harness.sink.as_slice()),
    }
}

// ─── Gate 断言 ─────────────────────────────────────────────────────────────

/// 参考包的 canonical mode token 必须被内核接受，不得因内核旧别名反向削弱目标载荷。
#[test]
fn canonical_item_mode_token_from_reference_is_accepted() {
    let payload = reference_payload();
    assert_eq!(
        payload["mode"], "item",
        "参考包是线型权威：MV3-G03a 的 mode token 必须是 item"
    );
    let mode = payload["mode"].as_str().expect("mode token").to_string();

    let mut harness = Harness::new_with_identity(&[TARGET_A], SENDER, "mv3-g03a", "", "", "");
    let rows = [source_row(
        SOURCE_A,
        "source-user-a",
        "TEXT",
        "A",
        CREATE_AT - 20,
    )];
    let body = drive_item_forward(
        &mut harness,
        &rows,
        &[cid(TARGET_A)],
        &mode,
        CREATE_AT as u64,
    );
    assert_eq!(
        body["posts"].as_array().map(Vec::len),
        Some(1),
        "canonical item token 必须走逐条装配，而不是被当成未知 mode 拒绝"
    );
    assert_eq!(body["posts"][0]["type"], "TEXT");
}

/// N 条源消息 × M 个目标频道：出站恰好 N 条 post 与 M 个目标，顺序与类型逐条保真。
#[test]
fn item_mode_assembles_one_post_per_source_for_every_target() {
    let mut harness =
        Harness::new_with_identity(&[TARGET_A, TARGET_B], SENDER, "mv3-g03a", "", "", "");
    let rows = [
        source_row(
            SOURCE_A,
            "source-user-a",
            "TEXT",
            "第一条源消息",
            CREATE_AT - 20,
        ),
        source_row(
            SOURCE_B,
            "source-user-b",
            "FILE",
            "第二条源消息",
            CREATE_AT - 10,
        ),
    ];
    let targets = [cid(TARGET_A), cid(TARGET_B)];
    let body = drive_item_forward(&mut harness, &rows, &targets, "item", CREATE_AT as u64);

    let posts = body["posts"].as_array().expect("posts array");
    assert_eq!(posts.len(), 2, "item 模式必须 1 源 = 1 post，禁止合并");
    assert_eq!(
        body["channelIds"],
        json!([cid(TARGET_A), cid(TARGET_B)]),
        "M 个目标频道必须原样进入同一次 createPosts"
    );
    assert_eq!(
        posts
            .iter()
            .map(|post| post["message"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["第一条源消息", "第二条源消息"],
        "逐条转发必须保持源消息顺序"
    );
    assert_eq!(
        posts
            .iter()
            .map(|post| post["type"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["TEXT", "FILE"],
        "逐条转发必须逐条保留源消息类型，不得统一降级为 TEXT"
    );
    assert_eq!(
        posts
            .iter()
            .map(|post| post["temporaryId"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>(),
        vec![format!("relay-{REQ_ID}-0"), format!("relay-{REQ_ID}-1")],
        "每条转发必须有各自的对账键（INV-02）"
    );
    assert!(
        posts
            .iter()
            .all(|post| post["type"] != "MULTIPLY" && post["props"].get("mergeMessage").is_none()),
        "item 模式禁止产出合并载荷"
    );
}

/// 接收者在每个目标频道各累加 N 次未读；N×M 全量核对（INV-12 数量语义）。
#[test]
fn item_mode_receiver_unread_accrues_once_per_forwarded_post_in_each_target() {
    let mut sender =
        Harness::new_with_identity(&[TARGET_A, TARGET_B], SENDER, "mv3-g03a", "", "", "");
    let rows = [
        source_row(
            SOURCE_A,
            "source-user-a",
            "TEXT",
            "第一条源消息",
            CREATE_AT - 20,
        ),
        source_row(
            SOURCE_B,
            "source-user-b",
            "TEXT",
            "第二条源消息",
            CREATE_AT - 10,
        ),
    ];
    let targets = [cid(TARGET_A), cid(TARGET_B)];
    let body = drive_item_forward(&mut sender, &rows, &targets, "item", CREATE_AT as u64);
    let posts = body["posts"].as_array().expect("posts array").clone();

    let mut receiver =
        Harness::new_with_identity(&[TARGET_A, TARGET_B], RECEIVER, "mv3-g03a", "", "", "");
    let mut table = MemberTable::default();
    let mut total_bumps = 0_i64;
    let mut id_seed = 100_u64;
    let mut clock = CREATE_AT as u64 + 100;

    for target in &targets {
        for (index, post) in posts.iter().enumerate() {
            let seq = index as u64 + 1;
            let frame = authority(post, target, id_seed, seq, CREATE_AT + 1 + index as i64);
            id_seed += 1;
            let applied =
                apply_authority(&mut receiver, &mut table, RECEIVER, target, &frame, clock);
            clock += 10;
            assert!(applied.committed, "接收者必须为每条转发独立提交");
            assert_eq!(
                applied.bumps,
                [1],
                "接收者每条转发只允许 +1 未读，且必须是 SQL 自增而非覆盖"
            );
            total_bumps += applied.bumps.iter().sum::<i64>();
        }
    }

    assert_eq!(
        total_bumps, 4,
        "N=2 源 × M=2 目标 → 接收者共计 4 次未读自增"
    );
    assert_eq!(table.unread(&cid(TARGET_A), RECEIVER), 2);
    assert_eq!(
        table.unread(&cid(TARGET_B), RECEIVER),
        2,
        "每个目标频道未读独立累加，互不串味"
    );
    assert_eq!(
        table.unread(&cid(TARGET_A), SENDER),
        0,
        "接收者回放禁止在发送者行上留下任何未读（INV-12 禁跨 persona 复制）"
    );
}

/// 发送者自身转发不产生未读：只允许绝对置零，禁止出现任何自增。
#[test]
fn sender_persona_never_accrues_unread_for_its_own_forward() {
    let mut sender = Harness::new_with_identity(&[TARGET_A], SENDER, "mv3-g03a", "", "", "");
    let rows = [source_row(
        SOURCE_A,
        "source-user-a",
        "TEXT",
        "第一条源消息",
        CREATE_AT - 20,
    )];
    let targets = [cid(TARGET_A)];
    let body = drive_item_forward(&mut sender, &rows, &targets, "item", CREATE_AT as u64);
    let http = http_corr(sender.sink.as_slice()).expect("createPosts correlation");
    sender.step(
        port_reply_with_body(http.raw(), acceptance(&targets)),
        CREATE_AT as u64 + 50,
    );
    let batch = emitted(sender.sink.as_slice())
        .into_iter()
        .find(|event| event["event"] == "im:posts:batch-result")
        .expect("HTTP admission must publish a per-target batch result");
    assert_eq!(batch["data"]["batchStatus"], "success");
    assert_eq!(batch["data"]["targets"][0]["deliveryStatus"], "pending");

    let post = body["posts"][0].clone();
    let frame = authority(&post, &cid(TARGET_A), 200, 1, CREATE_AT + 1);
    let mut table = MemberTable::default();
    let applied = apply_authority(
        &mut sender,
        &mut table,
        SENDER,
        &cid(TARGET_A),
        &frame,
        CREATE_AT as u64 + 60,
    );
    assert!(applied.committed);
    assert!(
        applied.bumps.is_empty(),
        "发送者视角禁止出现 unread 自增，否则自己转发会给自己制造未读"
    );
    assert!(
        applied.absolute.is_empty(),
        "发送者视角不得覆盖既有 unread_count"
    );
    assert_eq!(table.unread(&cid(TARGET_A), SENDER), 0);

    let channel_update = applied
        .events
        .iter()
        .find(|event| event["event"] == "im:channel:update")
        .expect("readback must publish the absolute channel projection");
    assert_eq!(
        channel_update["data"]["unreadCount"], 0,
        "发送者绝对投影未读必须为 0"
    );
    assert_eq!(channel_update["data"]["channelId"], cid(TARGET_A));
}

/// `post_ids` / `target_channel_ids` 的空洞值必须在任何 Effect 之前 fail closed。
#[test]
fn undefined_source_or_target_ids_fail_closed_before_any_effect() {
    let cases = [
        ("null 源 id", json!([sid(SOURCE_A), Value::Null])),
        ("空串源 id", json!([sid(SOURCE_A), ""])),
        ("空数组", json!([])),
        ("重复源 id", json!([sid(SOURCE_A), sid(SOURCE_A)])),
    ];
    for (label, post_ids) in cases {
        let mut harness = Harness::new_with_identity(&[TARGET_A], SENDER, "mv3-g03a", "", "", "");
        let payload = json!({
            "post_ids": post_ids,
            "target_channel_ids": [cid(TARGET_A)],
            "mode": "item",
            "req_id": REQ_ID,
        });
        let bytes = serde_json::to_vec(&payload).expect("payload encodes");
        let outcome =
            helix_im::client_api::build_command("im_relay_messages", &bytes).and_then(|command| {
                harness
                    .module
                    .handle(&Tick::Command(command), CREATE_AT as u64, &mut harness.sink)
                    .map_err(|error| helix_im::ImError::Parse(error.to_string()))
            });
        assert!(outcome.is_err(), "{label} 必须被拒绝");
        assert!(
            harness.sink.as_slice().is_empty(),
            "{label} 被拒绝时不得留下任何 Effect"
        );
    }

    let mut harness = Harness::new_with_identity(&[TARGET_A], SENDER, "mv3-g03a", "", "", "");
    let payload = json!({
        "post_ids": [sid(SOURCE_A)],
        "target_channel_ids": [cid(TARGET_A), Value::Null],
        "mode": "item",
        "req_id": REQ_ID,
    });
    let bytes = serde_json::to_vec(&payload).expect("payload encodes");
    let outcome =
        helix_im::client_api::build_command("im_relay_messages", &bytes).and_then(|command| {
            harness
                .module
                .handle(&Tick::Command(command), CREATE_AT as u64, &mut harness.sink)
                .map_err(|error| helix_im::ImError::Parse(error.to_string()))
        });
    assert!(outcome.is_err(), "null 目标频道必须被拒绝");
    assert!(harness.sink.as_slice().is_empty());
}

/// 任一来源行缺失 → 全部目标标记失败，且绝不出站 createPosts。
#[test]
fn missing_source_row_cancels_every_target_without_http() {
    let mut harness =
        Harness::new_with_identity(&[TARGET_A, TARGET_B], SENDER, "mv3-g03a", "", "", "");
    let rows = [
        source_row(
            SOURCE_A,
            "source-user-a",
            "TEXT",
            "第一条源消息",
            CREATE_AT - 20,
        ),
        source_row(
            SOURCE_B,
            "source-user-b",
            "TEXT",
            "第二条源消息",
            CREATE_AT - 10,
        ),
    ];
    let source_ids: Vec<String> = rows
        .iter()
        .map(|row| row["id"].as_str().expect("source id").to_string())
        .collect();
    let targets = [cid(TARGET_A), cid(TARGET_B)];
    harness.step(
        relay_tick(&relay_payload("item", &source_ids, &targets)),
        CREATE_AT as u64,
    );

    let first = expect_source_get(harness.sink.as_slice(), &source_ids[0]);
    let body = serde_json::to_vec(&json!([rows[0]])).expect("row encodes");
    harness.step(
        port_reply_with_body(first.raw(), Bytes::from(body)),
        CREATE_AT as u64 + 1,
    );

    // 第二条来源不存在（被过滤掉的 undefined 在服务端等价形态）。
    let second = expect_source_get(harness.sink.as_slice(), &source_ids[1]);
    harness.step(
        port_reply_with_body(second.raw(), Bytes::from("[]")),
        CREATE_AT as u64 + 2,
    );

    assert!(
        http_corr(harness.sink.as_slice()).is_none(),
        "来源缺失时禁止出站 createPosts"
    );
    let batch = emitted(harness.sink.as_slice())
        .into_iter()
        .find(|event| event["event"] == "im:posts:batch-result")
        .expect("来源缺失必须发布可观察的批失败");
    assert_eq!(batch["data"]["batchStatus"], "failed");
    assert_eq!(
        batch["data"]["failedCount"], 2,
        "所有 M 个目标都必须被标记为不可投递"
    );
    assert_eq!(
        batch["data"]["targets"].as_array().map(|targets| targets
            .iter()
            .map(|target| target["channelId"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>()),
        Some(vec![cid(TARGET_A), cid(TARGET_B)])
    );
}
